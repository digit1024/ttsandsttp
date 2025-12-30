//! DBus Daemon Service
//!
//! Provides a DBus interface for TTS and STT operations.
//! Uses channel-based architecture to handle services with non-Send types.

use anyhow::{Context, Result, bail};
use zbus::{connection, interface, fdo::Error as FdoError, Message};
use tokio::sync::mpsc;
use std::time::Duration;
use std::sync::{Arc, Mutex};
use wrtype::WrtypeClient;

use crate::config::SharedConfig;
use crate::services::{SttService, TtsService};

// STT timeout constants
const STT_TIMEOUT_MS: u64 = 60000; // 60 seconds total timeout
const STT_CHECK_INTERVAL_MS: u64 = 100; // Check every 100ms
const MAX_STT_TIMEOUT_CHECKS: u64 = STT_TIMEOUT_MS / STT_CHECK_INTERVAL_MS; // 600 checks

/// DBus Service for TTS and STT Operations
///
/// Exposes Text-to-Speech and Speech-to-Text functionality via DBus.
/// 
/// # Architecture
/// - TTS and STT services run in dedicated threads with their own tokio runtimes
/// - Communication via message channels to avoid Send/Sync issues with audio streams
/// - DBus interface: `com.github.digit1024.ttsstt.Service`
/// 
/// # DBus Methods
/// - `tts(text: String, language: String)` - Convert text to speech
/// - `stt(language: String, pause_duration: f64)` - Convert speech to text
/// - `stt_type(language: String, pause_duration: f64)` - Convert speech to text and type it
/// - `stop()` - Stop current playback
/// 
/// # DBus Signals
/// - `StatusChanged(status: String)` - Emitted when status changes (idle/speaking/listening/processing)
pub struct TtsSttService {
    tts_tx: mpsc::UnboundedSender<TtsRequest>,
    stt_tx: mpsc::UnboundedSender<SttRequest>,
    status_tx: Arc<Mutex<Option<mpsc::UnboundedSender<String>>>>,
    client: Arc<Mutex<Option<WrtypeClient>>>,
    // Shared cancellation channel for interrupting speak()
    speak_cancel_tx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<()>>>>,
}

enum TtsRequest {
    Init(tokio::sync::oneshot::Sender<Result<()>>),
    Speak {
        text: String,
        language: String,
        reply: tokio::sync::oneshot::Sender<Result<()>>,
    },
    Stop(tokio::sync::oneshot::Sender<Result<()>>),
}

enum SttRequest {
    Init(tokio::sync::oneshot::Sender<Result<()>>),
    StartListening {
        language: String,
        pause_duration: f64,
        reply: tokio::sync::oneshot::Sender<Result<String>>,
    },
    Stop(tokio::sync::oneshot::Sender<Result<String>>),
    
}

impl TtsSttService {
    /// Helper function to emit status updates
    fn emit_status(status_tx: &Arc<Mutex<Option<mpsc::UnboundedSender<String>>>>, status: &str) {
        if let Ok(guard) = status_tx.lock() {
            if let Some(ref tx) = *guard {
                let _ = tx.send(status.to_string());
            }
        }
    }
    
    /// Create a new DBus service instance
    pub fn new() -> Result<Self> {
        // Load shared config once
        let shared_config = SharedConfig::load()
            .context("Failed to load shared configuration")?;
        
        // Create channel for status updates
        let status_tx = Arc::new(Mutex::new(None::<mpsc::UnboundedSender<String>>));
        let status_tx_clone = Arc::clone(&status_tx);
        
        // Create channels for TTS
        let (tts_tx, mut tts_rx) = mpsc::unbounded_channel();
        let status_tx_for_tts = Arc::clone(&status_tx_clone);
        let shared_config_for_tts = shared_config.clone();
        
        // Create shared cancellation channel for interrupting speak()
        // This allows stop() to interrupt speak() even while it's blocking
        let speak_cancel_tx = Arc::new(Mutex::new(None::<tokio::sync::mpsc::UnboundedSender<()>>));
        let speak_cancel_tx_clone = Arc::clone(&speak_cancel_tx);
        
        let client = WrtypeClient::new()
            .expect("Failed to create wrtype client");
        
        // Spawn TTS handler thread (create service inside thread to avoid Send issues)
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tts = TtsService::new_with_config(&shared_config_for_tts)
                    .expect("Failed to create TTS service");
                while let Some(req) = tts_rx.recv().await {
                    match req {
                        TtsRequest::Init(reply) => {
                            let result = tts.init().await;
                            let _ = reply.send(result);
                        }
                        TtsRequest::Speak { text, language, reply } => {
                            // CRITICAL: If there's already a speak() in progress, cancel it first
                            // This ensures new TTS requests immediately stop the previous one
                            let had_previous_speak = {
                                let mut guard = speak_cancel_tx_clone.lock().unwrap();
                                if let Some(prev_cancel_tx) = guard.take() {
                                    tracing::debug!("New TTS request - cancelling previous speak() immediately");
                                    let _ = prev_cancel_tx.send(()); // Cancel previous speak()
                                    true
                                } else {
                                    false
                                }
                            };
                            
                            // If we cancelled a previous speak(), also stop TTS service and wait a bit
                            if had_previous_speak {
                                tracing::debug!("Stopping TTS service to ensure previous playback stops");
                                let _ = tts.stop();
                                // Small delay to ensure previous speak() cleanup completes
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            }
                            
                            // Emit "speaking" status
                            Self::emit_status(&status_tx_for_tts, "speaking");
                            
                            // Create cancellation channel for this speak operation
                            let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
                            
                            // Store cancel sender so stop() can interrupt this speak()
                            {
                                let mut guard = speak_cancel_tx_clone.lock().unwrap();
                                *guard = Some(cancel_tx);
                            }
                            
                            let speak_future = async {
                                tts.set_language(&language).await?;
                                tts.speak(&text).await?;
                                Ok::<(), anyhow::Error>(())
                            };
                            
                            // Use select to allow stop() to interrupt speak()
                            let result = tokio::select! {
                                res = speak_future => {
                                    // Clear cancel sender
                                    {
                                        let mut guard = speak_cancel_tx_clone.lock().unwrap();
                                        *guard = None;
                                    }
                                    res
                                }
                                _ = cancel_rx.recv() => {
                                    // Stop was called - interrupt speak immediately
                                    tracing::debug!("Speak interrupted by stop request - stopping TTS immediately");
                                    // Stop the TTS immediately
                                    let _ = tts.stop();
                                    // Clear cancel sender
                                    {
                                        let mut guard = speak_cancel_tx_clone.lock().unwrap();
                                        *guard = None;
                                    }
                                    Err(anyhow::anyhow!("TTS stopped"))
                                }
                            };
                            
                            // Emit "idle" status after speaking completes (or is stopped)
                            Self::emit_status(&status_tx_for_tts, "idle");
                            
                            let _ = reply.send(result);
                        }
                        TtsRequest::Stop(reply) => {
                            // Stop immediately - interrupt any ongoing speak()
                            tracing::debug!("Stop request received - interrupting speak() if running");
                            
                            // First, try to interrupt speak() via cancellation channel
                            {
                                let mut guard = speak_cancel_tx_clone.lock().unwrap();
                                if let Some(cancel_tx) = guard.take() {
                                    let _ = cancel_tx.send(());
                                    tracing::debug!("Sent cancellation signal to interrupt speak()");
                                }
                            }
                            
                            // Also stop the TTS service directly
                            let result = tts.stop();
                            
                            // Don't emit "idle" here - let the next operation emit its state
                            // This prevents "idle" from being emitted between "processing" and "speaking"
                            
                            let _ = reply.send(result);
                        }
                    }
                }
            });
        });

        // Create channels for STT
        let (stt_tx, mut stt_rx) = mpsc::unbounded_channel();
        let status_tx_for_stt = Arc::clone(&status_tx_clone);
        let shared_config_for_stt = shared_config.clone();
        
        // Spawn STT handler thread (create service inside thread to avoid Send issues)
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let stt = SttService::new_with_config(&shared_config_for_stt)
                    .expect("Failed to create STT service");
                while let Some(req) = stt_rx.recv().await {
                    match req {
                        SttRequest::Init(reply) => {
                            // Emit "processing" status during initialization
                            Self::emit_status(&status_tx_for_stt, "processing");
                            let result = stt.init().await;
                            // Emit "idle" status after initialization
                            Self::emit_status(&status_tx_for_stt, "idle");
                            let _ = reply.send(result);
                        }
                        SttRequest::StartListening { language, pause_duration, reply } => {
                            // Set up callbacks
                            stt.on_result(|_text| {});
                            // Emit "processing" status when pause is detected (same time as beep)
                            let status_tx_for_pause = Arc::clone(&status_tx_for_stt);
                            stt.on_pause_detected(move || {
                                Self::emit_status(&status_tx_for_pause, "processing");
                            });
                            stt.on_error(|err| {
                                tracing::error!("STT Error: {}", err);
                            });
                            
                            // Emit "listening" status
                            Self::emit_status(&status_tx_for_stt, "listening");
                            
                            let result = async {
                                stt.start_listening(&language, std::time::Duration::from_secs_f64(pause_duration)).await?;
                                
                                // Wait for listening to complete
                                let mut check_count = 0;
                                loop {
                                    tokio::time::sleep(Duration::from_millis(STT_CHECK_INTERVAL_MS)).await;
                                    
                                    if !stt.is_listening() {
                                        // Get the decoded result (decode should have completed by now)
                                        let text = stt.stop_listening()?;
                                        
                                        // Emit "idle" status after processing completes
                                        Self::emit_status(&status_tx_for_stt, "idle");
                                        return Ok(text);
                                    }

                                    check_count += 1;
                                    if check_count > MAX_STT_TIMEOUT_CHECKS {
                                        let _ = stt.stop_listening();
                                        // Emit "idle" status on timeout
                                        Self::emit_status(&status_tx_for_stt, "idle");
                                        bail!("STT operation timed out after {}ms", STT_TIMEOUT_MS);
                                    }
                                }
                            }.await;
                            let _ = reply.send(result);
                        }
                        SttRequest::Stop(reply) => {
                            // Emit "processing" status (same as pause detected - decoding/processing)
                            Self::emit_status(&status_tx_for_stt, "processing");
                            
                            // Stop listening and decode, return the recognized text
                            let result = if stt.is_listening() {
                                stt.stop_listening() // Returns Result<String>
                            } else {
                                // Not listening, return current text or empty string
                                Ok(stt.current_text().clone())
                            };
                            
                            // Don't emit "idle" here - let the next operation emit its state
                            // This prevents "idle" from being emitted between "processing" and "speaking"
                            
                            let _ = reply.send(result);
                        }
                        
                    }
                }
            });
        });

        Ok(Self { 
            tts_tx, 
            stt_tx,
            status_tx: status_tx_clone,
            client: Arc::new(Mutex::new(Some(client))),
            speak_cancel_tx: speak_cancel_tx,
        })
    }

    /// Initialize and preload both TTS and STT models
    pub async fn preload_models(&self) -> Result<()> {
        tracing::info!("Preloading TTS models...");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tts_tx.send(TtsRequest::Init(tx)).context("TTS channel closed")?;
        rx.await.context("TTS init reply channel error")??;

        tracing::info!("Preloading STT models...");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.stt_tx.send(SttRequest::Init(tx)).context("STT channel closed")?;
        rx.await.context("STT init reply channel error")??;

        tracing::info!("All models preloaded successfully");
        Ok(())
    }

    /// Start the DBus service and listen for requests
    pub async fn serve(&self) -> Result<()> {
        let service = self.clone();
        
        // Create channel for status updates from worker threads
        let (status_tx_internal, mut status_rx) = mpsc::unbounded_channel::<String>();
        {
            let mut guard = self.status_tx.lock().unwrap();
            *guard = Some(status_tx_internal);
        }
        
        let connection = connection::Builder::session()?
            .name("com.github.digit1024.ttsstt")?
            .serve_at("/com/github/digit1024/ttsstt", service.clone())?
            .build()
            .await?;

        // Spawn task to handle status updates and emit DBus signals
        tokio::spawn(async move {
            while let Some(status) = status_rx.recv().await {
                // Manually emit DBus signal
                let message_result = Message::signal(
                    "/com/github/digit1024/ttsstt",
                    "com.github.digit1024.ttsstt.Service",
                    "StatusChanged",
                )
                .and_then(|builder| builder.build(&(status,)));
                
                match message_result {
                    Ok(message) => {
                        if let Err(e) = connection.send(&message).await {
                            tracing::warn!("Failed to send StatusChanged signal: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to build StatusChanged signal: {}", e);
                    }
                }
            }
        });

        tracing::info!("DBus service started");
        tracing::info!("   Service: com.github.digit1024.ttsstt");
        tracing::info!("   Object: /com/github/digit1024/ttsstt");
        tracing::info!("   Interface: com.github.digit1024.ttsstt.Service");
        tracing::info!("   Waiting for requests...");

        // Keep the connection alive
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    }
}

impl Clone for TtsSttService {
    fn clone(&self) -> Self {
        Self {
            tts_tx: self.tts_tx.clone(),
            stt_tx: self.stt_tx.clone(),
            status_tx: Arc::clone(&self.status_tx),
            client: Arc::clone(&self.client),
            speak_cancel_tx: Arc::clone(&self.speak_cancel_tx),
        }
    }
}

impl TtsSttService {
    /// Helper: Cancel/stop TTS operation with proper error handling and logging
    async fn cancel_tts(&self) {
        tracing::debug!("Daemon: Cancelling TTS operation");
        
        // First, try to interrupt speak() immediately via cancellation channel
        // This works even if speak() is currently blocking
        {
            let mut guard = self.speak_cancel_tx.lock().unwrap();
            if let Some(cancel_tx) = guard.take() {
                let _ = cancel_tx.send(());
                tracing::debug!("Sent immediate cancellation signal to interrupt speak()");
            }
        }
        
        // Also send stop request to TTS thread
        let (tx, rx) = tokio::sync::oneshot::channel();
        if let Err(e) = self.tts_tx.send(TtsRequest::Stop(tx)) {
            tracing::warn!("Failed to send TTS stop request: {}", e);
            return;
        }

        // Don't wait for reply - we've already interrupted speak() via cancellation channel
        // The stop request will be processed when speak() is interrupted
        let _ = tokio::time::timeout(std::time::Duration::from_millis(50), rx).await;
        tracing::debug!("TTS stop signal sent (speak() should be interrupted)");
    }

    /// Helper: Cancel/stop STT operation with proper error handling and logging
    async fn cancel_stt(&self) {
        tracing::debug!("Daemon: Cancelling STT operation");
        let (tx, rx) = tokio::sync::oneshot::channel();
        if let Err(e) = self.stt_tx.send(SttRequest::Stop(tx)) {
            tracing::warn!("Failed to send STT stop request: {}", e);
            return;
        }

        match tokio::time::timeout(std::time::Duration::from_secs(2), rx).await {
            Ok(Ok(result)) => {
                match result {
                    Ok(text) => {
                        if !text.is_empty() {
                            tracing::debug!("STT stopped with text: '{}'", text);
                        } else {
                            tracing::debug!("STT stopped (no text)");
                        }
                    }
                    Err(e) => {
                        tracing::warn!("STT stop failed: {}", e);
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("STT stop channel error: {}", e);
            }
            Err(_) => {
                tracing::warn!("STT stop timeout (2s)");
            }
        }
    }
}

#[interface(interface = "com.github.digit1024.ttsstt.Service")]
impl TtsSttService {
    /// StatusChanged signal
    /// 
    /// Emitted when the service status changes. Possible values:
    /// - "idle" - No operation in progress
    /// - "speaking" - TTS is currently speaking
    /// - "listening" - STT is currently listening for audio
    /// - "processing" - STT is processing/decoding audio
    /// 
    /// Note: This signal is emitted manually from worker threads via Message::signal()
    /// to handle cross-thread signal emission. This declaration ensures it appears in introspection.
    #[zbus(signal)]
    async fn status_changed(emitter: &zbus::object_server::SignalEmitter<'_>, status: &str) -> zbus::Result<()>;

    /// Text-to-Speech: Convert text to speech
    ///
    /// Cancels any ongoing STT operation and stops any previous TTS operation
    /// before starting the new TTS request.
    ///
    /// Returns immediately after starting TTS (does not wait for completion).
    /// Use the StatusChanged signal to track when speaking completes.
    async fn tts(&self, text: String, language: String) -> Result<(), FdoError> {
        tracing::info!("Daemon: TTS request for language: {}, text: '{}'", language, text);

        // Cancel any ongoing STT operation and stop any previous TTS
        tracing::debug!("Cancelling any ongoing operations...");
        self.cancel_stt().await;
        self.cancel_tts().await;

        // Now start the new TTS request (return immediately, don't wait for completion)
        tracing::debug!("Starting TTS generation...");
        let (tx, _rx) = tokio::sync::oneshot::channel();
        self.tts_tx
            .send(TtsRequest::Speak { text, language, reply: tx })
            .map_err(|e| FdoError::Failed(format!("TTS channel closed: {}", e)))?;

        tracing::info!("TTS started successfully (running in background)");
        // Return immediately - TTS will continue in background
        Ok(())
    }

    /// Speech-to-Text: Convert speech to text
    ///
    /// Stops any ongoing TTS operation and cancels any previous STT operation
    /// before starting the new STT request.
    ///
    /// Waits for STT to complete and returns the recognized text.
    /// Use StatusChanged signal to track status changes (listening -> processing -> idle).
    async fn stt(&self, language: String, pause_duration: f64) -> Result<String, FdoError> {
        tracing::info!("Daemon: STT request for language: {}, pause: {}s", language, pause_duration);

        // Stop any ongoing TTS operation and cancel any previous STT
        tracing::debug!("Cancelling any ongoing operations...");
        self.cancel_tts().await;
        self.cancel_stt().await;

        // Start listening and wait for completion
        tracing::debug!("Starting STT listening...");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.stt_tx
            .send(SttRequest::StartListening { language, pause_duration, reply: tx })
            .map_err(|e| FdoError::Failed(format!("STT channel closed: {}", e)))?;

        // Wait for STT to complete and get the text
        tracing::debug!("Waiting for STT completion...");
        let text = rx.await
            .map_err(|e| FdoError::Failed(format!("STT reply channel error: {}", e)))?
            .map_err(|e| FdoError::Failed(format!("STT failed: {}", e)))?;

        tracing::info!("STT completed with text: '{}'", text);
        Ok(text)
    }

    /// Stop all operations with immediate cancellation
    ///
    /// Stops any ongoing TTS playback and any ongoing STT listening operation.
    /// Uses immediate cancellation tokens for both services.
    ///
    /// Returns the recognized text if STT was active, otherwise returns empty string.
    async fn stop(&self) -> Result<String, FdoError> {
        tracing::info!("Daemon: Stop all operations requested");

        // Stop TTS with immediate cancellation
        tracing::debug!("Stopping TTS...");
        self.cancel_tts().await;

        // Stop STT with immediate cancellation and get recognized text
        tracing::debug!("Stopping STT...");
        let (tx_stt, rx_stt) = tokio::sync::oneshot::channel();
        self.stt_tx
            .send(SttRequest::Stop(tx_stt))
            .map_err(|e| FdoError::Failed(format!("STT channel closed: {}", e)))?;

        // Wait for stop to complete with timeout
        let text = match tokio::time::timeout(std::time::Duration::from_secs(2), rx_stt).await {
            Ok(Ok(result)) => {
                match result {
                    Ok(text) => {
                        if !text.is_empty() {
                            tracing::info!("STT stopped with text: '{}'", text);
                        } else {
                            tracing::info!("STT stopped (no text recognized)");
                        }
                        text
                    }
                    Err(e) => {
                        tracing::warn!("STT stop failed: {}", e);
                        String::new()
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("STT stop channel error: {}", e);
                String::new()
            }
            Err(_) => {
                tracing::warn!("STT stop timeout (2s)");
                String::new()
            }
        };

        // Emit "idle" status after stopping all operations
        Self::emit_status(&self.status_tx, "idle");

        tracing::info!("All operations stopped successfully");
        Ok(text)
    }


    /// Speech-to-Text Type: Convert speech to text and type it character-by-character using wrtype
    ///
    /// Starts STT and waits for it to complete, then types the recognized text.
    /// Note: This method waits for STT completion internally (unlike stt() which returns immediately).
    /// It uses the original StartListening mechanism that waits for completion.
    async fn stt_type(&self, language: String, pause_duration: f64) -> Result<(), FdoError> {
        tracing::info!("Daemon: STT Type request for language: {}, pause: {}s", language, pause_duration);

        // Use StartListening directly and wait for completion (for stt_type we need the text)
        // Stop any ongoing operations first
        tracing::debug!("Cancelling any ongoing operations...");
        self.cancel_tts().await;
        self.cancel_stt().await;

        // Start listening and wait for completion
        tracing::debug!("Starting STT listening for typing...");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.stt_tx
            .send(SttRequest::StartListening { language, pause_duration, reply: tx })
            .map_err(|e| FdoError::Failed(format!("STT channel closed: {}", e)))?;

        // Wait for STT to complete and get the text
        tracing::debug!("Waiting for STT completion...");
        let text = rx.await
            .map_err(|e| FdoError::Failed(format!("STT reply channel error: {}", e)))?
            .map_err(|e| FdoError::Failed(format!("STT failed: {}", e)))?;

        tracing::info!("STT completed with text: '{}', now typing...", text);

        // Type the text
        self.type_text(text).await
    }
    
    /// Helper: Type text using wrtype (internal)
    async fn type_text(&self, text: String) -> Result<(), FdoError> {
        if text.is_empty() {
            tracing::debug!("No text to type");
            return Ok(()); // Nothing to type
        }

        tracing::debug!("Typing text: '{}'", text);

        // Clone the client Arc before moving into the closure
        let client_arc = Arc::clone(&self.client);

        // Type the text character-by-character using wrtype
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let mut client_guard = client_arc.lock().unwrap();
            let client = client_guard.as_mut().ok_or("Client not initialized")?;
            client.type_text_with_delay(&text, Duration::from_millis(10))
                .map_err(|e| format!("Failed to type text: {}", e))?;

            Ok(())
        })
        .await
        .map_err(|e| FdoError::Failed(format!("Task join error: {}", e)))?
        .map_err(|e| FdoError::Failed(e))?;

        tracing::debug!("Text typed successfully");
        Ok(())
    }
}
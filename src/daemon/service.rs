//! DBus Daemon Service
//!
//! Provides a DBus interface for TTS and STT operations.
//! Uses channel-based architecture to handle services with non-Send types.

use anyhow::Result;
use zbus::{connection, interface, fdo::Error as FdoError, Message};
use tokio::sync::mpsc;
use std::time::Duration;
use std::sync::{Arc, Mutex};

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
    Stop(tokio::sync::oneshot::Sender<Result<()>>),
    
}

impl TtsSttService {
    /// Create a new DBus service instance
    pub fn new() -> Result<Self> {
        // Create channel for status updates
        let status_tx = Arc::new(Mutex::new(None::<mpsc::UnboundedSender<String>>));
        let status_tx_clone = Arc::clone(&status_tx);
        
        // Create channels for TTS
        let (tts_tx, mut tts_rx) = mpsc::unbounded_channel();
        let status_tx_for_tts = Arc::clone(&status_tx_clone);
        
        // Spawn TTS handler thread (create service inside thread to avoid Send issues)
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tts = TtsService::new().expect("Failed to create TTS service");
                while let Some(req) = tts_rx.recv().await {
                    match req {
                        TtsRequest::Init(reply) => {
                            let result = tts.init().await;
                            let _ = reply.send(result);
                        }
                        TtsRequest::Speak { text, language, reply } => {
                            // Emit "speaking" status
                            if let Ok(guard) = status_tx_for_tts.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send("speaking".to_string());
                                }
                            }
                            
                            let result = async {
                                tts.set_language(&language).await?;
                                tts.speak(&text).await?;
                                Ok::<(), anyhow::Error>(())
                            }.await;
                            
                            // Emit "idle" status after speaking completes
                            if let Ok(guard) = status_tx_for_tts.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send("idle".to_string());
                                }
                            }
                            
                            let _ = reply.send(result);
                        }
                        TtsRequest::Stop(reply) => {
                            let result = tts.stop();
                            
                            // Emit "idle" status after stopping
                            if let Ok(guard) = status_tx_for_tts.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send("idle".to_string());
                                }
                            }
                            
                            let _ = reply.send(result);
                        }
                    }
                }
            });
        });

        // Create channels for STT
        let (stt_tx, mut stt_rx) = mpsc::unbounded_channel();
        let status_tx_for_stt = Arc::clone(&status_tx_clone);
        
        // Spawn STT handler thread (create service inside thread to avoid Send issues)
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let stt = SttService::new().expect("Failed to create STT service");
                while let Some(req) = stt_rx.recv().await {
                    match req {
                        SttRequest::Init(reply) => {
                            // Emit "processing" status during initialization
                            if let Ok(guard) = status_tx_for_stt.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send("processing".to_string());
                                }
                            }
                            let result = stt.init().await;
                            // Emit "idle" status after initialization
                            if let Ok(guard) = status_tx_for_stt.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send("idle".to_string());
                                }
                            }
                            let _ = reply.send(result);
                        }
                        SttRequest::StartListening { language, pause_duration, reply } => {
                            // Set up callbacks
                            stt.on_result(|_text| {});
                            // Emit "processing" status when pause is detected (same time as beep)
                            let status_tx_for_pause = Arc::clone(&status_tx_for_stt);
                            stt.on_pause_detected(move || {
                                if let Ok(guard) = status_tx_for_pause.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send("processing".to_string());
                                    }
                                }
                            });
                            stt.on_error(|err| {
                                eprintln!("❌ STT Error: {}", err);
                            });
                            
                            // Emit "listening" status
                            if let Ok(guard) = status_tx_for_stt.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send("listening".to_string());
                                }
                            }
                            
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
                                        if let Ok(guard) = status_tx_for_stt.lock() {
                                            if let Some(ref tx) = *guard {
                                                let _ = tx.send("idle".to_string());
                                            }
                                        }
                                        return Ok(text);
                                    }

                                    check_count += 1;
                                    if check_count > MAX_STT_TIMEOUT_CHECKS {
                                        let _ = stt.stop_listening();
                                        // Emit "idle" status on timeout
                                        if let Ok(guard) = status_tx_for_stt.lock() {
                                            if let Some(ref tx) = *guard {
                                                let _ = tx.send("idle".to_string());
                                            }
                                        }
                                        anyhow::bail!("STT operation timed out after {}ms", STT_TIMEOUT_MS);
                                    }
                                }
                            }.await;
                            let _ = reply.send(result);
                        }
                        SttRequest::Stop(reply) => {
                            // Emit "processing" status (same as pause detected - decoding/processing)
                            if let Ok(guard) = status_tx_for_stt.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send("processing".to_string());
                                }
                            }
                            
                            // Stop listening and decode
                            let result = if stt.is_listening() {
                                let _ = stt.stop_listening(); // Get decoded result
                                Ok(())
                            } else {
                                Ok(())
                            };
                            
                            // Emit "idle" status after stopping
                            if let Ok(guard) = status_tx_for_stt.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send("idle".to_string());
                                }
                            }
                            
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
        })
    }

    /// Initialize and preload both TTS and STT models
    pub async fn preload_models(&self) -> Result<()> {
        println!("📦 Preloading TTS models...");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tts_tx.send(TtsRequest::Init(tx)).map_err(|e| anyhow::anyhow!("TTS channel closed: {}", e))?;
        rx.await.map_err(|e| anyhow::anyhow!("TTS init reply channel error: {}", e))??;

        println!("📦 Preloading STT models...");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.stt_tx.send(SttRequest::Init(tx)).map_err(|e| anyhow::anyhow!("STT channel closed: {}", e))?;
        rx.await.map_err(|e| anyhow::anyhow!("STT init reply channel error: {}", e))??;

        println!("✅ All models preloaded successfully");
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
                            eprintln!("⚠️  Failed to send StatusChanged signal: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("⚠️  Failed to build StatusChanged signal: {}", e);
                    }
                }
            }
        });

        println!("✅ DBus service started");
        println!("   Service: com.github.digit1024.ttsstt");
        println!("   Object: /com/github/digit1024/ttsstt");
        println!("   Interface: com.github.digit1024.ttsstt.Service");
        println!("   Waiting for requests...");

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
        }
    }
}

impl TtsSttService {
    /// Helper: Cancel/stop TTS operation (ignores errors)
    async fn cancel_tts(&self) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.tts_tx.send(TtsRequest::Stop(tx));
        let _ = rx.await; // Ignore errors - just stop if possible
    }

    /// Helper: Cancel/stop STT operation (ignores errors)
    async fn cancel_stt(&self) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.stt_tx.send(SttRequest::Stop(tx));
        let _ = rx.await; // Ignore errors - just cancel if possible
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
    async fn tts(&self, text: String, language: String) -> Result<(), FdoError> {
        // Cancel any ongoing STT operation and stop any previous TTS
        self.cancel_stt().await;
        self.cancel_tts().await;
        
        // Now start the new TTS request
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tts_tx
            .send(TtsRequest::Speak { text, language, reply: tx })
            .map_err(|e| FdoError::Failed(format!("TTS channel closed: {}", e)))?;
        
        rx.await
            .map_err(|e| FdoError::Failed(format!("TTS reply channel error: {}", e)))?
            .map_err(|e| FdoError::Failed(format!("TTS failed: {}", e)))?;

        Ok(())
    }

    /// Speech-to-Text: Convert speech to text
    /// 
    /// Stops any ongoing TTS operation and cancels any previous STT operation
    /// before starting the new STT request.
    async fn stt(&self, language: String, pause_duration: f64) -> Result<String, FdoError> {
        // Stop any ongoing TTS operation and cancel any previous STT
        self.cancel_tts().await;
        self.cancel_stt().await;

        // Start listening
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.stt_tx
            .send(SttRequest::StartListening { language, pause_duration, reply: tx })
            .map_err(|e| FdoError::Failed(format!("STT channel closed: {}", e)))?;
        
        rx.await
            .map_err(|e| FdoError::Failed(format!("STT reply channel error: {}", e)))?
            .map_err(|e| FdoError::Failed(format!("STT failed: {}", e)))
    }

    /// Stop all operations
    /// 
    /// Stops any ongoing TTS playback and any ongoing STT listening operation.
    /// If STT is listening, it will be stopped and moved to processing/decoding state
    /// (same as when pause is detected), then to idle after completion.
    async fn stop(&self) -> Result<(), FdoError> {
        // Stop TTS (ignore errors)
        self.cancel_tts().await;
        
        // Stop STT (will move to processing/decoding, then idle)
        // Note: We propagate STT stop errors to caller
        let (tx_stt, rx_stt) = tokio::sync::oneshot::channel();
        self.stt_tx
            .send(SttRequest::Stop(tx_stt))
            .map_err(|e| FdoError::Failed(format!("STT channel closed: {}", e)))?;
        
        rx_stt.await
            .map_err(|e| FdoError::Failed(format!("STT reply channel error: {}", e)))?
            .map_err(|e| FdoError::Failed(format!("Stop STT failed: {}", e)))?;

        Ok(())
    }


    /// Speech-to-Text Type: Convert speech to text and type it using enigo (X11) or wtype (Wayland)
    async fn stt_type(&self, language: String, pause_duration: f64) -> Result<(), FdoError> {
        // First, get the text using STT
        let text = self.stt(language, pause_duration).await?;

        if text.is_empty() {
            return Ok(()); // Nothing to type
        }

        // Detect display server and use appropriate typing method
        let wayland_display = std::env::var("WAYLAND_DISPLAY").is_ok();
        let x11_display = std::env::var("DISPLAY").is_ok() && !wayland_display;

        // Type the text using appropriate method
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            if wayland_display {
                // Use wrtype Rust library for Wayland (handles spaces properly)
                use wrtype::WrtypeClient;
                
                let mut client = WrtypeClient::new()
                    .map_err(|e| format!("Failed to create wrtype client: {}", e))?;
                
                client.type_text(&text)
                    .map_err(|e| format!("Failed to type text with wrtype: {}", e))?;
                
                Ok(())
            } else if x11_display {
                // Use enigo for X11
                use enigo::Keyboard;
                let settings = enigo::Settings::default();
                let mut enigo = enigo::Enigo::new(&settings)
                    .map_err(|e| format!("Failed to create enigo: {}", e))?;
                for ch in text.chars() {
                    enigo.key(enigo::Key::Unicode(ch), enigo::Direction::Press)
                        .map_err(|e| format!("Failed to type character: {}", e))?;
                }
                Ok(())
            } else {
                Err("Neither WAYLAND_DISPLAY nor DISPLAY environment variables are set. Cannot determine display server.".to_string())
            }
        })
        .await
        .map_err(|e| FdoError::Failed(format!("Task join error: {}", e)))?
        .map_err(|e| FdoError::Failed(e))?;

        Ok(())
    }
}
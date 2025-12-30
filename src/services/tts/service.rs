//! Text-to-Speech (TTS) Service
//!
//! Provides offline text-to-speech synthesis using VITS models from sherpa-rs.
//! Handles model management, audio generation, and playback.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use rayon::prelude::*;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::config::SharedConfig;
use crate::services::ModelManager;
use crate::utils::{DirectSampleSource, normalize_language_code, split_into_sentences};

/// Text-to-Speech Service
///
/// Provides offline text-to-speech synthesis using VITS models.
/// 
/// # Features
/// - Language support (configurable)
/// - Non-blocking audio playback
/// 
/// # Example
/// ```no_run
/// use ttsandsttp::TtsService;
/// 
/// # async fn example() -> anyhow::Result<()> {
/// let tts = TtsService::new()?;
/// tts.init().await?;
/// tts.set_language("en-US")?;
/// tts.speak("Hello, world!").await?;
/// # Ok(())
/// # }
/// ```
pub struct TtsService {
    engine: Arc<Mutex<Option<sherpa_rs::tts::VitsTts>>>,
    state: Arc<RwLock<TtsState>>, // Use RwLock for read-heavy access
    model_manager: ModelManager,
    model_path: Arc<Mutex<Option<PathBuf>>>,
    _stream: Arc<Mutex<Option<rodio::OutputStream>>>, // Keep stream alive
    sink: Arc<Mutex<Option<Arc<rodio::Sink>>>>, // For audio playback
    shared_config: Arc<SharedConfig>, // Shared config and registry
    cancellation_token: Arc<Mutex<Option<CancellationToken>>>, // For immediate cancellation
    generation_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>, // Background generation task
}

#[derive(Clone, Debug)]
struct TtsState {
    initialized: bool,
    playing: bool,
    current_language: String,
}

impl Default for TtsState {
    fn default() -> Self {
        Self {
            initialized: false,
            playing: false,
            current_language: "en-US".to_string(),
        }
    }
}

impl TtsService {
    // Helper methods for state access
    
    /// Read state (for read-only operations)
    fn read_state(&self) -> std::sync::RwLockReadGuard<'_, TtsState> {
        self.state.read().unwrap()
    }
    
    /// Write state (for write operations)
    fn write_state(&self) -> std::sync::RwLockWriteGuard<'_, TtsState> {
        self.state.write().unwrap()
    }
    
    /// Create a new TTS service instance
    pub fn new() -> Result<Self> {
        Self::new_with_config(&SharedConfig::load()?)
    }
    
    /// Create a new TTS service instance with shared configuration
    pub fn new_with_config(shared_config: &SharedConfig) -> Result<Self> {
        let model_manager = ModelManager::new()?;

        Ok(Self {
            engine: Arc::new(Mutex::new(None)),
            state: Arc::new(RwLock::new(TtsState::default())),
            model_manager,
            model_path: Arc::new(Mutex::new(None)),
            _stream: Arc::new(Mutex::new(None)),
            sink: Arc::new(Mutex::new(None)),
            shared_config: Arc::new(shared_config.clone()),
            cancellation_token: Arc::new(Mutex::new(None)),
            generation_task: Arc::new(Mutex::new(None)),
        })
    }
    
    
    /// Get model files for a language code
    fn get_model_files_for_language(&self, lang_code: &str) -> Result<(PathBuf, PathBuf, PathBuf)> {
        let registry = self.shared_config.registry();
        
        // Get model_id from config for this language
        let config = self.shared_config.config();
        let model_id =             config.tts.languages
                .get(lang_code)
                .map(|lang_cfg| lang_cfg.model_id.clone())
                .with_context(|| format!("Language '{}' not configured", lang_code))?;
        
        let model_info = registry.get_tts_model(lang_code, &model_id)
            .with_context(|| format!("No TTS model '{}' found for language: {}", model_id, lang_code))?;
        
        // Find the .onnx file in required_files
        let onnx_file = model_info.required_files.iter()
            .find(|f| f.ends_with(".onnx"))
            .with_context(|| format!("No .onnx file found for language: {}", lang_code))?;
        
        // Model files are stored in ~/.local/share/stttts/tts/{lang_code}/
        let actual_path = self.model_manager.get_tts_model_path(lang_code, &model_info.id);
        
        let model_file = actual_path.join(onnx_file);
        let tokens_file = actual_path.join("tokens.txt");
        
        // For most models, espeak-ng-data is in the same directory
        // For archived models, it might be in a subdirectory
        let data_dir = actual_path.join("espeak-ng-data");
        
        Ok((model_file, tokens_file, data_dir))
    }

    /// Initialize the TTS engine for a specific language
    pub async fn init_with_language(&self, lang_code: &str) -> Result<()> {
        let normalized_lang = normalize_language_code(lang_code);
        
        // Check if already initialized for this language
        {
            let state = self.read_state();
            if state.initialized && state.current_language == normalized_lang {
                return Ok(());
            }
        }

        // Get model files for this language
        let (model_file, tokens_file, data_dir) = self.get_model_files_for_language(&normalized_lang)?;

        // Check if files exist
        if !model_file.exists() {
            bail!("Model file not found: {:?} (language: {})", model_file, normalized_lang);
        }
        if !tokens_file.exists() {
            bail!("Tokens file not found: {:?} (language: {})", tokens_file, normalized_lang);
        }
        // Note: espeak-ng-data might not exist for all models (e.g., Polish from Hugging Face)
        // We'll make it optional for now
        let data_dir_str = if data_dir.exists() {
            data_dir.to_string_lossy().to_string()
        } else {
            // Use empty string if not found (some models don't need it)
            String::new()
        };

        // Store model path
        let model_path = model_file.parent()
            .context("Model file has no parent directory")?;
        let mut path_guard = self.model_path.lock().unwrap();
        *path_guard = Some(model_path.to_path_buf());
        drop(path_guard);

        // Create VitsTtsConfig
        let config = sherpa_rs::tts::VitsTtsConfig {
            model: model_file.to_string_lossy().to_string(),
            tokens: tokens_file.to_string_lossy().to_string(),
            data_dir: data_dir_str,
            dict_dir: String::new(), // Optional, can be empty
            lexicon: String::new(),  // Optional, can be empty
            onnx_config: Default::default(),
            tts_config: Default::default(), // CommonTtsConfig with defaults
            length_scale: 1.0,
            noise_scale: 0.667,
            noise_scale_w: 0.8,
            silence_scale: 0.0,
        };

        // Initialize VitsTts
        let engine = sherpa_rs::tts::VitsTts::new(config);

        // Replace old engine
        let mut engine_guard = self.engine.lock().unwrap();
        *engine_guard = Some(engine);

        // Update state
        let mut state = self.write_state();
        state.initialized = true;
        state.current_language = normalized_lang;
        
        Ok(())
    }
    
    /// Initialize the TTS engine with default language from config
    pub async fn init(&self) -> Result<()> {
        let default_lang = self.shared_config.config().tts.default.clone();
        self.init_with_language(&default_lang).await
    }

    /// Ensure audio stream is ready (pre-create if needed)
    fn ensure_audio_stream(&self) -> Result<()> {
        use rodio::{OutputStreamBuilder, Sink};
        
        let mut stream_guard = self._stream.lock().unwrap();
        if stream_guard.is_none() {
            let stream = OutputStreamBuilder::open_default_stream()
                .context("Failed to create audio output stream")?;
            let mixer = stream.mixer();
            let sink = Arc::new(Sink::connect_new(&mixer));
            
            *stream_guard = Some(stream);
            let mut sink_guard = self.sink.lock().unwrap();
            *sink_guard = Some(sink);
        }
        Ok(())
    }

    /// Speak the given text (async) with chunking/streaming optimization
    pub async fn speak(&self, text: &str) -> Result<()> {
        {
            let state = self.read_state();
            if !state.initialized {
                drop(state);
                self.init().await?;
            }
        }
        
        // Ensure engine is initialized for current language
        {
            let state = self.read_state();
            let current_lang = state.current_language.clone();
            drop(state);
            self.init_with_language(&current_lang).await?;
        }

        // Pre-create audio stream before generation (optimization #2)
        self.ensure_audio_stream()?;

        {
            let mut state = self.write_state();
            state.playing = true;
        }

        // Split text into sentences for chunking (optimization #1)
        let sentences = split_into_sentences(text);
        
        if sentences.is_empty() {
            let mut state = self.write_state();
            state.playing = false;
            return Ok(());
        }

        // For single sentence or very short text, use simple path
        if sentences.len() == 1 && text.len() < 200 {
            return self.speak_simple(&sentences[0]).await;
        }

        // Chunked/streaming path for longer text
        let result = self.speak_chunked(&sentences).await;

        // Always reset playing state
        {
            let mut state = self.write_state();
            state.playing = false;
        }

        // Clear cancellation token on completion
        {
            let mut token_guard = self.cancellation_token.lock().unwrap();
            token_guard.take();
        }

        result
    }

    /// Simple speak path for short text (no chunking overhead) with cancellation support
    async fn speak_simple(&self, text: &str) -> Result<()> {
        tracing::debug!("Starting simple TTS generation for text: '{}'", text);

        // Create cancellation token for this generation session
        let cancellation_token = CancellationToken::new();
        {
            let mut token_guard = self.cancellation_token.lock().unwrap();
            *token_guard = Some(cancellation_token.clone());
        }

        // Generate audio from text
        let audio = {
            let mut engine_guard = self.engine.lock().unwrap();
            if let Some(ref mut engine) = *engine_guard {
                engine.create(text, 0, 1.0)
                    .map_err(|e| anyhow::Error::msg(format!("TTS generation failed: {}", e)))?
            } else {
                bail!("TTS engine not initialized");
            }
        };

        // Play audio using rodio with cancellation support
        let result = self.play_audio_direct_with_cancellation(&audio, cancellation_token).await;

        // Always reset playing state
        {
            let mut state = self.write_state();
            state.playing = false;
        }

        // Clear cancellation token on completion
        {
            let mut token_guard = self.cancellation_token.lock().unwrap();
            token_guard.take();
        }

        tracing::debug!("Simple TTS generation completed");
        result
    }

    /// Chunked/streaming speak path - generates and plays chunks in parallel with cancellation support
    async fn speak_chunked(&self, sentences: &[String]) -> Result<()> {
        tracing::debug!("Starting chunked TTS generation for {} sentences", sentences.len());

        // Create cancellation token for this generation session
        let cancellation_token = CancellationToken::new();
        {
            let mut token_guard = self.cancellation_token.lock().unwrap();
            *token_guard = Some(cancellation_token.clone());
        }

        // Get or create sink
        let sink = {
            let sink_guard = self.sink.lock().unwrap();
            sink_guard.clone().ok_or_else(|| anyhow::anyhow!("Audio sink not available"))?
        };

        // Channel for sending generated audio chunks
        let (tx, mut rx) = mpsc::unbounded_channel::<(Vec<i16>, u32)>();

        let engine = Arc::clone(&self.engine);
        let sentences_clone: Vec<String> = sentences.iter().cloned().collect();

        // Generate first chunk immediately and start playing
        let first_sentence = sentences_clone[0].clone();
        let first_audio = {
            let mut engine_guard = self.engine.lock().unwrap();
            if let Some(ref mut eng) = *engine_guard {
                eng.create(&first_sentence, 0, 1.0)
                    .map_err(|e| anyhow::Error::msg(format!("TTS generation failed: {}", e)))?
            } else {
                bail!("TTS engine not initialized");
            }
        };

        // Start playing first chunk immediately (this is the key optimization!)
        let first_samples = self.convert_samples_parallel(&first_audio.samples);
        self.append_audio_to_sink(&sink, first_samples, first_audio.sample_rate)?;

        // Generate remaining chunks in background with cancellation support
        if sentences_clone.len() > 1 {
            let remaining_sentences = sentences_clone[1..].to_vec();
            let tx_clone = tx.clone();
            let cancellation_token_clone = cancellation_token.clone();

            let generation_task = tokio::spawn(async move {
                let mut engine_guard = engine.lock().unwrap();
                if let Some(ref mut eng) = *engine_guard {
                    for sentence in remaining_sentences {
                        // Check for cancellation before each generation
                        if cancellation_token_clone.is_cancelled() {
                            tracing::debug!("Generation task cancelled - stopping generation");
                            break;
                        }

                        // Also check cancellation before sending to channel
                        if cancellation_token_clone.is_cancelled() {
                            tracing::debug!("Generation task cancelled before sending chunk");
                            break;
                        }

                        match eng.create(&sentence, 0, 1.0) {
                            Ok(audio) => {
                                // Convert samples in parallel
                                let samples_i16: Vec<i16> = audio.samples
                                    .par_iter()
                                    .map(|&sample| {
                                        let clamped = sample.max(-1.0).min(1.0);
                                        (clamped * i16::MAX as f32) as i16
                                    })
                                    .collect();

                                // Check cancellation again before sending
                                if cancellation_token_clone.is_cancelled() {
                                    tracing::debug!("Generation task cancelled before sending chunk");
                                    break;
                                }
                                
                                if tx_clone.send((samples_i16, audio.sample_rate)).is_err() {
                                    tracing::debug!("Channel closed, stopping generation");
                                    break; // Receiver dropped
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to generate chunk: {}", e);
                                // Continue with next chunk
                            }
                        }
                    }
                }
            });

            // Store the task handle
            {
                let mut task_guard = self.generation_task.lock().unwrap();
                *task_guard = Some(generation_task);
            }
        }

        // Close the sender so receiver knows when to stop
        drop(tx);

        // Play queued chunks as they arrive (streaming playback) with cancellation
        // CRITICAL: Check cancellation BEFORE each append, as append might block
        let sink_clone = sink.clone();
        let cancellation_token_for_playback = cancellation_token.clone();
        let mut playback_task = tokio::spawn(async move {
            while let Some((samples, sample_rate)) = rx.recv().await {
                // Check cancellation BEFORE appending (append might block synchronously)
                if cancellation_token_for_playback.is_cancelled() {
                    tracing::debug!("Cancellation detected in playback task, stopping");
                    sink_clone.stop();
                    return;
                }
                
                // Append chunk to sink - sink will queue it and play it
                // NOTE: This might block if sink is full, so we check cancellation first
                let source = DirectSampleSource::new(samples, sample_rate);
                sink_clone.append(source);
            }
            tracing::debug!("Playback task completed (all chunks received)");
        });

        // Wait for playback to complete with cancellation support
        let playback_completed = tokio::select! {
            result = &mut playback_task => {
                match result {
                    Ok(()) => tracing::debug!("Playback completed normally"),
                    Err(e) => tracing::warn!("Playback task failed: {}", e),
                }
                true
            }
            _ = cancellation_token.cancelled() => {
                tracing::debug!("Playback cancelled");
                false
            }
        };

        // If playback was cancelled, abort the task and stop sink immediately
        if !playback_completed {
            playback_task.abort();
            // Stop sink synchronously - rodio playback is synchronous
            sink.stop();
            // Small synchronous wait to ensure stop takes effect
            std::thread::sleep(Duration::from_millis(10));
            // Clear the sink to ensure no audio continues
            {
                let mut sink_guard = self.sink.lock().unwrap();
                *sink_guard = None;
            }
            tracing::debug!("Playback stopped immediately due to cancellation");
            return Ok(());
        }

        // Use async polling instead of blocking sleep_until_end()
        // This allows immediate cancellation and makes the function fully async
        // Check cancellation VERY frequently (every 1ms) for instant response
        let sink_for_poll = sink.clone();
        let cancellation_token_poll = cancellation_token.clone();
        
        // Spawn a task that continuously checks cancellation and sink status
        let poll_task = tokio::spawn(async move {
            loop {
                // Check cancellation FIRST - highest priority
                if cancellation_token_poll.is_cancelled() {
                    tracing::debug!("Cancellation detected in poll task, stopping sink");
                    // Stop sink synchronously - rodio playback is synchronous
                    sink_for_poll.stop();
                    std::thread::sleep(Duration::from_millis(10)); // Ensure stop takes effect
                    return;
                }
                
                // Check if sink is empty (non-blocking)
                if sink_for_poll.empty() {
                    // Double-check after a tiny delay, but check cancellation during wait
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(10)) => {
                            if sink_for_poll.empty() {
                                tracing::debug!("All audio playback completed");
                                return;
                            }
                        }
                        _ = cancellation_token_poll.cancelled() => {
                            tracing::debug!("Cancellation during empty check, stopping sink");
                            // Stop sink synchronously - rodio playback is synchronous
                            sink_for_poll.stop();
                            std::thread::sleep(Duration::from_millis(10)); // Ensure stop takes effect
                            return;
                        }
                    }
                } else {
                    // Sleep very briefly before next check (1ms for instant cancellation response)
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        });
        
        // Clone handle before select so we can abort if needed
        let poll_task_abort = poll_task.abort_handle();
        
        // Wait for poll task with cancellation support
        tokio::select! {
            _ = poll_task => {
                tracing::debug!("Poll task completed");
            }
            _ = cancellation_token.cancelled() => {
                tracing::debug!("Cancellation received, stopping sink and aborting poll");
                // Stop sink synchronously - rodio playback is synchronous
                sink.stop();
                std::thread::sleep(Duration::from_millis(10)); // Ensure stop takes effect
                poll_task_abort.abort();
            }
        }

        // Clear stored task handle
        {
            let mut task_guard = self.generation_task.lock().unwrap();
            task_guard.take();
        }

        tracing::info!("Chunked TTS generation completed");
        Ok(())
    }

    /// Convert f32 samples to i16 using parallel processing (optimization #4)
    fn convert_samples_parallel(&self, samples: &[f32]) -> Vec<i16> {
        samples
            .par_iter()
            .map(|&sample| {
                let clamped = sample.max(-1.0).min(1.0);
                (clamped * i16::MAX as f32) as i16
            })
            .collect()
    }

    /// Append audio directly to sink without WAV encoding (optimization #3)
    fn append_audio_to_sink(&self, sink: &Arc<rodio::Sink>, samples: Vec<i16>, sample_rate: u32) -> Result<()> {
        let source = DirectSampleSource::new(samples, sample_rate);
        sink.append(source);
        Ok(())
    }

    /// Play audio samples using rodio (direct, no WAV encoding)
    async fn play_audio_direct(&self, audio: &sherpa_rs::tts::TtsAudio) -> Result<()> {
        // Use default cancellation token for backward compatibility
        let cancellation_token = CancellationToken::new();
        self.play_audio_direct_with_cancellation(audio, cancellation_token).await
    }

    /// Play audio samples using rodio with cancellation support
    async fn play_audio_direct_with_cancellation(&self, audio: &sherpa_rs::tts::TtsAudio, cancellation_token: CancellationToken) -> Result<()> {
        tracing::debug!("Starting audio playback with cancellation support");

        // Get or create sink
        let sink = {
            let sink_guard = self.sink.lock().unwrap();
            sink_guard.clone().ok_or_else(|| anyhow::anyhow!("Audio sink not available"))?
        };

        // Convert samples using parallel processing (optimization #4)
        let samples_i16 = self.convert_samples_parallel(&audio.samples);

        // Append directly without WAV encoding (optimization #3)
        self.append_audio_to_sink(&sink, samples_i16, audio.sample_rate)?;

        // Use async polling instead of blocking sleep_until_end()
        // This allows immediate cancellation and makes the function fully async
        let sink_for_poll = sink.clone();
        let cancellation_token_poll = cancellation_token.clone();
        
        // Spawn a task that continuously checks cancellation and sink status
        let poll_task = tokio::spawn(async move {
            loop {
                // Check cancellation FIRST - highest priority
                if cancellation_token_poll.is_cancelled() {
                    tracing::debug!("Cancellation detected in poll task, stopping sink");
                    // Stop sink synchronously - rodio playback is synchronous
                    sink_for_poll.stop();
                    std::thread::sleep(Duration::from_millis(10)); // Ensure stop takes effect
                    return;
                }
                
                // Check if sink is empty (non-blocking)
                if sink_for_poll.empty() {
                    // Double-check after a tiny delay, but check cancellation during wait
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(10)) => {
                            if sink_for_poll.empty() {
                                tracing::debug!("Audio playback completed normally");
                                return;
                            }
                        }
                        _ = cancellation_token_poll.cancelled() => {
                            tracing::debug!("Cancellation during empty check, stopping sink");
                            // Stop sink synchronously - rodio playback is synchronous
                            sink_for_poll.stop();
                            std::thread::sleep(Duration::from_millis(10)); // Ensure stop takes effect
                            return;
                        }
                    }
                } else {
                    // Sleep very briefly before next check (1ms for instant cancellation response)
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        });
        
        // Clone handle before select so we can abort if needed
        let poll_task_abort = poll_task.abort_handle();
        
        // Wait for poll task with cancellation support
        tokio::select! {
            _ = poll_task => {
                tracing::debug!("Poll task completed");
            }
            _ = cancellation_token.cancelled() => {
                tracing::debug!("Cancellation received, stopping sink and aborting poll");
                // Stop sink synchronously - rodio playback is synchronous
                sink.stop();
                std::thread::sleep(Duration::from_millis(10)); // Ensure stop takes effect
                poll_task_abort.abort();
            }
        }

        Ok(())
    }


    /// Stop current speech immediately with cancellation
    /// 
    /// This method MUST return quickly (< 100ms) to avoid timeout in daemon.
    /// It does NOT wait for tasks to complete - it just cancels and stops.
    pub fn stop(&self) -> Result<()> {
        tracing::debug!("TTS stop requested");

        let mut state = self.write_state();

        if !state.playing {
            tracing::debug!("TTS not playing, nothing to stop");
            return Ok(());
        }

        // Set playing to false immediately to prevent new operations
        state.playing = false;
        drop(state);

        // Cancel cancellation token first to interrupt any waiting operations
        // This is fast and non-blocking
        {
            let mut token_guard = self.cancellation_token.lock().unwrap();
            if let Some(token) = token_guard.take() {
                token.cancel();
                tracing::debug!("Cancellation token triggered");
            }
        }

        // Stop audio playback IMMEDIATELY - this kills playback in the middle of a sentence
        // CRITICAL: When we clone the sink with Arc, all clones point to the SAME sink
        // But the REAL driver is the STREAM - dropping the stream stops ALL sinks immediately
        // So we drop the stream FIRST, which invalidates all sinks and stops playback instantly
        
        // First, stop the sink (affects all Arc clones since they share the same sink)
        let mut sink_guard = self.sink.lock().unwrap();
        if let Some(ref sink) = *sink_guard {
            tracing::debug!("Stopping audio sink (affects all Arc clones)");
            sink.stop();
        }
        *sink_guard = None;
        drop(sink_guard);
        
        // CRITICAL: Drop the stream FIRST - this is what actually drives audio playback
        // Dropping the stream invalidates ALL sinks immediately, stopping playback instantly
        // This is the nuclear option that works even if Arc clones are still holding sinks
        let mut stream_guard = self._stream.lock().unwrap();
        *stream_guard = None;
        drop(stream_guard);
        
        tracing::debug!("Stream dropped - all sinks (including Arc clones) are now invalid, playback stopped");

        // Cancel any ongoing generation task (non-blocking, don't wait)
        // We don't wait for it to complete - just cancel the token and let it finish in background
        {
            let mut task_guard = self.generation_task.lock().unwrap();
            if let Some(task) = task_guard.take() {
                task.abort(); // Abort immediately, don't wait
            }
        }

        tracing::info!("TTS stopped successfully (playback killed immediately)");
        Ok(())
    }

    /// Cancel any ongoing generation task with timeout
    fn cancel_generation_task(&self) -> Result<()> {
        tracing::debug!("Cancelling TTS generation task");

        // Cancel via token first
        {
            let mut token_guard = self.cancellation_token.lock().unwrap();
            if let Some(token) = token_guard.take() {
                token.cancel();
                tracing::debug!("Cancellation token triggered");
            }
        }

        // Wait for task completion with timeout
        let mut task_guard = self.generation_task.lock().unwrap();
        if let Some(task) = task_guard.take() {
            tracing::debug!("Waiting for generation task to complete...");
            let rt = tokio::runtime::Handle::try_current();
            match rt {
                Ok(handle) => {
                    // In async context, use timeout
                    match handle.block_on(async {
                        tokio::time::timeout(Duration::from_millis(500), task).await
                    }) {
                        Ok(Ok(())) => tracing::debug!("Generation task completed gracefully"),
                        Ok(Err(e)) => tracing::warn!("Generation task failed: {}", e),
                        Err(_) => tracing::warn!("Generation task timeout, dropping"),
                    }
                }
                Err(_) => {
                    // Not in async context, create runtime
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    match rt.block_on(async {
                        tokio::time::timeout(Duration::from_millis(500), task).await
                    }) {
                        Ok(Ok(())) => tracing::debug!("Generation task completed gracefully"),
                        Ok(Err(e)) => tracing::warn!("Generation task failed: {}", e),
                        Err(_) => tracing::warn!("Generation task timeout, dropping"),
                    }
                }
            }
        }

        Ok(())
    }

    /// Set the language (e.g., "pl", "en", "pl-PL")
    /// 
    /// This will reload the TTS engine with the appropriate model for the language.
    /// The language code is normalized (e.g., "pl-PL" -> "pl").
    pub async fn set_language(&self, lang: &str) -> Result<()> {
        let normalized_lang = normalize_language_code(lang);
        
        // Check if language is enabled in config
        let config = self.shared_config.config();
        let lang_config = config.tts.languages.get(&normalized_lang)
            .ok_or_else(|| anyhow::anyhow!("Language '{}' not configured", normalized_lang))?;
        
        if !lang_config.enabled {
            anyhow::bail!("Language '{}' is not enabled in config", normalized_lang);
        }
        
        // Update state
        {
            let mut state = self.write_state();
            state.current_language = normalized_lang.clone();
            state.initialized = false; // Force reinitialization
        }
        
        // Reinitialize with new language
        let mut engine_guard = self.engine.lock().unwrap();
        *engine_guard = None;
        drop(engine_guard);
        
        // Initialize with new language
        self.init_with_language(&normalized_lang).await?;
        
        Ok(())
    }

    /// Check if currently speaking
    pub fn is_playing(&self) -> bool {
        let state = self.read_state();
        state.playing
    }

    /// Get current language
    pub fn current_language(&self) -> String {
        let state = self.read_state();
        state.current_language.clone()
    }
}

impl TtsService {
    /// Get the model path (if initialized)
    pub fn model_path(&self) -> Option<PathBuf> {
        let path_guard = self.model_path.lock().unwrap();
        path_guard.clone()
    }
}

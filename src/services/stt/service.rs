//! Speech-to-Text (STT) Service
//!
//! Provides offline speech recognition using Whisper models from sherpa-rs.
//! Handles audio capture, pause detection, and transcription.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use super::audio_processor::AudioProcessor;
use super::audio_utils::{
    calculate_audio_stats, has_sufficient_amplitude, validate_and_clean_audio, TARGET_SAMPLE_RATE,
};
use super::pause_detector::PauseDetector;
use crate::config::{ConfigLoader, ConfigValidator};
use crate::services::ModelManager;
use crate::utils::{format_timestamp, play_beep, play_beep_blocking, BEEP_HIGH_WAV, BEEP_LOW_WAV};

// Audio processing constants
const MIN_AUDIO_DURATION: usize = 8000; // 0.5 seconds at 16kHz
const AUDIO_TIMEOUT: Duration = Duration::from_secs(2);
const MIN_AMPLITUDE_THRESHOLD: f32 = 0.001;

/// Speech-to-Text Service
///
/// Provides offline speech recognition using Whisper models.
/// 
/// # Features
/// - Automatic model download and management
/// - Real-time audio capture from default input device
/// - Pause detection for automatic transcription
/// - Callback-based result reporting
/// 
/// # Architecture
/// - Uses `AudioProcessor` for audio preprocessing (resampling, mono conversion)
/// - Uses `PauseDetector` for silence/pause detection
/// - Uses `ModelManager` for model lifecycle management
/// 
/// # Example
/// ```no_run
/// use ttsandsttp::SttService;
/// 
/// # async fn example() -> anyhow::Result<()> {
/// let stt = SttService::new()?;
/// stt.init().await?;
/// 
/// stt.on_result(|text| {
///     println!("Recognized: {}", text);
/// });
/// 
/// stt.start_listening("en", std::time::Duration::from_secs(2)).await?;
/// let result = stt.stop_listening()?;
/// # Ok(())
/// # }
/// ```
pub struct SttService {
    recognizer: Arc<Mutex<Option<sherpa_rs::whisper::WhisperRecognizer>>>,
    state: Arc<RwLock<SttState>>, // Use RwLock for read-heavy access
    model_manager: ModelManager,
    model_path: Arc<Mutex<Option<PathBuf>>>,
    result_callback: Arc<Mutex<Option<Box<dyn Fn(&str) + Send + Sync>>>>,
    pause_callback: Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>,
    error_callback: Arc<Mutex<Option<Box<dyn Fn(String) + Send + Sync>>>>,
    audio_thread_handle: Arc<Mutex<Option<(std::thread::Thread, std::sync::mpsc::Sender<()>)>>>,
    audio_task_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    decode_complete_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<String>>>>,
    decode_complete_rx: Arc<Mutex<Option<tokio::sync::oneshot::Receiver<String>>>>,
    config: Arc<Mutex<Option<crate::config::AppConfig>>>, // Cached config
    registry: Arc<Mutex<Option<crate::config::ModelRegistry>>>, // Cached registry
}

/// Internal state of the STT service
#[derive(Clone, Debug)]
struct SttState {
    initialized: bool,
    listening: bool,
    current_text: String,
    current_language: String,
    pause_duration: Duration,
    beep_played: bool, // Track if low beep was already played (prevents double beep)
}

impl Default for SttState {
    fn default() -> Self {
        Self {
            initialized: false,
            listening: false,
            current_text: String::new(),
            current_language: "en".to_string(),
            pause_duration: Duration::from_secs(2),
            beep_played: false,
        }
    }
}

impl SttService {
    // Helper methods for state access
    
    /// Read state (for read-only operations)
    fn read_state(&self) -> std::sync::RwLockReadGuard<'_, SttState> {
        self.state.read().unwrap()
    }
    
    /// Write state (for write operations)
    fn write_state(&self) -> std::sync::RwLockWriteGuard<'_, SttState> {
        self.state.write().unwrap()
    }
    
    /// Invoke result callback if set
    // this dead code is allowed because it is used in the daemon service
    #[allow(dead_code)]
    fn invoke_result_callback(&self, text: &str) {
        let cb = self.result_callback.lock().unwrap();
        if let Some(ref callback) = *cb {
            callback(text);
        }
    }
    
    /// Invoke pause callback if set
    #[allow(dead_code)]
    fn invoke_pause_callback(&self) {
        let cb = self.pause_callback.lock().unwrap();
        if let Some(ref callback) = *cb {
            callback();
        }
    }
    #[allow(dead_code)]
    /// Invoke error callback if set
    fn invoke_error_callback(&self, error: String) {
        let cb = self.error_callback.lock().unwrap();
        if let Some(ref callback) = *cb {
            callback(error);
        }
    }
    
    /// Create a new STT service instance
    pub fn new() -> Result<Self> {
        let model_manager = ModelManager::new()?;
        
        // Load config and registry
        let config = ConfigLoader::load_or_create()
            .context("Failed to load config")?;
        let registry = ConfigValidator::get_registry()
            .context("Failed to load model registry")?;
        
        Ok(Self {
            recognizer: Arc::new(Mutex::new(None)),
            state: Arc::new(RwLock::new(SttState::default())),
            model_manager,
            model_path: Arc::new(Mutex::new(None)),
            result_callback: Arc::new(Mutex::new(None)),
            pause_callback: Arc::new(Mutex::new(None)),
            error_callback: Arc::new(Mutex::new(None)),
            audio_thread_handle: Arc::new(Mutex::new(None)),
            audio_task_handle: Arc::new(Mutex::new(None)),
            decode_complete_tx: Arc::new(Mutex::new(None)),
            decode_complete_rx: Arc::new(Mutex::new(None)),
            config: Arc::new(Mutex::new(Some(config))),
            registry: Arc::new(Mutex::new(Some(registry))),
        })
    }
    
    /// Get language code from language string (e.g., "pl" from "pl" or "pl-PL")
    /// Maps TTS language codes to Whisper language codes
    fn normalize_language_code(&self, lang: &str) -> String {
        // Extract base language code (e.g., "pl" from "pl" or "pl-PL")
        let base_lang = lang.split('-').next().unwrap_or(lang).to_lowercase();
        
        // Whisper uses standard language codes, same as TTS
        // Return the normalized code
        base_lang
    }
    
    /// Get Whisper model files based on config
    fn get_whisper_model_files(&self) -> Result<(PathBuf, PathBuf, PathBuf, String)> {
        let model_id = {
            let config_guard = self.config.lock().unwrap();
            let config = config_guard.as_ref()
                .ok_or_else(|| anyhow::anyhow!("Config not loaded"))?;
            config.stt.model_id.clone()
        };
        
        let registry_guard = self.registry.lock().unwrap();
        let registry = registry_guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Registry not loaded"))?;
        
        let model_info = registry.get_stt_model(&model_id)
            .ok_or_else(|| anyhow::anyhow!("STT model '{}' not found in registry", model_id))?;
        
        // Find encoder, decoder, and tokens files
        let encoder_file = model_info.required_files.iter()
            .find(|f| f.contains("encoder") && f.ends_with(".onnx"))
            .ok_or_else(|| anyhow::anyhow!("No encoder file found for model: {}", model_id))?;
        let decoder_file = model_info.required_files.iter()
            .find(|f| f.contains("decoder") && f.ends_with(".onnx"))
            .ok_or_else(|| anyhow::anyhow!("No decoder file found for model: {}", model_id))?;
        let tokens_file = model_info.required_files.iter()
            .find(|f| f.ends_with("tokens.txt"))
            .ok_or_else(|| anyhow::anyhow!("No tokens file found for model: {}", model_id))?;
        
        // Model files are stored in ~/.local/share/stttts/whisper/{model_id}/
        let models_dir = self.model_manager.models_dir();
        let model_path = models_dir.join("whisper").join(&model_id);
        
        // Handle subdirectories (for archived models)
        let actual_path = self.model_manager.find_actual_model_path(&model_path, &model_id);
        
        let encoder_path = actual_path.join(encoder_file);
        let decoder_path = actual_path.join(decoder_file);
        let tokens_path = actual_path.join(tokens_file);
        
        // Get language code from model info (multilingual or specific language)
        let language_code = model_info.language_code.clone();
        
        Ok((encoder_path, decoder_path, tokens_path, language_code))
    }

    /// Initialize the Whisper STT engine
    /// 
    /// This will automatically download Whisper models if they're not present.
    pub async fn init(&self) -> Result<()> {
        {
            let state = self.read_state();
            if state.initialized {
                return Ok(());
            }
        }

        // Set environment variable early to enable debug mode (prevents C++ exceptions)
        if std::env::var("SHERPA_ONNX_LOG_LEVEL").is_err() {
            std::env::set_var("SHERPA_ONNX_LOG_LEVEL", "DEBUG");
        }

        // Get Whisper model files from config
        let (encoder_file, decoder_file, tokens_file, default_language) = self.get_whisper_model_files()?;

        // Validate model files exist
        Self::validate_model_files(&encoder_file, &decoder_file, &tokens_file)?;

        // Store model path
        {
            let model_path = encoder_file.parent()
                .ok_or_else(|| anyhow::anyhow!("Encoder file has no parent directory"))?;
            let mut path_guard = self.model_path.lock().unwrap();
            *path_guard = Some(model_path.to_path_buf());
        }

        // Get default language from state or use model's default
        let language = {
            let state = self.read_state();
            let current_lang = state.current_language.clone();
            drop(state);
            
            if current_lang.is_empty() || current_lang == "en" {
                // Use model's language code, or "en" as fallback
                if default_language == "multilingual" {
                    "en".to_string() // Default to English for multilingual models
                } else {
                    default_language.clone()
                }
            } else {
                // Use current language from state
                self.normalize_language_code(&current_lang)
            }
        };

        // Create Whisper configuration
        let config = sherpa_rs::whisper::WhisperConfig {
            encoder: encoder_file.to_string_lossy().to_string(),
            decoder: decoder_file.to_string_lossy().to_string(),
            tokens: tokens_file.to_string_lossy().to_string(),
            language: language.clone(),
            bpe_vocab: None,
            tail_paddings: None,
            provider: None,
            num_threads: Some(1),
            debug: true,
        };

        // Initialize Whisper recognizer
        let recognizer = sherpa_rs::whisper::WhisperRecognizer::new(config)
            .map_err(|e| anyhow::anyhow!("Failed to create Whisper recognizer: {}", e))?;

        {
            let mut recognizer_guard = self.recognizer.lock().unwrap();
            *recognizer_guard = Some(recognizer);
        }

        {
            let mut state = self.write_state();
            state.initialized = true;
        }

        eprintln!("[{}] ✅ Whisper STT initialized", format_timestamp());
        Ok(())
    }
    
    /// Initialize the Whisper STT engine with a specific language
    pub async fn init_with_language(&self, lang_code: &str) -> Result<()> {
        let normalized_lang = self.normalize_language_code(lang_code);
        
        // Get Whisper model files from config
        let (encoder_file, decoder_file, tokens_file, model_language) = self.get_whisper_model_files()?;

        // Validate model files exist
        Self::validate_model_files(&encoder_file, &decoder_file, &tokens_file)?;

        // Determine language to use
        // If model is multilingual, use the requested language
        // If model is language-specific, use the model's language
        let language = if model_language == "multilingual" {
            normalized_lang.clone()
        } else {
            // Model is language-specific, use its language
            model_language.clone()
        };

        // Create Whisper configuration with the specified language
        let config = sherpa_rs::whisper::WhisperConfig {
            encoder: encoder_file.to_string_lossy().to_string(),
            decoder: decoder_file.to_string_lossy().to_string(),
            tokens: tokens_file.to_string_lossy().to_string(),
            language: language.clone(),
            bpe_vocab: None,
            tail_paddings: None,
            provider: None,
            num_threads: Some(1),
            debug: true,
        };

        // Initialize Whisper recognizer
        let recognizer = sherpa_rs::whisper::WhisperRecognizer::new(config)
            .map_err(|e| anyhow::anyhow!("Failed to create Whisper recognizer: {}", e))?;

        // Replace old recognizer
        {
            let mut recognizer_guard = self.recognizer.lock().unwrap();
            *recognizer_guard = Some(recognizer);
        }

        // Update state
        {
            let mut state = self.write_state();
            state.initialized = true;
            state.current_language = normalized_lang;
        }

        eprintln!("[{}] ✅ Whisper STT initialized with language: {}", format_timestamp(), language);
        Ok(())
    }

    /// Validate that all required model files exist
    fn validate_model_files(
        encoder: &PathBuf,
        decoder: &PathBuf,
        tokens: &PathBuf,
    ) -> Result<()> {
        if !encoder.exists() {
            anyhow::bail!("Encoder file not found: {:?}", encoder);
        }
        if !decoder.exists() {
            anyhow::bail!("Decoder file not found: {:?}", decoder);
        }
        if !tokens.exists() {
            anyhow::bail!("Tokens file not found: {:?}", tokens);
        }
        Ok(())
    }

    /// Start listening for speech (async)
    pub async fn start_listening(&self, lang: &str, pause_duration: Duration) -> Result<()> {
        let normalized_lang = self.normalize_language_code(lang);
        
        {
            let state = self.read_state();
            let needs_reinit = !state.initialized || state.current_language != normalized_lang;
            drop(state);
            
            if needs_reinit {
                // Update language in state first
                {
                    let mut state = self.write_state();
                    state.current_language = normalized_lang.clone();
                }
                
                // Reinitialize with new language if needed
                if !self.read_state().initialized {
                    self.init().await?;
                } else {
                    // Language changed, need to reinitialize recognizer
                    self.init_with_language(&normalized_lang).await?;
                }
            }
        }

        {
            let mut state = self.write_state();
            if state.listening {
                return Ok(());
            }

            state.listening = true;
            state.current_language = normalized_lang;
            state.pause_duration = pause_duration;
            state.current_text.clear();
            state.beep_played = false; // Reset beep flag for new recording session
        }

        // Create a flag to control when recording actually starts (after beep completes)
        let recording_started = Arc::new(Mutex::new(false));

        // Clone necessary data for the audio capture task
        let recognizer = self.recognizer.clone();
        let result_callback = self.result_callback.clone();
        let pause_callback = self.pause_callback.clone();
        let error_callback = self.error_callback.clone();
        let state_clone = Arc::clone(&self.state);
        let audio_thread_handle = self.audio_thread_handle.clone();
        let audio_task_handle = self.audio_task_handle.clone();
        let pause_duration = pause_duration;
        let recording_started_clone = recording_started.clone();

        // Create channel for decode completion signal
        let (decode_tx, decode_rx) = tokio::sync::oneshot::channel();
        {
            let mut tx_guard = self.decode_complete_tx.lock().unwrap();
            *tx_guard = Some(decode_tx);
            let mut rx_guard = self.decode_complete_rx.lock().unwrap();
            *rx_guard = Some(decode_rx);
        }

        // Start audio capture in a separate task IMMEDIATELY (before beep)
        // This ensures microphone is ready and capturing, but we won't accumulate samples until beep completes
        let decode_complete_tx_clone = self.decode_complete_tx.clone();
        let handle = tokio::spawn(async move {
            if let Err(e) = Self::audio_capture_loop(
                recognizer,
                result_callback,
                pause_callback,
                error_callback.clone(),
                state_clone,
                audio_thread_handle,
                pause_duration,
                recording_started_clone,
                decode_complete_tx_clone,
            )
            .await
            {
                let error_cb = error_callback.lock().unwrap();
                if let Some(ref cb) = *error_cb {
                    cb(format!("Audio capture error: {}", e));
                }
            }
        });
        
        // Store the task handle so we can await it later
        {
            let mut handle_guard = audio_task_handle.lock().unwrap();
            *handle_guard = Some(handle);
        }

        // Play high beep in parallel - once it completes, start accumulating samples
        let recording_started_beep = recording_started.clone();
        tokio::spawn(async move {
            eprintln!("[{}] 🔊 Playing high beep before recording...", format_timestamp());
            if let Err(e) = play_beep(BEEP_HIGH_WAV).await {
                eprintln!("[{}] ⚠️  Failed to play high beep: {}", format_timestamp(), e);
            }
            // Once beep completes, start accumulating samples
            {
                let mut flag = recording_started_beep.lock().unwrap();
                *flag = true;
                eprintln!("[{}] ✅ Recording started (beep completed)", format_timestamp());
            }
        });

        Ok(())
    }

    /// Audio capture loop - handles audio input and recognition
    async fn audio_capture_loop(
        recognizer: Arc<Mutex<Option<sherpa_rs::whisper::WhisperRecognizer>>>,
        result_callback: Arc<Mutex<Option<Box<dyn Fn(&str) + Send + Sync>>>>,
        pause_callback: Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>,
        error_callback: Arc<Mutex<Option<Box<dyn Fn(String) + Send + Sync>>>>,
        state: Arc<RwLock<SttState>>,
        audio_thread_handle: Arc<Mutex<Option<(std::thread::Thread, std::sync::mpsc::Sender<()>)>>>,
        pause_duration: Duration,
        recording_started: Arc<Mutex<bool>>,
        decode_complete_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<String>>>>,
    ) -> Result<()> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        // Get default input device
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("No input device available"))?;

        // Get default input config
        let config = device
            .default_input_config()
            .map_err(|e| anyhow::anyhow!("Failed to get input config: {}", e))?;

        let input_sample_rate = config.sample_rate() as u32;
        let channels = config.channels() as usize;
        
        // Initialize audio processor and pause detector
        let audio_processor = AudioProcessor::new(input_sample_rate, channels);
        let mut pause_detector = PauseDetector::new(pause_duration);

        // Create a channel for audio samples
        let (sample_tx, mut sample_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();

        // Build input stream - run in blocking thread
        // Use a channel to signal the thread to stop
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        
        {
            let handle = std::thread::spawn(move || {
                let stream = match device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let samples: Vec<f32> = data.to_vec();
                        if sample_tx.send(samples).is_err() {
                            // Receiver dropped - normal when stopping
                        }
                    },
                    |err| {
                        eprintln!("[{}] ❌ Audio input error: {}", format_timestamp(), err);
                    },
                    None,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("[{}] ❌ Failed to build stream: {}", format_timestamp(), e);
                        return;
                    }
                };

                if let Err(e) = stream.play() {
                    eprintln!("[{}] ❌ Failed to play stream: {}", format_timestamp(), e);
                    return;
                }

                eprintln!("[{}] ✅ Audio stream started", format_timestamp());
                
                // Wait for stop signal or park (park will be used as fallback)
                // Check stop signal periodically
                loop {
                    if stop_rx.try_recv().is_ok() {
                        // Stop signal received - pause and drop stream
                        let _ = stream.pause();
                        drop(stream);
                        break;
                    }
                    std::thread::park_timeout(Duration::from_millis(100));
                }
                
                eprintln!("[{}] ⚠️  Audio stream thread exiting", format_timestamp());
            });
            
            // Store the thread handle and stop channel
            {
                let mut handle_guard = audio_thread_handle.lock().unwrap();
                *handle_guard = Some((handle.thread().clone(), stop_tx));
            }
        }

        // Process audio samples
        let mut accumulated_audio: Vec<f32> = Vec::new();
        let mut last_sample_time = Instant::now();
        let mut first_sample_received = false;

        loop {
            // Check if we should stop
            let should_stop = {
                let state_guard = state.read().unwrap();
                !state_guard.listening
            };

            if should_stop {
                eprintln!(
                    "[{}] 🛑 Audio capture loop stopping",
                    format_timestamp()
                );

                // Decode any accumulated audio before stopping
                Self::decode_accumulated_audio(
                    &recognizer,
                    &result_callback,
                    &state,
                    &mut accumulated_audio,
                    &decode_complete_tx,
                )
                .await;

                // Signal the audio thread to stop
                {
                    let handle_guard = audio_thread_handle.lock().unwrap();
                    if let Some((ref thread, ref stop_tx)) = *handle_guard {
                        let _ = stop_tx.send(());
                        thread.unpark(); // Also unpark in case it's parked
                    }
                }
                
                drop(sample_rx);
                break;
            }

            tokio::select! {
                // Receive audio samples
                samples = sample_rx.recv() => {
                    if let Some(new_samples) = samples {
                        // Check if recording has started (beep completed)
                        let should_accumulate = {
                            let flag = recording_started.lock().unwrap();
                            *flag
                        };

                        // Only process and accumulate samples after beep completes
                        if should_accumulate {
                            // Process audio: convert to mono and resample
                            let processed_samples = audio_processor.process(new_samples);
                            
                            // Check for pause detection
                            match pause_detector.process_samples(&processed_samples) {
                                Some(true) => {
                                    // Pause detected - stop recording
                                    if accumulated_audio.len() >= MIN_AUDIO_DURATION {
                                        eprintln!(
                                            "[{}] ⏸️  Pause detected, stopping recording...",
                                            format_timestamp(),
                                        );

                                        // Play low beep immediately (only once) - gives audio feedback that recording stopped
                                        // We play it in a spawned task so it doesn't block decode
                                        let should_play_beep = {
                                            let mut state_guard = state.write().unwrap();
                                            if !state_guard.beep_played {
                                                state_guard.beep_played = true;
                                                true
                                            } else {
                                                false
                                            }
                                        };
                                        
                                        if should_play_beep {
                                            eprintln!(
                                                "[{}] 🔊 Playing low beep after recording stopped...",
                                                format_timestamp()
                                            );
                                            // Play beep immediately in background - fire and forget using std::thread
                                            // This gives immediate audio feedback without blocking decode
                                            // Using std::thread instead of tokio::spawn_blocking to avoid thread pool delays
                                            let beep_data = BEEP_LOW_WAV;
                                            std::thread::spawn(move || {
                                                if let Err(e) = play_beep_blocking(beep_data) {
                                                    eprintln!("[{}] ⚠️  Failed to play low beep: {}", format_timestamp(), e);
                                                }
                                            });
                                        }

                                        // Call pause callback BEFORE decode - this allows DBus to emit "processing" status
                                        {
                                            let pause_cb = pause_callback.lock().unwrap();
                                            if let Some(ref cb) = *pause_cb {
                                                cb();
                                            }
                                        }

                                        // Now decode the accumulated audio (this can take time)
                                        eprintln!(
                                            "[{}] 🔍 Decoding {} samples...",
                                            format_timestamp(),
                                            accumulated_audio.len()
                                        );
                                        Self::decode_accumulated_audio(
                                            &recognizer,
                                            &result_callback,
                                            &state,
                                            &mut accumulated_audio,
                                            &decode_complete_tx,
                                        )
                                        .await;

                                        // Stop listening AFTER decode completes
                                        // This ensures stop_listening() can get the decoded result
                                        {
                                            let mut state_guard = state.write().unwrap();
                                            state_guard.listening = false;
                                        }

                                        eprintln!("[{}] 🛑 Stopping due to pause detection", format_timestamp());

                                        // Signal the audio thread to stop
                                        {
                                            let handle_guard = audio_thread_handle.lock().unwrap();
                                            if let Some((ref thread, ref stop_tx)) = *handle_guard {
                                                let _ = stop_tx.send(());
                                                thread.unpark();
                                            }
                                        }

                                        break;
                                    }
                                }
                                Some(false) => {
                                    // Speech/activity detected - continue recording
                                }
                                None => {
                                    // No significant change - continue monitoring
                                }
                            }

                            // Accumulate processed samples
                            accumulated_audio.extend(processed_samples);
                        }
                        // If not accumulating yet, samples are discarded (but stream stays active)

                        last_sample_time = Instant::now();

                        if !first_sample_received {
                            first_sample_received = true;
                            eprintln!("[{}] ✅ Audio capture confirmed (stream active, waiting for beep...)", format_timestamp());
                        }
                    } else {
                        eprintln!("[{}] ⚠️  Audio channel closed", format_timestamp());
                        break;
                    }
                }
                // Periodic check for audio timeout
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                    Self::check_audio_timeout(
                        first_sample_received,
                        &last_sample_time,
                        &error_callback,
                    );
                }
            }
        }

        // Clean up: signal the audio thread to stop
        {
            let handle_guard = audio_thread_handle.lock().unwrap();
            if let Some((ref thread, ref stop_tx)) = *handle_guard {
                let _ = stop_tx.send(());
                thread.unpark();
            }
        }

        Ok(())
    }


    /// Check for audio timeout and report errors
    fn check_audio_timeout(
        first_sample_received: bool,
        last_sample_time: &Instant,
        error_callback: &Arc<Mutex<Option<Box<dyn Fn(String) + Send + Sync>>>>,
    ) {
        if !first_sample_received && last_sample_time.elapsed() > AUDIO_TIMEOUT {
            let ts = format_timestamp();
            let error_msg = format!(
                "⚠️  WARNING: No audio samples received after {} seconds. Audio capture may not be working!",
                AUDIO_TIMEOUT.as_secs()
            );
            eprintln!("[{}] {}", ts, error_msg);
            eprintln!("[{}]    Check: Is your microphone enabled? Is the audio device working?", ts);

            let error_cb = error_callback.lock().unwrap();
            if let Some(ref cb) = *error_cb {
                cb(error_msg);
            }
        }
    }

    /// Decode accumulated audio using Whisper
    async fn decode_accumulated_audio(
        recognizer: &Arc<Mutex<Option<sherpa_rs::whisper::WhisperRecognizer>>>,
        result_callback: &Arc<Mutex<Option<Box<dyn Fn(&str) + Send + Sync>>>>,
        state: &Arc<RwLock<SttState>>,
        accumulated_audio: &mut Vec<f32>,
        decode_complete_tx: &Arc<Mutex<Option<tokio::sync::oneshot::Sender<String>>>>,
    ) {
        if accumulated_audio.len() < MIN_AUDIO_DURATION {
            eprintln!(
                "[{}] ⚠️  Not enough accumulated audio ({} < {} samples), skipping decode",
                format_timestamp(),
                accumulated_audio.len(),
                MIN_AUDIO_DURATION
            );
            return;
        }

        // Validate audio has sufficient amplitude
        if !has_sufficient_amplitude(accumulated_audio, MIN_AMPLITUDE_THRESHOLD) {
            let stats = calculate_audio_stats(accumulated_audio);
            eprintln!(
                "[{}] ⚠️  Accumulated audio is too quiet (max_abs={:.6}), skipping decode",
                format_timestamp(),
                stats.max_abs
            );
            return;
        }

        // Take ownership of audio data
        let mut audio_to_decode = std::mem::take(accumulated_audio);

        // Validate and clean audio
        let validation_stats = validate_and_clean_audio(&mut audio_to_decode);
        if validation_stats.has_invalid {
            eprintln!(
                "[{}] ⚠️  Found invalid samples (NaN/Inf), cleaned them",
                format_timestamp()
            );
        }

        // Log audio stats
        let stats = calculate_audio_stats(&audio_to_decode);
        eprintln!(
            "[{}] 🔍 Decoding {} samples ({:.2}s) at {}Hz...",
            format_timestamp(),
            stats.sample_count,
            stats.sample_count as f32 / TARGET_SAMPLE_RATE as f32,
            TARGET_SAMPLE_RATE
        );

        // Skip decode if environment variable is set (for testing)
        let skip_decode = std::env::var("SKIP_DECODE").is_ok();
        if skip_decode {
            eprintln!(
                "[{}] ⏭️  SKIP_DECODE: Would decode {} samples",
                format_timestamp(),
                audio_to_decode.len()
            );
            return;
        }

        // Transcribe using Whisper
        let decode_start = Instant::now();
        let text = {
            let mut rec_guard = recognizer.lock().unwrap();
            if let Some(ref mut rec) = *rec_guard {
                let result = rec.transcribe(TARGET_SAMPLE_RATE, &audio_to_decode);
                result.text
            } else {
                String::new()
            }
        };

        let decode_duration = decode_start.elapsed();
        eprintln!(
            "[{}] ✅ Decode completed: '{}' (took {:.3}s)",
            format_timestamp(),
            if text.is_empty() { "(empty)" } else { &text },
            decode_duration.as_secs_f64()
        );

        // Always update state with decoded text (even if empty)
        {
            let mut state_guard = state.write().unwrap();
            state_guard.current_text = text.clone();
        }

        if !text.is_empty() {
            // Call result callback
            {
                let result_cb = result_callback.lock().unwrap();
                if let Some(ref cb) = *result_cb {
                    cb(&text);
                }
            }
        }

        // Signal decode completion
        {
            let mut tx_guard = decode_complete_tx.lock().unwrap();
            if let Some(tx) = tx_guard.take() {
                let _ = tx.send(text);
            }
        }
    }

    /// Stop listening and return the recognized text
    pub fn stop_listening(&self) -> Result<String> {
        eprintln!("[{}] 🔍 stop_listening() called", format_timestamp());
        let was_listening = {
            let state = self.read_state();
            state.listening
        };

        let mut state = self.write_state();

        if !state.listening {
            eprintln!("[{}] 🔍 Already stopped, returning current text", format_timestamp());
            return Ok(state.current_text.clone());
        }

        eprintln!("[{}] 🔍 Setting listening=false", format_timestamp());
        state.listening = false;
        drop(state);

        // Play low beep if we were actually listening and beep hasn't been played yet
        if was_listening {
            let mut state_guard = self.write_state();
            if !state_guard.beep_played {
                state_guard.beep_played = true;
                drop(state_guard);
                
                eprintln!(
                    "[{}] 🔊 Playing low beep after recording stopped...",
                    format_timestamp()
                );
                if let Err(e) = play_beep_blocking(BEEP_LOW_WAV) {
                    eprintln!("[{}] ⚠️  Failed to play low beep: {}", format_timestamp(), e);
                }
            }
        }

        // Signal the audio thread to stop
        {
            let handle_guard = self.audio_thread_handle.lock().unwrap();
            if let Some((ref thread, ref stop_tx)) = *handle_guard {
                let _ = stop_tx.send(());
                thread.unpark();
            }
        }
        
        // Clear the task handle - the task will complete when audio_capture_loop returns
        // Note: We can't await here since this is a sync function, but dropping the handle
        // will allow the task to be cleaned up when it completes
        {
            let mut handle_guard = self.audio_task_handle.lock().unwrap();
            handle_guard.take();
        }
        
        // Wait for decode to complete using async coordination (with timeout)
        // The decode happens asynchronously in the audio loop
        let text = {
            let mut rx_guard = self.decode_complete_rx.lock().unwrap();
            if let Some(rx) = rx_guard.take() {
                // Use blocking wait with timeout
                let handle = tokio::runtime::Handle::try_current();
                if let Ok(handle) = handle {
                    // We're in an async context, use timeout
                    match handle.block_on(async {
                        tokio::time::timeout(Duration::from_secs(5), rx).await
                    }) {
                        Ok(Ok(result)) => {
                            eprintln!(
                                "[{}] 🔍 stop_listening() received decode result via channel",
                                format_timestamp()
                            );
                            result
                        }
                        Ok(Err(_)) => {
                            // Channel was closed, get text from state
                            let state = self.read_state();
                            state.current_text.clone()
                        }
                        Err(_) => {
                            // Timeout - get text from state
                            eprintln!(
                                "[{}] 🔍 stop_listening() decode timeout, using state text",
                                format_timestamp()
                            );
                            let state = self.read_state();
                            state.current_text.clone()
                        }
                    }
                } else {
                    // Not in async context, use blocking wait
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    match rt.block_on(async {
                        tokio::time::timeout(Duration::from_secs(5), rx).await
                    }) {
                        Ok(Ok(result)) => result,
                        _ => {
                            let state = self.read_state();
                            state.current_text.clone()
                        }
                    }
                }
            } else {
                // No receiver available, get text from state
                let state = self.read_state();
                state.current_text.clone()
            }
        };

        // Drop the recognizer AFTER decode completes
        // This ensures sherpa-rs resources are cleaned up after decode is done
        {
            let mut recognizer_guard = self.recognizer.lock().unwrap();
            if let Some(recognizer) = recognizer_guard.take() {
                drop(recognizer);
                eprintln!("[{}] 🔧 WhisperRecognizer dropped", format_timestamp());
            }
        }

        eprintln!(
            "[{}] 🔍 stop_listening() returning text: '{}'",
            format_timestamp(),
            if text.is_empty() { "(empty)" } else { &text }
        );
        Ok(text)
    }

    /// Cancel listening without returning text
    pub fn cancel(&self) -> Result<()> {
        let mut state = self.write_state();

        if !state.listening {
            return Ok(());
        }

        state.listening = false;
        state.current_text.clear();
        Ok(())
    }

    /// Set callback for partial results
    pub fn on_result<F>(&self, callback: F)
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        let mut cb = self.result_callback.lock().unwrap();
        *cb = Some(Box::new(callback));
    }

    /// Set callback when pause is detected
    pub fn on_pause_detected<F>(&self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut cb = self.pause_callback.lock().unwrap();
        *cb = Some(Box::new(callback));
    }

    /// Set error callback
    pub fn on_error<F>(&self, callback: F)
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        let mut cb = self.error_callback.lock().unwrap();
        *cb = Some(Box::new(callback));
    }

    /// Check if currently listening
    pub fn is_listening(&self) -> bool {
        let state = self.read_state();
        state.listening
    }

    /// Get current recognized text
    pub fn current_text(&self) -> String {
        let state = self.read_state();
        state.current_text.clone()
    }

    /// Get the model path (if initialized)
    pub fn model_path(&self) -> Option<PathBuf> {
        let path_guard = self.model_path.lock().unwrap();
        path_guard.clone()
    }
}

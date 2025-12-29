//! Text-to-Speech (TTS) Service
//!
//! Provides offline text-to-speech synthesis using VITS models from sherpa-rs.
//! Handles model management, audio generation, and playback.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use rayon::prelude::*;

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
        self.speak_chunked(&sentences).await?;

        {
            let mut state = self.write_state();
            state.playing = false;
        }
        
        Ok(())
    }

    /// Simple speak path for short text (no chunking overhead)
    async fn speak_simple(&self, text: &str) -> Result<()> {
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

        // Play audio using rodio
        self.play_audio_direct(&audio).await?;

        {
            let mut state = self.write_state();
            state.playing = false;
        }
        
        Ok(())
    }

    /// Chunked/streaming speak path - generates and plays chunks in parallel
    async fn speak_chunked(&self, sentences: &[String]) -> Result<()> {
        use tokio::sync::mpsc;

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

        // Generate remaining chunks in background and queue them
        if sentences_clone.len() > 1 {
            let remaining_sentences = sentences_clone[1..].to_vec();
            let tx_clone = tx.clone();
            tokio::task::spawn_blocking(move || {
                let mut engine_guard = engine.lock().unwrap();
                if let Some(ref mut eng) = *engine_guard {
                    for sentence in remaining_sentences {
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
                                
                                if tx_clone.send((samples_i16, audio.sample_rate)).is_err() {
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
        }

        // Close the sender so receiver knows when to stop
        drop(tx);

        // Play queued chunks as they arrive (streaming playback)
        while let Some((samples, sample_rate)) = rx.recv().await {
            self.append_audio_to_sink(&sink, samples, sample_rate)?;
        }

        // Wait for all playback to complete
        let sink_for_wait = sink.clone();
        tokio::task::spawn_blocking(move || {
            sink_for_wait.sleep_until_end();
        }).await?;

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

        // Get or create sink
        let sink = {
            let sink_guard = self.sink.lock().unwrap();
            sink_guard.clone().ok_or_else(|| anyhow::anyhow!("Audio sink not available"))?
        };

        // Convert samples using parallel processing (optimization #4)
        let samples_i16 = self.convert_samples_parallel(&audio.samples);

        // Append directly without WAV encoding (optimization #3)
        self.append_audio_to_sink(&sink, samples_i16, audio.sample_rate)?;

        // Wait for playback to complete
        let sink_for_wait = sink.clone();
        tokio::task::spawn_blocking(move || {
            sink_for_wait.sleep_until_end();
        }).await?;

        Ok(())
    }


    /// Stop current speech
    pub fn stop(&self) -> Result<()> {
        let mut state = self.write_state();
        
        if !state.playing {
            return Ok(());
        }

        // Stop audio playback
        let mut sink_guard = self.sink.lock().unwrap();
        if let Some(ref sink) = *sink_guard {
            sink.stop();
        }
        *sink_guard = None;

        state.playing = false;
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

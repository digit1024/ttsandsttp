//! Text-to-Speech (TTS) Service
//!
//! Provides offline text-to-speech synthesis using VITS models from sherpa-rs.
//! Handles model management, audio generation, and playback.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use crate::config::{ConfigLoader, ConfigValidator};
use crate::services::ModelManager;

/// Text-to-Speech Service
///
/// Provides offline text-to-speech synthesis using VITS models.
/// 
/// # Features
/// - Automatic model download and management
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
    config: Arc<Mutex<Option<crate::config::AppConfig>>>, // Cached config
    registry: Arc<Mutex<Option<crate::config::ModelRegistry>>>, // Cached registry
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
        let model_manager = ModelManager::new()?;
        
        // Load config and registry
        let config = ConfigLoader::load_or_create()
            .context("Failed to load config")?;
        let registry = ConfigValidator::get_registry()
            .context("Failed to load model registry")?;
        
        Ok(Self {
            engine: Arc::new(Mutex::new(None)),
            state: Arc::new(RwLock::new(TtsState::default())),
            model_manager,
            model_path: Arc::new(Mutex::new(None)),
            _stream: Arc::new(Mutex::new(None)),
            sink: Arc::new(Mutex::new(None)),
            config: Arc::new(Mutex::new(Some(config))),
            registry: Arc::new(Mutex::new(Some(registry))),
        })
    }
    
    /// Get language code from language string (e.g., "pl" from "pl" or "pl-PL")
    fn normalize_language_code(&self, lang: &str) -> String {
        // Extract base language code (e.g., "pl" from "pl" or "pl-PL")
        lang.split('-').next().unwrap_or(lang).to_lowercase()
    }
    
    /// Get model files for a language code
    fn get_model_files_for_language(&self, lang_code: &str) -> Result<(PathBuf, PathBuf, PathBuf)> {
        let registry_guard = self.registry.lock().unwrap();
        let registry = registry_guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Registry not loaded"))?;
        
        // Get model_id from config for this language
        let model_id = {
            let config_guard = self.config.lock().unwrap();
            let config = config_guard.as_ref()
                .ok_or_else(|| anyhow::anyhow!("Config not loaded"))?;
            config.tts.languages
                .get(lang_code)
                .map(|lang_cfg| lang_cfg.model_id.clone())
                .ok_or_else(|| anyhow::anyhow!("Language '{}' not configured", lang_code))?
        };
        
        let model_info = registry.get_tts_model(lang_code, &model_id)
            .ok_or_else(|| anyhow::anyhow!("No TTS model '{}' found for language: {}", model_id, lang_code))?;
        
        // Find the .onnx file in required_files
        let onnx_file = model_info.required_files.iter()
            .find(|f| f.ends_with(".onnx"))
            .ok_or_else(|| anyhow::anyhow!("No .onnx file found for language: {}", lang_code))?;
        
        // Model files are stored in ~/.local/share/stttts/tts/{lang_code}/
        let models_dir = self.model_manager.models_dir();
        let model_path = models_dir.join("tts").join(lang_code);
        
        // Handle subdirectories (for archived models)
        let actual_path = self.model_manager.find_actual_model_path(&model_path, &model_info.id);
        
        let model_file = actual_path.join(onnx_file);
        let tokens_file = actual_path.join("tokens.txt");
        
        // For most models, espeak-ng-data is in the same directory
        // For archived models, it might be in a subdirectory
        let data_dir = actual_path.join("espeak-ng-data");
        
        Ok((model_file, tokens_file, data_dir))
    }

    /// Initialize the TTS engine for a specific language
    /// 
    /// This will automatically download models if they're not present.
    pub async fn init_with_language(&self, lang_code: &str) -> Result<()> {
        let normalized_lang = self.normalize_language_code(lang_code);
        
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
            anyhow::bail!("Model file not found: {:?} (language: {})", model_file, normalized_lang);
        }
        if !tokens_file.exists() {
            anyhow::bail!("Tokens file not found: {:?} (language: {})", tokens_file, normalized_lang);
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
            .ok_or_else(|| anyhow::anyhow!("Model file has no parent directory"))?;
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
        let default_lang = {
            let config_guard = self.config.lock().unwrap();
            let config = config_guard.as_ref()
                .ok_or_else(|| anyhow::anyhow!("Config not loaded"))?;
            config.tts.default.clone()
        };
        
        self.init_with_language(&default_lang).await
    }

    /// Speak the given text (async)
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

        {
            let mut state = self.write_state();
            state.playing = true;
        }

        // Generate audio from text
        let audio = {
            let mut engine_guard = self.engine.lock().unwrap();
            if let Some(ref mut engine) = *engine_guard {
                // create(text, speaker_id, speed)
                // speaker_id: 0 for default, speed: 1.0 for normal
                engine.create(text, 0, 1.0)
                    .map_err(|e| anyhow::anyhow!("TTS generation failed: {}", e))?
            } else {
                anyhow::bail!("TTS engine not initialized");
            }
        };

        // Play audio using rodio
        self.play_audio(&audio).await?;

        {
            let mut state = self.write_state();
            state.playing = false;
        }
        
        Ok(())
    }

    /// Play audio samples using rodio
    async fn play_audio(&self, audio: &sherpa_rs::tts::TtsAudio) -> Result<()> {
        use rodio::{Decoder, OutputStreamBuilder, Sink};
        use std::io::Cursor;

        // Create output stream and sink
        let stream = OutputStreamBuilder::open_default_stream()
            .context("Failed to create audio output stream")?;
        
        let mixer = stream.mixer();
        let sink = Arc::new(Sink::connect_new(&mixer));

        // Store stream to keep it alive
        {
            let mut stream_guard = self._stream.lock().unwrap();
            *stream_guard = Some(stream);
        }
        
        // Convert f32 samples to i16 for rodio
        // TtsAudio has samples: Vec<f32> and sample_rate: u32
        let samples_i16: Vec<i16> = audio.samples
            .iter()
            .map(|&sample| {
                // Clamp to [-1.0, 1.0] and convert to i16
                let clamped = sample.max(-1.0).min(1.0);
                (clamped * i16::MAX as f32) as i16
            })
            .collect();

        // Create a source from the samples
        // We need to create a custom source or use a buffer
        // For simplicity, we'll write to a WAV buffer and decode it
        let wav_buffer = self.create_wav_buffer(&samples_i16, audio.sample_rate)?;
        
        let cursor = Cursor::new(wav_buffer);
        let source = Decoder::new(cursor)
            .map_err(|e| anyhow::anyhow!("Failed to create audio decoder: {}", e))?;

        // Append to sink and store it
        sink.append(source);
        {
            let mut sink_guard = self.sink.lock().unwrap();
            *sink_guard = Some(sink.clone());
        }
        
        // Wait for playback to complete (blocking call, run in blocking task)
        let sink_for_wait = sink.clone();
        tokio::task::spawn_blocking(move || {
            sink_for_wait.sleep_until_end();
        }).await?;

        Ok(())
    }

    /// Create a simple WAV buffer from samples
    fn create_wav_buffer(&self, samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
        use std::io::Write;
        
        let mut buffer = Vec::new();
        
        // WAV header
        buffer.write_all(b"RIFF")?;
        let data_size = (samples.len() * 2 + 36) as u32;
        buffer.write_all(&data_size.to_le_bytes())?;
        buffer.write_all(b"WAVE")?;
        
        // fmt chunk
        buffer.write_all(b"fmt ")?;
        buffer.write_all(&16u32.to_le_bytes())?; // fmt chunk size
        buffer.write_all(&1u16.to_le_bytes())?; // audio format (PCM)
        buffer.write_all(&1u16.to_le_bytes())?; // num channels (mono)
        buffer.write_all(&sample_rate.to_le_bytes())?; // sample rate
        let byte_rate = sample_rate * 2; // sample_rate * num_channels * bits_per_sample / 8
        buffer.write_all(&byte_rate.to_le_bytes())?;
        buffer.write_all(&2u16.to_le_bytes())?; // block align
        buffer.write_all(&16u16.to_le_bytes())?; // bits per sample
        
        // data chunk
        buffer.write_all(b"data")?;
        let data_chunk_size = (samples.len() * 2) as u32;
        buffer.write_all(&data_chunk_size.to_le_bytes())?;
        
        // Write samples
        for &sample in samples {
            buffer.write_all(&sample.to_le_bytes())?;
        }
        
        Ok(buffer)
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
        let normalized_lang = self.normalize_language_code(lang);
        
        // Check if language is enabled in config
        let config_guard = self.config.lock().unwrap();
        let config = config_guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Config not loaded"))?;
        
        let lang_config = config.tts.languages.get(&normalized_lang)
            .ok_or_else(|| anyhow::anyhow!("Language '{}' not configured", normalized_lang))?;
        
        if !lang_config.enabled {
            anyhow::bail!("Language '{}' is not enabled in config", normalized_lang);
        }
        drop(config_guard);
        
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

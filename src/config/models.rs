//! Configuration data structures

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub tts: TtsConfig,
    pub stt: SttConfig,
}

/// TTS configuration section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsConfig {
    /// Default language code (e.g., "en")
    pub default: String,
    /// Language-specific configurations
    #[serde(flatten)]
    pub languages: HashMap<String, TtsLanguageConfig>,
}

/// TTS language configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsLanguageConfig {
    /// Whether this language is enabled
    pub enabled: bool,
    /// Model ID to use for this language
    pub model_id: String,
}

/// STT configuration section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttConfig {
    /// Whisper model ID to use
    pub model_id: String,
}


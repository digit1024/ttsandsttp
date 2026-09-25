//! Configuration data structures

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub tts: TtsConfig,
    pub stt: SttConfig,
    /// Cloud API provider settings (DashScope)
    #[serde(default)]
    pub api: ApiConfig,
}

/// TTS provider backend
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// Local sherpa-onnx model
    #[default]
    Local,
    /// Alibaba Cloud DashScope (Qwen) API
    Dashscope,
}

impl Provider {
    pub fn is_dashscope(&self) -> bool {
        matches!(self, Provider::Dashscope)
    }
}

/// DashScope region preset
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ApiRegion {
    /// Singapore international endpoint
    #[default]
    Singapore,
    /// China (Beijing) endpoint
    Beijing,
    /// Fully custom `base_url`
    Custom,
}

impl ApiRegion {
    /// Default API base URL for this region
    pub fn base_url(&self) -> &'static str {
        match self {
            ApiRegion::Singapore => "https://dashscope-intl.aliyuncs.com/api/v1",
            ApiRegion::Beijing => "https://dashscope.aliyuncs.com/api/v1",
            ApiRegion::Custom => "",
        }
    }

    /// Slug used to scope keys per region (keyring/credential/file)
    pub fn slug(&self) -> &'static str {
        match self {
            ApiRegion::Singapore => "singapore",
            ApiRegion::Beijing => "beijing",
            ApiRegion::Custom => "custom",
        }
    }

    /// Default environment variable that holds the API key for this region
    pub fn default_env(&self) -> &'static str {
        "DASHSCOPE_API_KEY"
    }
}

/// Where the API key should be read from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// Try credential dir, then keyring, then file, then env
    #[default]
    Auto,
    /// systemd `$CREDENTIALS_DIRECTORY`
    Credential,
    /// Freedesktop Secret Service (GNOME Keyring / KWallet)
    Keyring,
    /// 0600 file
    File,
    /// Environment variable (dev only)
    Env,
}

/// Cloud API (DashScope) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Region preset
    #[serde(default)]
    pub region: ApiRegion,
    /// Optional explicit base URL (required when `region = "custom"`)
    #[serde(default)]
    pub base_url: String,
    /// Optional env var override; defaults to the region's variable
    #[serde(default)]
    pub api_key_env: String,
    /// Key resolution strategy
    #[serde(default)]
    pub key_source: KeySource,
    /// Optional explicit key file path
    #[serde(default)]
    pub key_file: String,
    /// HTTP request timeout in seconds
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            region: ApiRegion::default(),
            base_url: String::new(),
            api_key_env: String::new(),
            key_source: KeySource::default(),
            key_file: String::new(),
            timeout_secs: default_timeout_secs(),
        }
    }
}

impl ApiConfig {
    /// Resolve the effective base URL
    pub fn effective_base_url(&self) -> String {
        if !self.base_url.is_empty() {
            self.base_url.clone()
        } else {
            self.region.base_url().to_string()
        }
    }

    /// Resolve the effective API key environment variable name
    pub fn effective_env(&self) -> String {
        if !self.api_key_env.is_empty() {
            self.api_key_env.clone()
        } else {
            self.region.default_env().to_string()
        }
    }
}

fn default_timeout_secs() -> u64 {
    60
}

fn default_true() -> bool {
    true
}

fn default_asr_model() -> String {
    "qwen3-asr-flash".to_string()
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
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Backend provider for this language
    #[serde(default)]
    pub provider: Provider,
    /// Local sherpa model ID (required when `provider = "local"`)
    #[serde(default)]
    pub model_id: String,
    /// Cloud model ID (e.g. "qwen3-tts-flash")
    #[serde(default = "default_tts_model")]
    pub api_model: String,
    /// Cloud voice name (e.g. "Cherry")
    #[serde(default)]
    pub voice: String,
    /// Cloud `language_type` (e.g. "English"); derived from the language code if empty
    #[serde(default)]
    pub language_type: String,
    /// Use SSE streaming PCM playback (cloud only)
    #[serde(default = "default_true")]
    pub stream: bool,
}

fn default_tts_model() -> String {
    "qwen3-tts-flash".to_string()
}

/// STT configuration section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttConfig {
    /// Backend provider
    #[serde(default)]
    pub provider: Provider,
    /// Whisper model ID (required when `provider = "local"`)
    #[serde(default)]
    pub model_id: String,
    /// Cloud model ID (e.g. "qwen3-asr-flash")
    #[serde(default = "default_asr_model")]
    pub api_model: String,
    /// Optional fixed language hint for the cloud ASR (empty = use requested language)
    #[serde(default)]
    pub api_language: String,
}

/// Map a normalized language code to a DashScope `language_type` value
pub fn language_type_for_code(lang: &str) -> &'static str {
    match lang {
        "zh" => "Chinese",
        "en" => "English",
        "fr" => "French",
        "de" => "German",
        "ru" => "Russian",
        "it" => "Italian",
        "es" => "Spanish",
        "pt" => "Portuguese",
        "ja" => "Japanese",
        "ko" => "Korean",
        _ => "English",
    }
}

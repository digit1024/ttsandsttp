//! Configuration data structures

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::utils::normalize_language_code;

/// Provider id used for the offline sherpa/piper backends.
pub const LOCAL_PROVIDER: &str = "local";

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub tts: TtsConfig,
    pub stt: SttConfig,
    /// Legacy DashScope section. Migrated into `[providers.dashscope]` on load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<ApiConfig>,
    /// Named cloud provider sections (catalog presets and custom entries).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub providers: HashMap<String, ProviderConfig>,
}

impl AppConfig {
    /// Move a legacy `[api]` section into `[providers.dashscope]`.
    /// Returns true when a migration happened. An explicit
    /// `[providers.dashscope]` section always wins over the legacy one.
    pub fn migrate_legacy(&mut self) -> bool {
        let Some(api) = self.api.take() else {
            return false;
        };
        if self.providers.contains_key("dashscope") {
            return true;
        }
        self.providers.insert(
            "dashscope".to_string(),
            ProviderConfig {
                label: String::new(),
                kind: "dashscope".to_string(),
                base_url: api.base_url,
                region: Some(api.region),
                stt_endpoint: String::new(),
                tts_endpoint: String::new(),
                api_key_env: api.api_key_env,
                key_source: api.key_source,
                key_file: api.key_file,
                timeout_secs: api.timeout_secs,
                requires_api_key: None,
            },
        );
        true
    }

    /// DashScope settings for the `dashscope` provider, with defaults.
    pub fn dashscope_api(&self) -> ApiConfig {
        let mut api = ApiConfig::default();
        if let Some(pc) = self.providers.get("dashscope") {
            if let Some(region) = pc.region {
                api.region = region;
            }
            api.base_url = pc.base_url.clone();
            api.api_key_env = pc.api_key_env.clone();
            api.key_source = pc.key_source;
            api.key_file = pc.key_file.clone();
            if pc.timeout_secs > 0 {
                api.timeout_secs = pc.timeout_secs;
            }
        }
        api
    }

    /// The user config section for a provider, if present.
    pub fn provider_config(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.get(id)
    }
}

/// User-facing configuration for one provider (`[providers.<id>]`).
///
/// Catalog presets only need the section to exist (or can be referenced
/// without one); custom providers must set `kind` plus `base_url` or
/// explicit endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Display label override.
    #[serde(default)]
    pub label: String,
    /// `openai` or `dashscope`; required for ids not in the catalog.
    #[serde(default)]
    pub kind: String,
    /// API base URL (OpenAI-compatible) or DashScope base override.
    #[serde(default)]
    pub base_url: String,
    /// DashScope region preset (singapore | beijing | custom).
    #[serde(default)]
    pub region: Option<ApiRegion>,
    /// Full STT endpoint override.
    #[serde(default)]
    pub stt_endpoint: String,
    /// Full TTS endpoint override.
    #[serde(default)]
    pub tts_endpoint: String,
    /// Env var holding the API key (defaults to the catalog's).
    #[serde(default)]
    pub api_key_env: String,
    /// Key resolution strategy.
    #[serde(default)]
    pub key_source: KeySource,
    /// Explicit key file path.
    #[serde(default)]
    pub key_file: String,
    /// HTTP timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Set false to allow key-less access (local OpenAI-compatible servers).
    #[serde(default)]
    pub requires_api_key: Option<bool>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            label: String::new(),
            kind: String::new(),
            base_url: String::new(),
            region: None,
            stt_endpoint: String::new(),
            tts_endpoint: String::new(),
            api_key_env: String::new(),
            key_source: KeySource::default(),
            key_file: String::new(),
            timeout_secs: default_timeout_secs(),
            requires_api_key: None,
        }
    }
}

impl ProviderConfig {
    pub fn needs_key(&self) -> bool {
        self.requires_api_key.unwrap_or(true)
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

fn default_local_provider() -> String {
    LOCAL_PROVIDER.to_string()
}

fn default_extra_params() -> Value {
    Value::Null
}

/// TTS configuration section
///
/// The top-level fields (besides `default`) form an optional global fallback
/// used for any language without its own `[tts.<lang>]` section. This makes
/// the language optional for multilingual cloud models: one `[tts]` provider
/// can speak every language. The fallback must be a cloud provider — local
/// models are language-specific and need `[tts.<lang>]` sections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsConfig {
    /// Default language code (used when the caller does not specify one)
    pub default: String,
    /// Global fallback provider id (empty = no fallback)
    #[serde(default)]
    pub provider: String,
    /// Global fallback cloud model
    #[serde(default)]
    pub model: String,
    /// Accepted for symmetry; the global fallback must be a cloud provider,
    /// so a local `model_id` here is rejected by the validator.
    #[serde(default)]
    pub model_id: String,
    /// Global fallback voice
    #[serde(default)]
    pub voice: String,
    /// Global fallback style instructions
    #[serde(default)]
    pub instructions: String,
    /// Global fallback DashScope `language_type`
    #[serde(default)]
    pub language_type: String,
    /// Global fallback DashScope SSE streaming
    #[serde(default = "default_true")]
    pub stream: bool,
    /// Global fallback extra request fields
    #[serde(default = "default_extra_params")]
    pub extra_params: Value,
    /// Language-specific configurations
    #[serde(flatten)]
    pub languages: HashMap<String, TtsLanguageConfig>,
}

impl TtsConfig {
    /// The global `[tts]` fallback as a language config, or `None` when no
    /// global provider is set.
    pub fn global_fallback(&self) -> Option<TtsLanguageConfig> {
        if self.provider.trim().is_empty() {
            return None;
        }
        Some(TtsLanguageConfig {
            enabled: true,
            provider: self.provider.trim().to_string(),
            model: self.model.clone(),
            model_id: self.model_id.clone(),
            api_model: String::new(),
            voice: self.voice.clone(),
            instructions: self.instructions.clone(),
            language_type: self.language_type.clone(),
            stream: self.stream,
            extra_params: self.extra_params.clone(),
        })
    }

    /// Config for a language: its own `[tts.<lang>]` section when present,
    /// otherwise the global `[tts]` fallback. `None` when neither exists.
    pub fn effective_language_config(&self, lang: &str) -> Option<TtsLanguageConfig> {
        if let Some(cfg) = self.languages.get(lang) {
            return Some(cfg.clone());
        }
        self.global_fallback()
    }
}

/// TTS language configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TtsLanguageConfig {
    /// Whether this language is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `local` or a provider id (catalog preset or `[providers.<id>]` entry)
    #[serde(default = "default_local_provider")]
    pub provider: String,
    /// Cloud model (wire name or catalog id)
    #[serde(default)]
    pub model: String,
    /// Local sherpa model ID (required when `provider = "local"`)
    #[serde(default)]
    pub model_id: String,
    /// Legacy alias for `model` (cloud)
    #[serde(default)]
    pub api_model: String,
    /// Voice name (checked against the model's catalog voices when known)
    #[serde(default)]
    pub voice: String,
    /// Style instructions (only sent to models that support them)
    #[serde(default)]
    pub instructions: String,
    /// DashScope `language_type` (e.g. "English"); derived from the language code if empty
    #[serde(default)]
    pub language_type: String,
    /// Use SSE streaming PCM playback (DashScope only)
    #[serde(default = "default_true")]
    pub stream: bool,
    /// Extra request fields merged into the synthesis body
    #[serde(default = "default_extra_params")]
    pub extra_params: Value,
}

impl TtsLanguageConfig {
    pub fn is_local(&self) -> bool {
        self.provider.trim().is_empty() || self.provider == LOCAL_PROVIDER
    }

    /// Local sherpa model id (`model_id`, falling back to `model`).
    pub fn local_model(&self) -> String {
        if !self.model_id.trim().is_empty() {
            self.model_id.clone()
        } else {
            self.model.clone()
        }
    }

    /// Effective cloud model: `model`, legacy `api_model`, then the
    /// dashscope default.
    pub fn cloud_model(&self) -> String {
        if !self.model.trim().is_empty() {
            return self.model.trim().to_string();
        }
        if !self.api_model.trim().is_empty() {
            return self.api_model.trim().to_string();
        }
        if self.provider == "dashscope" {
            return "qwen3-tts-flash".to_string();
        }
        String::new()
    }
}

/// Per-language STT overrides (`[stt.<lang>]`). Empty fields inherit from the
/// global `[stt]` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SttLanguageOverride {
    /// Override provider for this language.
    #[serde(default)]
    pub provider: String,
    /// Override cloud model for this language.
    #[serde(default)]
    pub model: String,
    /// Override local whisper model for this language.
    #[serde(default)]
    pub model_id: String,
    /// Override fixed language hint for this language.
    #[serde(default)]
    pub language: String,
    /// Override vocabulary hints for this language.
    #[serde(default)]
    pub hints: Vec<String>,
    /// Override extra request fields for this language.
    #[serde(default)]
    pub extra_params: Value,
}

/// STT configuration section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttConfig {
    /// `local` or a provider id (catalog preset or `[providers.<id>]` entry)
    #[serde(default = "default_local_provider")]
    pub provider: String,
    /// Cloud model (wire name or catalog id)
    #[serde(default)]
    pub model: String,
    /// Whisper model ID (required when `provider = "local"`)
    #[serde(default)]
    pub model_id: String,
    /// Legacy alias for `model` (cloud)
    #[serde(default)]
    pub api_model: String,
    /// Fixed language hint; empty = auto (per-call language / provider LID)
    #[serde(default)]
    pub language: String,
    /// Legacy alias for `language`
    #[serde(default)]
    pub api_language: String,
    /// Vocabulary hints (names, jargon) for providers that support them
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    /// Extra request fields merged into the transcription request
    #[serde(default = "default_extra_params")]
    pub extra_params: Value,
    /// Per-language overrides (`[stt.<lang>]`)
    #[serde(default, flatten)]
    pub languages: HashMap<String, SttLanguageOverride>,
}

impl SttConfig {
    pub fn is_local(&self) -> bool {
        self.provider.trim().is_empty() || self.provider == LOCAL_PROVIDER
    }

    /// Effective config for a request language: the global `[stt]` values
    /// with any `[stt.<lang>]` overrides merged in. Callers should pass a
    /// normalized language code.
    pub fn for_language(&self, lang: &str) -> SttConfig {
        let lang = normalize_language_code(lang);
        let Some(o) = self.languages.get(&lang) else {
            return self.clone();
        };
        let pick = |override_value: &str, global: &str| {
            if override_value.trim().is_empty() {
                global.to_string()
            } else {
                override_value.trim().to_string()
            }
        };
        SttConfig {
            provider: pick(&o.provider, &self.provider),
            model: pick(&o.model, &self.model),
            model_id: pick(&o.model_id, &self.model_id),
            api_model: self.api_model.clone(),
            language: pick(&o.language, &self.language),
            api_language: self.api_language.clone(),
            hints: if o.hints.is_empty() {
                self.hints.clone()
            } else {
                o.hints.clone()
            },
            extra_params: if o.extra_params.is_null() {
                self.extra_params.clone()
            } else {
                o.extra_params.clone()
            },
            languages: HashMap::new(),
        }
    }

    /// Local whisper model id (`model_id`, falling back to `model`).
    pub fn local_model(&self) -> String {
        if !self.model_id.trim().is_empty() {
            self.model_id.clone()
        } else {
            self.model.clone()
        }
    }

    /// Effective cloud model: `model`, legacy `api_model`, then the
    /// dashscope default.
    pub fn cloud_model(&self) -> String {
        if !self.model.trim().is_empty() {
            return self.model.trim().to_string();
        }
        if !self.api_model.trim().is_empty() {
            return self.api_model.trim().to_string();
        }
        if self.provider == "dashscope" {
            return "qwen3-asr-flash".to_string();
        }
        String::new()
    }

    /// Effective fixed language hint: `language`, then legacy `api_language`.
    pub fn effective_language(&self) -> String {
        if !self.language.trim().is_empty() {
            self.language.trim().to_string()
        } else {
            self.api_language.trim().to_string()
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stt_effective_model_priority() {
        let cfg = SttConfig {
            provider: "groq".into(),
            model: "whisper-large-v3-turbo".into(),
            api_model: "ignored".into(),
            ..default_stt()
        };
        assert_eq!(cfg.cloud_model(), "whisper-large-v3-turbo");

        let legacy = SttConfig {
            provider: "dashscope".into(),
            api_model: "qwen3-asr-flash".into(),
            ..default_stt()
        };
        assert_eq!(legacy.cloud_model(), "qwen3-asr-flash");

        let bare = SttConfig {
            provider: "dashscope".into(),
            ..default_stt()
        };
        assert_eq!(bare.cloud_model(), "qwen3-asr-flash");
    }

    #[test]
    fn local_vs_cloud() {
        let mut cfg = default_stt();
        assert!(cfg.is_local());
        cfg.provider = "openrouter".into();
        assert!(!cfg.is_local());
    }

    #[test]
    fn legacy_api_migrates_into_providers() {
        let toml = r#"
[api]
region = "beijing"
timeout_secs = 30
[tts]
default = "en"
[tts.en]
enabled = true
provider = "dashscope"
api_model = "qwen3-tts-flash"
voice = "Cherry"
[stt]
provider = "dashscope"
api_model = "qwen3-asr-flash"
"#;
        let mut cfg: AppConfig = toml::from_str(toml).expect("legacy parses");
        assert!(cfg.migrate_legacy());
        assert!(cfg.api.is_none());
        let ds = cfg.provider_config("dashscope").expect("migrated");
        assert_eq!(ds.region, Some(ApiRegion::Beijing));
        assert_eq!(ds.timeout_secs, 30);
        let api = cfg.dashscope_api();
        assert_eq!(api.region, ApiRegion::Beijing);
        assert_eq!(api.timeout_secs, 30);
        assert_eq!(
            api.effective_base_url(),
            "https://dashscope.aliyuncs.com/api/v1"
        );
        assert_eq!(cfg.stt.cloud_model(), "qwen3-asr-flash");
        assert_eq!(cfg.tts.languages["en"].cloud_model(), "qwen3-tts-flash");
    }

    #[test]
    fn provider_section_fields() {
        let toml = r#"
[providers.vllm]
kind = "openai"
base_url = "http://192.168.1.20:8000/v1"
requires_api_key = false
[tts]
default = "en"
[tts.en]
enabled = true
provider = "vllm"
model = "tts-1"
voice = "nova"
[stt]
provider = "vllm"
model = "whisper-large-v3"
hints = ["Filip"]
"#;
        let cfg: AppConfig = toml::from_str(toml).expect("parses");
        let vllm = cfg.provider_config("vllm").unwrap();
        assert_eq!(vllm.kind, "openai");
        assert!(!vllm.needs_key());
        assert_eq!(cfg.stt.hints, vec!["Filip".to_string()]);
        assert_eq!(cfg.tts.languages["en"].cloud_model(), "tts-1");
    }

    fn default_stt() -> SttConfig {
        toml::from_str("provider = \"local\"").unwrap()
    }

    #[test]
    fn stt_per_language_override_merges() {
        let toml = r#"
provider = "groq"
model = "whisper-large-v3-turbo"
hints = ["Filip"]

[pl]
provider = "local"
model_id = "sherpa-onnx-whisper-small"

[de]
model = "whisper-large-v3"
"#;
        let stt: SttConfig = toml::from_str(toml).expect("parses");
        assert_eq!(stt.languages.len(), 2);

        // No override: global as-is.
        let en = stt.for_language("en");
        assert_eq!(en.provider, "groq");
        assert_eq!(en.cloud_model(), "whisper-large-v3-turbo");

        // Full override to local.
        let pl = stt.for_language("pl-PL"); // normalized to "pl"
        assert!(pl.is_local());
        assert_eq!(pl.local_model(), "sherpa-onnx-whisper-small");
        assert_eq!(pl.hints, vec!["Filip".to_string()]); // inherited

        // Partial override: only model.
        let de = stt.for_language("de");
        assert_eq!(de.provider, "groq");
        assert_eq!(de.cloud_model(), "whisper-large-v3");
    }

    #[test]
    fn tts_global_fallback_and_sections() {
        let toml = r#"
default = "en"
provider = "openrouter"
model = "elevenlabs/eleven-v4-turbo"
voice = "sarah"

[en]
enabled = true
provider = "local"
model_id = "vits-piper-en_US-amy-low"
"#;
        let tts: TtsConfig = toml::from_str(toml).expect("parses");

        // Explicit section wins.
        let en = tts.effective_language_config("en").expect("en");
        assert!(en.is_local());
        assert_eq!(en.local_model(), "vits-piper-en_US-amy-low");

        // Any other language falls back to the global cloud config.
        let pl = tts.effective_language_config("pl").expect("fallback");
        assert_eq!(pl.provider, "openrouter");
        assert_eq!(pl.cloud_model(), "elevenlabs/eleven-v4-turbo");
        assert_eq!(pl.voice, "sarah");

        // No global provider → no fallback.
        let bare: TtsConfig = toml::from_str("default = \"en\"").unwrap();
        assert!(bare.effective_language_config("pl").is_none());
        assert!(bare.global_fallback().is_none());
    }
}

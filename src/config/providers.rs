//! Provider and model catalog (bundled `providers.json`).
//!
//! The catalog mirrors the provider/model presets used by the sibling apps
//! (mbdictate STT, LunaAI mobile_app TTS) so that every cloud model can be
//! selected from `config.toml` without code changes. User config can add
//! custom providers and override endpoints; unknown models for known
//! providers pass through with a warning.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// Transport family of a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// OpenAI-compatible REST API (`/audio/transcriptions`, `/audio/speech`).
    Openai,
    /// Alibaba DashScope native multimodal-generation API.
    Dashscope,
}

/// STT wire protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SttProtocol {
    /// Multipart `POST /audio/transcriptions`.
    OpenaiTranscription,
    /// DashScope multimodal-generation ASR (base64 audio input).
    DashscopeAsr,
}

/// TTS wire protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsProtocol {
    /// JSON `POST /audio/speech`.
    OpenaiSpeech,
    /// DashScope multimodal-generation TTS.
    DashscopeTts,
}

/// Which directions a provider supports.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub stt: Option<SttProtocol>,
    #[serde(default)]
    pub tts: Option<TtsProtocol>,
}

/// A catalog provider preset.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderDef {
    pub label: String,
    pub kind: ProviderKind,
    /// API base URL (OpenAI-compatible providers).
    #[serde(default)]
    pub base_url: String,
    /// Default environment variable holding the API key.
    #[serde(default)]
    pub env: String,
    /// Custom providers require endpoints/base_url from user config.
    #[serde(default)]
    pub is_custom: bool,
    #[serde(default)]
    pub capabilities: Capabilities,
}

/// A catalog STT model preset.
#[derive(Debug, Clone, Deserialize)]
pub struct SttModelDef {
    pub provider: String,
    pub id: String,
    pub label: String,
    /// Wire model name ("" for custom pass-through entries).
    pub model: String,
    #[serde(default = "default_max_audio_seconds")]
    pub max_audio_seconds: u32,
    #[serde(default)]
    pub supports_vocabulary_hint: bool,
    #[serde(default)]
    pub requires_language: bool,
}

fn default_max_audio_seconds() -> u32 {
    60
}

/// A catalog TTS model preset.
#[derive(Debug, Clone, Deserialize)]
pub struct TtsModelDef {
    pub provider: String,
    pub id: String,
    pub label: String,
    /// Wire model name ("" for custom pass-through entries).
    pub model: String,
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
    #[serde(default)]
    pub voices: Vec<String>,
    #[serde(default)]
    pub default_voice: String,
    #[serde(default)]
    pub supports_instructions: bool,
    #[serde(default = "default_response_format")]
    pub response_format: String,
}

fn default_max_chars() -> usize {
    600
}

fn default_response_format() -> String {
    "mp3".to_string()
}

/// The full bundled catalog.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderCatalog {
    #[allow(dead_code)]
    pub version: String,
    pub providers: HashMap<String, ProviderDef>,
    pub stt_models: Vec<SttModelDef>,
    pub tts_models: Vec<TtsModelDef>,
}

static CATALOG: OnceLock<ProviderCatalog> = OnceLock::new();

impl ProviderCatalog {
    /// The bundled catalog, parsed once on first use.
    pub fn bundled() -> &'static ProviderCatalog {
        CATALOG.get_or_init(|| {
            serde_json::from_str(include_str!("../providers.json"))
                .expect("bundled providers.json must be valid")
        })
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderDef> {
        self.providers.get(id)
    }

    pub fn stt_models_for(&self, provider_id: &str) -> Vec<&SttModelDef> {
        self.stt_models
            .iter()
            .filter(|m| m.provider == provider_id)
            .collect()
    }

    pub fn tts_models_for(&self, provider_id: &str) -> Vec<&TtsModelDef> {
        self.tts_models
            .iter()
            .filter(|m| m.provider == provider_id)
            .collect()
    }

    /// Find an STT model by wire model name or catalog id. Custom providers
    /// (empty `model` entries) match any requested model.
    pub fn stt_model(&self, provider_id: &str, model: &str) -> Option<&SttModelDef> {
        let models = self.stt_models_for(provider_id);
        models
            .iter()
            .find(|m| !m.model.is_empty() && (m.model == model || m.id == model))
            .or_else(|| models.iter().find(|m| m.model.is_empty()))
            .copied()
    }

    /// Find a TTS model by wire model name or catalog id. Custom providers
    /// (empty `model` entries) match any requested model.
    pub fn tts_model(&self, provider_id: &str, model: &str) -> Option<&TtsModelDef> {
        let models = self.tts_models_for(provider_id);
        models
            .iter()
            .find(|m| !m.model.is_empty() && (m.model == model || m.id == model))
            .or_else(|| models.iter().find(|m| m.model.is_empty()))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_parses() {
        let catalog = ProviderCatalog::bundled();
        assert!(catalog.provider("groq").is_some());
        assert!(catalog.provider("openrouter").is_some());
        assert!(catalog.provider("dashscope").is_some());
        assert!(catalog.provider("custom_openai").is_some());
        assert!(catalog.provider("custom_dashscope").is_some());
    }

    #[test]
    fn catalog_protocols() {
        let catalog = ProviderCatalog::bundled();
        let groq = catalog.provider("groq").unwrap();
        assert_eq!(groq.kind, ProviderKind::Openai);
        assert_eq!(groq.capabilities.stt, Some(SttProtocol::OpenaiTranscription));
        assert_eq!(groq.capabilities.tts, Some(TtsProtocol::OpenaiSpeech));
        let qwen = catalog.provider("dashscope").unwrap();
        assert_eq!(qwen.kind, ProviderKind::Dashscope);
        assert_eq!(qwen.capabilities.stt, Some(SttProtocol::DashscopeAsr));
        assert_eq!(qwen.capabilities.tts, Some(TtsProtocol::DashscopeTts));
    }

    #[test]
    fn stt_models_present() {
        let catalog = ProviderCatalog::bundled();
        let m = catalog
            .stt_model("groq", "whisper-large-v3-turbo")
            .expect("groq turbo");
        assert_eq!(m.max_audio_seconds, 300);
        assert!(m.supports_vocabulary_hint);
        let nova = catalog.stt_model("openrouter", "deepgram/nova-3").unwrap();
        assert!(nova.requires_language);
        // Custom providers pass through any model string.
        let custom = catalog.stt_model("custom_openai", "whatever/model").unwrap();
        assert_eq!(custom.id, "custom");
    }

    #[test]
    fn tts_models_present() {
        let catalog = ProviderCatalog::bundled();
        let eleven = catalog
            .tts_model("openrouter", "elevenlabs/eleven-v4-turbo")
            .expect("eleven v4 turbo");
        assert_eq!(eleven.max_chars, 600);
        assert_eq!(eleven.default_voice, "sarah");
        assert!(eleven.supports_instructions);
        assert!(eleven.voices.contains(&"sarah".to_string()));
        let orpheus = catalog.tts_model("groq", "canopylabs/orpheus-v1-english").unwrap();
        assert_eq!(orpheus.max_chars, 200);
        assert_eq!(orpheus.response_format, "wav");
        let qwen = catalog.tts_model("dashscope", "qwen3-tts-flash").unwrap();
        assert!(qwen.voices.len() > 40);
    }

    #[test]
    fn unknown_model_is_none() {
        let catalog = ProviderCatalog::bundled();
        assert!(catalog.stt_model("groq", "nope").is_none());
        assert!(catalog.tts_model("openrouter", "nope/nope").is_none());
    }
}

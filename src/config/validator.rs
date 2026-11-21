//! Configuration validator
//!
//! Validates that configuration references valid model IDs (local registry)
//! and known providers/models (cloud catalog).

use anyhow::{Context, Result};
use std::collections::HashMap;

use super::models::AppConfig;
use super::providers::ProviderCatalog;

/// Model registry loaded from models.json
#[derive(Debug, Clone)]
pub struct ModelRegistry {
    // Maps language code -> list of available models
    tts_models: HashMap<String, Vec<TtsModelInfo>>,
    stt_models: HashMap<String, SttModelInfo>,
}

#[derive(Debug, Clone)]
pub struct TtsModelInfo {
    pub id: String,
    pub download_url: String,
    pub required_files: Vec<String>,
    pub json_url: Option<String>, // Optional: for models that need JSON to generate tokens.txt (not used for official sherpa-onnx models)
}

#[derive(Debug, Clone)]
pub struct SttModelInfo {
    pub id: String,
    pub download_url: String,
    pub required_files: Vec<String>,
    pub size: String,
    pub language_code: String,
}

impl ModelRegistry {
    /// Load model registry from embedded models.json
    pub fn load() -> Result<Self> {
        let models_json = include_str!("../models.json");
        let data: serde_json::Value = serde_json::from_str(models_json)
            .context("Failed to parse models.json")?;

        let mut tts_models = HashMap::new();
        let mut stt_models = HashMap::new();

        // Load TTS models (now arrays per language)
        if let Some(tts) = data.get("tts").and_then(|v| v.as_object()) {
            for (lang_code, models_array) in tts {
                if let Some(models) = models_array.as_array() {
                    let mut lang_models = Vec::new();

                    for model_data in models {
                        if let Some(model_obj) = model_data.as_object() {
                            let id = model_obj.get("id")
                                .and_then(|v| v.as_str())
                                .context(format!("Missing 'id' for TTS model in {}", lang_code))?;
                            let download_url = model_obj.get("download_url")
                                .and_then(|v| v.as_str())
                                .context(format!("Missing 'download_url' for TTS model {} in {}", id, lang_code))?;
                            let required_files = model_obj.get("required_files")
                                .and_then(|v| v.as_array())
                                .context(format!("Missing 'required_files' for TTS model {} in {}", id, lang_code))?
                                .iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect();

                            let json_url = model_obj.get("json_url")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());

                            lang_models.push(TtsModelInfo {
                                id: id.to_string(),
                                download_url: download_url.to_string(),
                                required_files,
                                json_url,
                            });
                        }
                    }

                    if !lang_models.is_empty() {
                        tts_models.insert(lang_code.clone(), lang_models);
                    }
                }
            }
        }

        // Load STT models
        if let Some(stt) = data.get("stt_whisper").and_then(|v| v.as_object()) {
            for (model_id, model_data) in stt {
                if let Some(model_obj) = model_data.as_object() {
                    let id = model_obj.get("id")
                        .and_then(|v| v.as_str())
                        .context(format!("Missing 'id' for STT model {}", model_id))?;
                    let download_url = model_obj.get("download_url")
                        .and_then(|v| v.as_str())
                        .context(format!("Missing 'download_url' for STT model {}", model_id))?;
                    let required_files = model_obj.get("required_files")
                        .and_then(|v| v.as_array())
                        .context(format!("Missing 'required_files' for STT model {}", model_id))?
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    let size = model_obj.get("size")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let language_code = model_obj.get("language_code")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    stt_models.insert(model_id.clone(), SttModelInfo {
                        id: id.to_string(),
                        download_url: download_url.to_string(),
                        required_files,
                        size,
                        language_code,
                    });
                }
            }
        }

        Ok(Self {
            tts_models,
            stt_models,
        })
    }

    /// Get TTS model info by language code and model ID
    pub fn get_tts_model(&self, lang_code: &str, model_id: &str) -> Option<&TtsModelInfo> {
        self.tts_models
            .get(lang_code)?
            .iter()
            .find(|m| m.id == model_id)
    }

    /// Get all TTS models for a language
    pub fn get_tts_models_for_language(&self, lang_code: &str) -> Option<&Vec<TtsModelInfo>> {
        self.tts_models.get(lang_code)
    }

    /// Check if a TTS model ID exists for any language
    pub fn is_valid_tts_model_id(&self, model_id: &str) -> bool {
        self.tts_models.values().any(|models| {
            models.iter().any(|m| m.id == model_id)
        })
    }

    /// Get STT model info by model ID
    pub fn get_stt_model(&self, model_id: &str) -> Option<&SttModelInfo> {
        self.stt_models.get(model_id)
    }


    /// Check if STT model ID is valid
    pub fn is_valid_stt_model_id(&self, model_id: &str) -> bool {
        self.stt_models.contains_key(model_id)
    }
}

/// Which direction of a provider is being validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Capability {
    Stt,
    Tts,
}

fn endpoint_for(pc: &crate::config::ProviderConfig, cap: Capability) -> &str {
    match cap {
        Capability::Stt => &pc.stt_endpoint,
        Capability::Tts => &pc.tts_endpoint,
    }
}

/// Configuration validator
pub struct ConfigValidator;

impl ConfigValidator {
    /// Validate configuration against the local model registry and the
    /// bundled cloud provider catalog.
    pub fn validate(config: &AppConfig) -> Result<()> {
        let registry = ModelRegistry::load()
            .context("Failed to load model registry")?;

        Self::validate_tts(config, &registry)?;
        Self::validate_stt(config, &registry)?;
        Ok(())
    }

    fn validate_tts(config: &AppConfig, registry: &ModelRegistry) -> Result<()> {
        // The global [tts] fallback must be a cloud provider (local models
        // are language-specific and need [tts.<lang>] sections).
        if let Some(fallback) = config.tts.global_fallback() {
            if fallback.is_local() {
                anyhow::bail!(
                    "The global [tts] fallback requires a cloud provider; \
                     local models are language-specific — use [tts.<lang>] sections"
                );
            }
            Self::validate_tts_cloud(config, &fallback, "tts fallback")?;
        }

        // The default language must resolve to a section or the fallback.
        if config.tts.effective_language_config(&config.tts.default).is_none() {
            let supported: Vec<&str> = registry.tts_models.keys().map(|s| s.as_str()).collect();
            anyhow::bail!(
                "TTS default language '{}' has no [tts.{}] section and no global [tts] \
                 fallback is configured. Local model languages: {}",
                config.tts.default,
                config.tts.default,
                supported.join(", ")
            );
        }

        // Validate enabled per-language sections.
        for (lang_code, lang_config) in &config.tts.languages {
            if !lang_config.enabled {
                continue;
            }
            if lang_config.is_local() {
                Self::validate_tts_local(lang_code, lang_config, registry)?;
            } else {
                Self::validate_tts_cloud(config, lang_config, &format!("tts.{}", lang_code))?;
            }
        }
        Ok(())
    }

    fn validate_tts_local(
        lang_code: &str,
        lang_config: &crate::config::TtsLanguageConfig,
        registry: &ModelRegistry,
    ) -> Result<()> {
        if !registry.tts_models.contains_key(lang_code) {
            let supported: Vec<&str> = registry.tts_models.keys().map(|s| s.as_str()).collect();
            anyhow::bail!(
                "Invalid TTS language code: '{}'. Supported languages: {}",
                lang_code,
                supported.join(", ")
            );
        }
        let model_id = lang_config.local_model();
        if registry.get_tts_model(lang_code, &model_id).is_none() {
            let available_models: Vec<String> = registry
                .get_tts_models_for_language(lang_code)
                .map(|models| models.iter().map(|m| m.id.clone()).collect())
                .unwrap_or_default();
            anyhow::bail!(
                "Invalid TTS model ID '{}' for language '{}'. Available models: {}",
                model_id,
                lang_code,
                available_models.join(", ")
            );
        }
        Ok(())
    }

    fn validate_tts_cloud(
        config: &AppConfig,
        lang_config: &crate::config::TtsLanguageConfig,
        ctx: &str,
    ) -> Result<()> {
        let catalog = ProviderCatalog::bundled();
        Self::validate_cloud_provider(config, &lang_config.provider, Capability::Tts)
            .map_err(|e| anyhow::anyhow!("[{}] {}", ctx, e))?;
        let model = lang_config.cloud_model();
        if model.is_empty() {
            anyhow::bail!(
                "[{}] uses provider '{}' but has no model set",
                ctx,
                lang_config.provider
            );
        }
        match catalog.tts_model(&lang_config.provider, &model) {
            Some(def) => {
                let voice = lang_config.voice.trim();
                if !voice.is_empty()
                    && !def.voices.is_empty()
                    && !def.voices.iter().any(|v| v.eq_ignore_ascii_case(voice))
                {
                    tracing::warn!(
                        "[{}] TTS voice '{}' is not in the catalog list for '{}'; \
                         passing it through verbatim",
                        ctx,
                        voice,
                        model
                    );
                }
                if !lang_config.instructions.trim().is_empty() && !def.supports_instructions {
                    tracing::warn!(
                        "[{}] TTS model '{}' does not advertise instruction support; \
                         instructions will still be sent",
                        ctx,
                        model
                    );
                }
            }
            None => tracing::warn!(
                "[{}] TTS model '{}' for provider '{}' is not in the bundled catalog; \
                 passing it through",
                ctx,
                model,
                lang_config.provider
            ),
        }
        Ok(())
    }

    fn validate_stt(config: &AppConfig, registry: &ModelRegistry) -> Result<()> {
        // Global [stt] section.
        Self::validate_stt_config(config, &config.stt, registry, "stt")?;
        // Per-language overrides, merged with the global section.
        let mut langs: Vec<&String> = config.stt.languages.keys().collect();
        langs.sort();
        for lang in langs {
            let effective = config.stt.for_language(lang);
            Self::validate_stt_config(config, &effective, registry, &format!("stt.{}", lang))?;
        }
        Ok(())
    }

    fn validate_stt_config(
        config: &AppConfig,
        stt: &crate::config::SttConfig,
        registry: &ModelRegistry,
        ctx: &str,
    ) -> Result<()> {
        let catalog = ProviderCatalog::bundled();
        if !stt.is_local() {
            Self::validate_cloud_provider(config, &stt.provider, Capability::Stt)
                .map_err(|e| anyhow::anyhow!("[{}] {}", ctx, e))?;
            let model = stt.cloud_model();
            if model.is_empty() {
                anyhow::bail!("[{}] uses provider '{}' but has no model set", ctx, stt.provider);
            }
            if catalog.stt_model(&stt.provider, &model).is_none() {
                tracing::warn!(
                    "[{}] STT model '{}' for provider '{}' is not in the bundled catalog; \
                     passing it through",
                    ctx,
                    model,
                    stt.provider
                );
            }
        } else if !registry.is_valid_stt_model_id(&stt.local_model()) {
            let supported: Vec<&str> = registry.stt_models.keys().map(|s| s.as_str()).collect();
            anyhow::bail!(
                "[{}] invalid STT model ID: '{}'. Supported models: {}",
                ctx,
                stt.local_model(),
                supported.join(", ")
            );
        }
        Ok(())
    }

    /// Ensure a cloud provider id resolves: a catalog preset (with required
    /// user config for custom presets) or a user-defined `[providers.<id>]`
    /// section with a valid kind.
    fn validate_cloud_provider(config: &AppConfig, id: &str, cap: Capability) -> Result<()> {
        let catalog = ProviderCatalog::bundled();
        if let Some(def) = catalog.provider(id) {
            let supports = match cap {
                Capability::Stt => def.capabilities.stt.is_some(),
                Capability::Tts => def.capabilities.tts.is_some(),
            };
            if !supports {
                anyhow::bail!(
                    "Provider '{}' does not support {:?} in the bundled catalog",
                    id,
                    cap
                );
            }
            if def.is_custom {
                let pc = config.provider_config(id).with_context(|| {
                    format!(
                        "Provider '{}' is a custom preset: add a [providers.{}] section \
                         with base_url or endpoint overrides",
                        id, id
                    )
                })?;
                Self::check_endpoints(&pc.base_url, endpoint_for(pc, cap), id, cap)?;
            }
            return Ok(());
        }

        let pc = config.provider_config(id).with_context(|| {
            format!(
                "Unknown provider '{}': not in the bundled catalog and no [providers.{}] \
                 section in config.toml",
                id, id
            )
        })?;
        match pc.kind.as_str() {
            "openai" | "dashscope" => {}
            other => anyhow::bail!(
                "Provider '{}' has kind '{}'; expected 'openai' or 'dashscope'",
                id,
                other
            ),
        }
        Self::check_endpoints(&pc.base_url, endpoint_for(pc, cap), id, cap)
    }

    fn check_endpoints(base_url: &str, endpoint: &str, id: &str, cap: Capability) -> Result<()> {
        if base_url.trim().is_empty() && endpoint.trim().is_empty() {
            anyhow::bail!(
                "Provider '{}' needs a base_url or {:?} endpoint in its [providers.{}] section",
                id,
                cap,
                id
            );
        }
        Ok(())
    }

    /// Get model registry (for use in model downloader)
    pub fn get_registry() -> Result<ModelRegistry> {
        ModelRegistry::load()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::loader::ConfigLoader;
    use crate::config::models::{ApiRegion, AppConfig};

    fn parse(toml_str: &str) -> AppConfig {
        ConfigLoader::parse_and_migrate(toml_str).expect("config parses")
    }

    #[test]
    fn default_template_parses() {
        let cfg = parse(include_str!("../../config.toml.default"));
        assert_eq!(cfg.tts.default, "en");
        assert!(cfg.tts.languages["en"].is_local());
        assert!(cfg.stt.is_local());
        ConfigValidator::validate(&cfg).expect("default template validates");
    }

    #[test]
    fn legacy_config_defaults_to_local() {
        let toml = r#"
[tts]
default = "en"
[tts.en]
enabled = true
model_id = "vits-piper-en_US-amy-low"
[stt]
model_id = "sherpa-onnx-whisper-tiny"
"#;
        let cfg = parse(toml);
        assert!(cfg.tts.languages["en"].is_local());
        assert!(cfg.stt.is_local());
        assert_eq!(
            cfg.tts.languages["en"].local_model(),
            "vits-piper-en_US-amy-low"
        );
        ConfigValidator::validate(&cfg).expect("legacy config validates");
    }

    #[test]
    fn legacy_api_section_migrates_and_validates() {
        let toml = r#"
[api]
region = "singapore"
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
        let cfg = parse(toml);
        assert!(cfg.api.is_none());
        assert_eq!(
            cfg.provider_config("dashscope").and_then(|p| p.region),
            Some(ApiRegion::Singapore)
        );
        ConfigValidator::validate(&cfg).expect("migrated cloud config validates");
    }

    #[test]
    fn catalog_providers_validate() {
        let toml = r#"
[tts]
default = "en"
[tts.en]
enabled = true
provider = "openrouter"
model = "elevenlabs/eleven-v4-turbo"
voice = "sarah"
[stt]
provider = "groq"
model = "whisper-large-v3-turbo"
hints = ["Filip"]
"#;
        let cfg = parse(toml);
        ConfigValidator::validate(&cfg).expect("catalog providers validate");
    }

    #[test]
    fn unknown_provider_fails() {
        let toml = r#"
[tts]
default = "en"
[tts.en]
enabled = true
model_id = "vits-piper-en_US-amy-low"
[stt]
provider = "acme"
model = "whatever"
"#;
        let cfg = parse(toml);
        let err = ConfigValidator::validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("Unknown provider 'acme'"), "got: {}", err);
    }

    #[test]
    fn custom_provider_requires_endpoint() {
        let toml = r#"
[tts]
default = "en"
[tts.en]
enabled = true
model_id = "vits-piper-en_US-amy-low"
[stt]
provider = "custom_openai"
model = "whisper-large-v3"
"#;
        let cfg = parse(toml);
        let err = ConfigValidator::validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("custom preset"), "got: {}", err);

        let with_endpoint = r#"
[providers.custom_openai]
base_url = "http://localhost:8000/v1"
[tts]
default = "en"
[tts.en]
enabled = true
model_id = "vits-piper-en_US-amy-low"
[stt]
provider = "custom_openai"
model = "whisper-large-v3"
"#;
        let cfg = parse(with_endpoint);
        ConfigValidator::validate(&cfg).expect("custom provider with base_url validates");
    }

    #[test]
    fn user_defined_provider_validates() {
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
"#;
        let cfg = parse(toml);
        ConfigValidator::validate(&cfg).expect("user-defined provider validates");
    }

    #[test]
    fn cloud_model_required() {
        let toml = r#"
[tts]
default = "en"
[tts.en]
enabled = true
model_id = "vits-piper-en_US-amy-low"
[stt]
provider = "groq"
"#;
        let cfg = parse(toml);
        let err = ConfigValidator::validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("no model set"), "got: {}", err);
    }

    #[test]
    fn api_region_presets() {
        let mut api = crate::config::models::ApiConfig::default();
        assert_eq!(
            api.effective_base_url(),
            "https://dashscope-intl.aliyuncs.com/api/v1"
        );
        api.region = ApiRegion::Beijing;
        assert_eq!(
            api.effective_base_url(),
            "https://dashscope.aliyuncs.com/api/v1"
        );
        api.region = ApiRegion::Custom;
        api.base_url = "https://ws.ap-southeast-1.maas.aliyuncs.com/api/v1".into();
        assert_eq!(
            api.effective_base_url(),
            "https://ws.ap-southeast-1.maas.aliyuncs.com/api/v1"
        );
    }

    #[test]
    fn cloud_only_config_without_language_sections() {
        // Language is optional: a global [tts] cloud fallback speaks any
        // language, and [stt] auto-detects.
        let toml = r#"
[tts]
default = "ko"
provider = "dashscope"
model = "qwen3-tts-flash"
voice = "Cherry"
[stt]
provider = "groq"
model = "whisper-large-v3"
"#;
        let cfg = parse(toml);
        ConfigValidator::validate(&cfg).expect("cloud-only config validates");
        assert!(cfg.tts.effective_language_config("ko").is_some());
        assert!(cfg.tts.effective_language_config("pl").is_some());
    }

    #[test]
    fn global_tts_fallback_must_be_cloud() {
        let toml = r#"
[tts]
default = "en"
provider = "local"
model_id = "vits-piper-en_US-amy-low"
[stt]
model_id = "sherpa-onnx-whisper-tiny"
"#;
        let cfg = parse(toml);
        let err = ConfigValidator::validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("global [tts] fallback requires a cloud provider"), "got: {}", err);
    }

    #[test]
    fn default_language_must_resolve() {
        let toml = r#"
[tts]
default = "en"
[tts.pl]
enabled = true
model_id = "vits-piper-pl_PL-gosia-medium"
[stt]
model_id = "sherpa-onnx-whisper-tiny"
"#;
        let cfg = parse(toml);
        let err = ConfigValidator::validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("no [tts.en] section"), "got: {}", err);
    }

    #[test]
    fn stt_per_language_override_validates() {
        let toml = r#"
[tts]
default = "en"
[tts.en]
enabled = true
model_id = "vits-piper-en_US-amy-low"
[stt]
provider = "groq"
model = "whisper-large-v3-turbo"
[stt.pl]
provider = "local"
model_id = "sherpa-onnx-whisper-small"
"#;
        let cfg = parse(toml);
        ConfigValidator::validate(&cfg).expect("mixed global cloud + local override validates");

        let bad = r#"
[tts]
default = "en"
[tts.en]
enabled = true
model_id = "vits-piper-en_US-amy-low"
[stt]
provider = "groq"
model = "whisper-large-v3-turbo"
[stt.pl]
provider = "nope"
model = "x"
"#;
        let cfg = parse(bad);
        let err = ConfigValidator::validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("stt.pl"), "error should mention the section: {}", err);
    }
}

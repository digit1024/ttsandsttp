//! Configuration validator
//!
//! Validates that configuration references valid model IDs

use anyhow::{Context, Result};
use std::collections::HashMap;

use super::models::AppConfig;

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

/// Configuration validator
pub struct ConfigValidator;

impl ConfigValidator {
    /// Validate configuration against model registry
    pub fn validate(config: &AppConfig) -> Result<()> {
        let registry = ModelRegistry::load()
            .context("Failed to load model registry")?;

        // Validate TTS default language exists
        if !registry.tts_models.contains_key(&config.tts.default) {
            let supported: Vec<&str> = registry.tts_models.keys().map(|s| s.as_str()).collect();
            anyhow::bail!(
                "Invalid TTS default language: '{}'. Supported languages: {}",
                config.tts.default,
                supported.join(", ")
            );
        }

        // Validate enabled TTS languages
        for (lang_code, lang_config) in &config.tts.languages {
            if lang_config.enabled {
                // Check if language code is valid
                if !registry.tts_models.contains_key(lang_code) {
                    let supported: Vec<&str> = registry.tts_models.keys().map(|s| s.as_str()).collect();
                    anyhow::bail!(
                        "Invalid TTS language code: '{}'. Supported languages: {}",
                        lang_code,
                        supported.join(", ")
                    );
                }

                // Check if model ID is valid for this language
                if registry.get_tts_model(lang_code, &lang_config.model_id).is_none() {
                    let available_models: Vec<String> = registry
                        .get_tts_models_for_language(lang_code)
                        .map(|models| models.iter().map(|m| m.id.clone()).collect())
                        .unwrap_or_default();
                    anyhow::bail!(
                        "Invalid TTS model ID '{}' for language '{}'. Available models: {}",
                        lang_config.model_id,
                        lang_code,
                        available_models.join(", ")
                    );
                }
            }
        }

        // Validate STT model
        if !registry.is_valid_stt_model_id(&config.stt.model_id) {
            let supported: Vec<&str> = registry.stt_models.keys().map(|s| s.as_str()).collect();
            anyhow::bail!(
                "Invalid STT model ID: '{}'. Supported models: {}",
                config.stt.model_id,
                supported.join(", ")
            );
        }

        Ok(())
    }

    /// Get model registry (for use in model downloader)
    pub fn get_registry() -> Result<ModelRegistry> {
        ModelRegistry::load()
    }
}


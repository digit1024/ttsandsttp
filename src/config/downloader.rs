//! Model downloader based on configuration
//!
//! Uses ModelManager to download models specified in the configuration file

use anyhow::{Context, Result};
use tracing;

use super::models::AppConfig;
use super::validator::ConfigValidator;
use crate::services::ModelManager;

/// Model downloader that uses configuration
pub struct ConfigModelDownloader;

impl ConfigModelDownloader {
    /// Download all models required by the configuration
    pub async fn download_required_models(config: &AppConfig) -> Result<()> {
        let registry = ConfigValidator::get_registry()
            .context("Failed to load model registry")?;

        let model_manager = ModelManager::new()?;

        // Download enabled TTS models
        for (lang_code, lang_config) in &config.tts.languages {
            if lang_config.enabled {
                if let Some(model_info) = registry.get_tts_model(lang_code, &lang_config.model_id) {
                    tracing::info!("Downloading TTS model for {}: {}", lang_code, model_info.id);
                    
                    let models_dir = model_manager.models_dir();
                    let model_path = models_dir.join("tts").join(lang_code);
                    
                    model_manager
                        .download_model_by_url(
                            &model_info.download_url,
                            &model_info.id,
                            &model_path,
                            &model_info.required_files,
                        )
                        .await
                        .with_context(|| format!("Failed to download TTS model for {}", lang_code))?;
                    
                    // Try to add missing sample_rate metadata for Polish models (optional, non-fatal)
                    if lang_code == "pl" {
                        if let Some(onnx_file) = model_info.required_files.iter().find(|f| f.ends_with(".onnx")) {
                            let onnx_path = model_manager
                                .find_actual_model_path(&model_path, &model_info.id)
                                .join(onnx_file);
                            
                            if onnx_path.exists() {
                                // Try to add metadata (non-fatal if it fails)
                                if let Err(e) = crate::utils::onnx_metadata::add_sample_rate_metadata(&onnx_path, 22050) {
                                    tracing::warn!("Could not add sample_rate metadata (warning is non-fatal): {}", e);
                                }
                            }
                        }
                    }
                    
                    tracing::info!("TTS model for {} ready", lang_code);
                }
            }
        }

        // Download STT model
        if let Some(model_info) = registry.get_stt_model(&config.stt.model_id) {
            tracing::info!("Downloading STT model: {}", model_info.id);
            
            let models_dir = model_manager.models_dir();
            let model_path = models_dir.join("whisper").join(&model_info.id);
            
            model_manager
                .download_model_by_url(
                    &model_info.download_url,
                    &model_info.id,
                    &model_path,
                    &model_info.required_files,
                )
                .await
                .with_context(|| format!("Failed to download STT model: {}", model_info.id))?;
            
            tracing::info!("STT model {} ready", model_info.id);
        }

        Ok(())
    }

}


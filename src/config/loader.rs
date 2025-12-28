//! Configuration loader
//!
//! Loads configuration from ~/.config/ttsandsttp/config.toml
//! Creates default config if it doesn't exist

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use dirs;

use super::models::AppConfig;

/// Configuration loader
pub struct ConfigLoader;

impl ConfigLoader {
    /// Get the config file path
    pub fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Could not determine config directory")?;
        let config_path = config_dir.join("ttsandsttp").join("config.toml");
        Ok(config_path)
    }

    /// Get the config directory
    pub fn config_dir() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Could not determine config directory")?;
        Ok(config_dir.join("ttsandsttp"))
    }

    /// Load configuration, creating default if it doesn't exist
    pub fn load_or_create() -> Result<AppConfig> {
        let config_path = Self::config_path()?;

        if !config_path.exists() {
            eprintln!("📝 Creating default configuration at: {}", config_path.display());
            Self::create_default_config(&config_path)?;
        }

        Self::load_config(&config_path)
    }

    /// Load configuration from file
    pub fn load_config(config_path: &PathBuf) -> Result<AppConfig> {
        let content = fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

        let config: AppConfig = toml::from_str(&content)
            .context("Failed to parse config file")?;

        Ok(config)
    }

    /// Create default configuration file
    fn create_default_config(config_path: &PathBuf) -> Result<()> {
        // Create config directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
        }

        // Read default config from embedded resource or file
        let default_config = include_str!("../../config.toml.default");

        // Write default config
        fs::write(config_path, default_config)
            .with_context(|| format!("Failed to write default config: {}", config_path.display()))?;

        Ok(())
    }
}



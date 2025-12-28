//! Shared configuration and registry management
//!
//! Provides a shared, thread-safe access to configuration and model registry
//! to avoid duplication across services.

use anyhow::{Context, Result};
use std::sync::Arc;

use super::{AppConfig, ConfigLoader, ConfigValidator, ModelRegistry};

/// Shared configuration and registry manager
///
/// This struct holds shared references to the application configuration
/// and model registry, avoiding duplication across multiple services.
#[derive(Clone)]
pub struct SharedConfig {
    config: Arc<AppConfig>,
    registry: Arc<ModelRegistry>,
}

impl SharedConfig {
    /// Load configuration and registry, creating a shared instance
    pub fn load() -> Result<Self> {
        let config = ConfigLoader::load_or_create()
            .context("Failed to load config")?;
        let registry = ConfigValidator::get_registry()
            .context("Failed to load model registry")?;
        
        Ok(Self {
            config: Arc::new(config),
            registry: Arc::new(registry),
        })
    }
    
    /// Get a reference to the configuration
    pub fn config(&self) -> &AppConfig {
        &self.config
    }
    
    /// Get a reference to the model registry
    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }
    
    /// Get an Arc reference to the configuration
    pub fn config_arc(&self) -> Arc<AppConfig> {
        Arc::clone(&self.config)
    }
    
    /// Get an Arc reference to the model registry
    pub fn registry_arc(&self) -> Arc<ModelRegistry> {
        Arc::clone(&self.registry)
    }
}


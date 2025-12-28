use anyhow::{Result, Context};
use ttsandsttp::{TtsSttService, ConfigLoader, ConfigValidator, ConfigModelDownloader};
use tracing_subscriber;

/// TTSandSTTP Daemon - DBus service for Text-to-Speech and Speech-to-Text
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    
    tracing::info!("Starting DBus daemon service...");
    
    // Load and validate configuration
    tracing::info!("Loading configuration...");
    let config = ConfigLoader::load_or_create()
        .context("Failed to load or create configuration")?;
    
    tracing::info!("Configuration loaded from: {}", ConfigLoader::config_path()?.display());
    
    // Validate configuration
    tracing::info!("Validating configuration...");
    ConfigValidator::validate(&config)
        .context("Configuration validation failed")?;
    tracing::info!("Configuration is valid");
    
    // Download required models
    tracing::info!("Checking and downloading required models...");
    ConfigModelDownloader::download_required_models(&config).await
        .context("Failed to download required models")?;
    tracing::info!("All required models are ready");
    
    let service = TtsSttService::new()?;
    
    // Preload models
    tracing::info!("Preloading models...");
    service.preload_models().await?;
    
    // Start serving DBus requests
    service.serve().await?;

    Ok(())
}

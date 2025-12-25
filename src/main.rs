use anyhow::{Result, Context};
use ttsandsttp::{TtsSttService, ConfigLoader, ConfigValidator, ConfigModelDownloader};

/// TTSandSTTP Daemon - DBus service for Text-to-Speech and Speech-to-Text
#[tokio::main]
async fn main() -> Result<()> {
    println!("🚀 Starting DBus daemon service...");
    
    // Load and validate configuration
    println!("📋 Loading configuration...");
    let config = ConfigLoader::load_or_create()
        .context("Failed to load or create configuration")?;
    
    println!("✅ Configuration loaded from: {}", ConfigLoader::config_path()?.display());
    
    // Validate configuration
    println!("🔍 Validating configuration...");
    ConfigValidator::validate(&config)
        .context("Configuration validation failed")?;
    println!("✅ Configuration is valid");
    
    // Download required models
    println!("📥 Checking and downloading required models...");
    ConfigModelDownloader::download_required_models(&config).await
        .context("Failed to download required models")?;
    println!("✅ All required models are ready");
    
    let service = TtsSttService::new()?;
    
    // Preload models
    println!("📦 Preloading models...");
    service.preload_models().await?;
    
    // Start serving DBus requests
    service.serve().await?;

    Ok(())
}

//! Configuration management module
//!
//! Handles loading, validating, and managing user configuration.
//! Configuration is stored in ~/.config/ttsandsttp/config.toml

mod loader;
mod validator;
mod models;
mod downloader;
mod shared;

pub use loader::ConfigLoader;
pub use validator::{ConfigValidator, ModelRegistry};
pub use models::{AppConfig, TtsLanguageConfig, SttConfig};
pub use downloader::ConfigModelDownloader;
pub use shared::SharedConfig;


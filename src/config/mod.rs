//! Configuration management module
//!
//! Handles loading, validating, and managing user configuration.
//! Configuration is stored in ~/.config/ttsandsttp/config.toml

mod loader;
mod validator;
mod models;
mod downloader;
mod shared;
mod voices;

pub use loader::ConfigLoader;
pub use validator::{ConfigValidator, ModelRegistry};
pub use models::{
    language_type_for_code, ApiConfig, ApiRegion, AppConfig, KeySource, Provider, SttConfig,
    TtsLanguageConfig,
};
pub use downloader::ConfigModelDownloader;
pub use shared::SharedConfig;
pub use voices::{is_known_voice, load_voices, Voice};


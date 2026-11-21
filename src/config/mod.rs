//! Configuration management module
//!
//! Handles loading, validating, and managing user configuration.
//! Configuration is stored in ~/.config/ttsandsttp/config.toml

mod loader;
mod validator;
mod models;
mod downloader;
mod providers;
mod shared;
mod voices;

pub use loader::ConfigLoader;
pub use validator::{ConfigValidator, ModelRegistry};
pub use models::{
    language_type_for_code, ApiConfig, ApiRegion, AppConfig, KeySource, ProviderConfig,
    SttConfig, TtsLanguageConfig, LOCAL_PROVIDER,
};
pub use providers::{
    Capabilities, ProviderCatalog, ProviderDef, ProviderKind, SttModelDef, SttProtocol,
    TtsModelDef, TtsProtocol,
};
pub use downloader::ConfigModelDownloader;
pub use shared::SharedConfig;
pub use voices::{is_known_voice, load_voices, Voice};

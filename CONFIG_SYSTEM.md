# Configuration System

## Overview

The application now uses a configuration file-based system for managing TTS and STT models. Configuration is stored at `~/.config/ttsandsttp/config.toml`.

## Files Created

### 1. `config.toml.default`
Default configuration template that is copied to user's config directory on first run.

**Location**: Project root (template)
**User Location**: `~/.config/ttsandsttp/config.toml`

### 2. `src/models.json`
Simplified model registry containing only essential information:
- Model IDs
- Download URLs
- Required files

**Contains**:
- 10 TTS models (one per supported language)
- 7 STT/Whisper models (various sizes)

### 3. Config Module (`src/config/`)
- `mod.rs` - Module exports
- `models.rs` - Configuration data structures
- `loader.rs` - Loads and creates default config
- `validator.rs` - Validates config against model registry
- `downloader.rs` - Downloads models based on config

## Configuration Structure

### TTS Section

```toml
[tts]
default = "en"  # Default language

[tts.en]
enabled = true
model_id = "vits-piper-en_US-amy-low"

# Other languages are commented by default
# [tts.es]
# enabled = false
# model_id = "vits-piper-es_ES-shared-medium"
```

### STT Section

```toml
[stt]
model_id = "sherpa-onnx-whisper-tiny"  # Whisper model to use
```

## Supported Languages

- ✅ English (en) - **Enabled by default**
- ✅ Spanish (es)
- ✅ French (fr)
- ✅ German (de)
- ✅ Italian (it)
- ✅ Portuguese (pt)
- ✅ Chinese (zh)
- ✅ Japanese (ja)
- ✅ Russian (ru)
- ✅ Polish (pl)

## Usage

### Loading Configuration

```rust
use ttsandsttp::ConfigLoader;

// Load or create default config
let config = ConfigLoader::load_or_create()?;
```

### Validating Configuration

```rust
use ttsandsttp::ConfigValidator;

// Validate config against model registry
ConfigValidator::validate(&config)?;
```

### Downloading Models

```rust
use ttsandsttp::ConfigModelDownloader;

// Download all required models
ConfigModelDownloader::download_required_models(&config).await?;
```

## Model Registry

The model registry (`src/models.json`) is embedded in the binary and used for:
- Validating model IDs in config
- Getting download URLs
- Checking required files

## Integration Points

### Startup Flow

1. **Load Config**: `ConfigLoader::load_or_create()`
2. **Validate Config**: `ConfigValidator::validate(&config)`
3. **Download Models**: `ConfigModelDownloader::download_required_models(&config)`
4. **Initialize Services**: Use config to select models

### Model Manager Integration

The existing `ModelManager` can be used alongside the config system. The config downloader uses `ModelManager` for directory management.

## Example: Full Startup

```rust
use ttsandsttp::{ConfigLoader, ConfigValidator, ConfigModelDownloader};

// 1. Load config
let config = ConfigLoader::load_or_create()?;

// 2. Validate
ConfigValidator::validate(&config)?;

// 3. Download required models
ConfigModelDownloader::download_required_models(&config).await?;

// 4. Use config to initialize services
let default_lang = &config.tts.default;
let stt_model = &config.stt.model_id;
```

## Next Steps

To fully integrate this into the daemon:

1. Update `daemon/service.rs` to use config on startup
2. Update `services/tts/service.rs` to use config for model selection
3. Update `services/stt/service.rs` to use config for Whisper model
4. Remove hardcoded model references from `domain/models.rs`

## Notes

- Config is created automatically on first run
- Only enabled TTS languages are downloaded
- STT model is always downloaded (as specified in config)
- Model validation happens before download
- All models are stored in `~/.local/share/stttts/`


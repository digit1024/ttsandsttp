# Language & Model Support Analysis

## 📊 Current Capabilities

### ✅ What Works Now

**TTS (Text-to-Speech):**
- ✅ **Language**: English (US) only
- ✅ **Model**: `vits-piper-en_US-amy-low` (hardcoded)
- ✅ **Voice**: Single voice (Amy, low quality)
- ✅ **Location**: Downloaded from GitHub releases automatically

**STT (Speech-to-Text):**
- ✅ **Language**: English only
- ✅ **Model**: `sherpa-onnx-whisper-tiny.en` (hardcoded)
- ✅ **Size**: Tiny model (smallest, fastest)
- ✅ **Location**: Downloaded from GitHub releases automatically

**VAD (Voice Activity Detection):**
- ✅ **Model**: `silero_vad` (language-agnostic)
- ✅ **Status**: Optional (works without it)

### ❌ Current Limitations

1. **Hardcoded Language**: All models are English-only
2. **Hardcoded Model Selection**: No way to choose different models
3. **No Language Detection**: Can't auto-detect language
4. **Single Model Per Type**: Can't switch between model sizes/qualities
5. **No Multi-Language Support**: Can't use multiple languages in one session

---

## 🏗️ Architecture Analysis

### Current Model Selection Flow

```
User Request (language: "en-US")
    ↓
Service (TTS/STT)
    ↓
ModelType enum (hardcoded to English models)
    ↓
ModelManager.download_model() (uses hardcoded URL)
    ↓
Service initializes with hardcoded file names
```

### Key Files & Hardcoded Values

**1. `src/domain/models.rs`** - Model Configuration
```rust
// Hardcoded model names
ModelType::Tts => "vits-piper-en_US-amy-low"
ModelType::Whisper => "sherpa-onnx-whisper-tiny.en"

// Hardcoded URLs
ModelType::Tts => "https://.../vits-piper-en_US-amy-low.tar.bz2"
ModelType::Whisper => "https://.../sherpa-onnx-whisper-tiny.en.tar.bz2"

// Hardcoded file names
ModelType::Tts => vec!["en_US-amy-low.onnx", "tokens.txt"]
ModelType::Whisper => vec!["tiny.en-encoder.int8.onnx", ...]
```

**2. `src/services/stt/service.rs`** - STT Initialization
```rust
// Line 200: Hardcoded language
language: "en".to_string(),

// Lines 182-190: Hardcoded file names
"tiny.en-encoder.int8.onnx"
"tiny.en-decoder.int8.onnx"
"tiny.en-tokens.txt"
```

**3. `src/services/tts/service.rs`** - TTS Initialization
```rust
// Line 122: Hardcoded model file
"en_US-amy-low.onnx"

// Line 55: Hardcoded default language
current_language: "en-US".to_string()
```

---

## 🎯 Required Changes for Multi-Language Support

### Phase 1: Model Configuration System

**Goal**: Make model selection language-aware and configurable

#### 1.1 Create Language-Aware Model Configuration

**New File**: `src/domain/model_config.rs`

```rust
#[derive(Clone, Debug)]
pub struct ModelConfig {
    pub language: String,           // e.g., "en", "es", "fr", "zh"
    pub model_name: String,         // e.g., "tiny.en", "base", "small"
    pub model_url: String,          // Download URL
    pub required_files: Vec<String>, // File names to check
}

pub struct ModelRegistry {
    // Map: (ModelType, Language) -> ModelConfig
    models: HashMap<(ModelType, String), ModelConfig>,
}

impl ModelRegistry {
    pub fn get_config(&self, model_type: ModelType, language: &str) -> Option<&ModelConfig>;
    pub fn list_languages(&self, model_type: ModelType) -> Vec<String>;
    pub fn list_models(&self, model_type: ModelType, language: &str) -> Vec<String>;
}
```

#### 1.2 Update ModelType Enum

**File**: `src/domain/models.rs`

**Changes**:
- Remove hardcoded `default_url()`, `default_model_name()`, `required_files()`
- Add method: `get_config_for_language(language: &str) -> Option<ModelConfig>`
- Keep enum for type identification only

#### 1.3 Update ModelManager

**File**: `src/services/models/manager.rs`

**Changes**:
- Accept `ModelConfig` instead of `ModelType` for downloads
- Support language-specific model directories: `{models_dir}/{type}/{language}/`
- Update `ensure_models_present()` to accept language parameter

**New Structure**:
```
~/.local/share/stttts/
├── tts/
│   ├── en/
│   │   └── vits-piper-en_US-amy-low/
│   ├── es/
│   │   └── vits-piper-es_ES-shared-medium/
│   └── fr/
│       └── vits-piper-fr_FR-upmc-medium/
└── whisper/
    ├── en/
    │   └── sherpa-onnx-whisper-tiny.en/
    ├── es/
    │   └── sherpa-onnx-whisper-tiny.es/
    └── multilingual/
        └── sherpa-onnx-whisper-tiny/
```

### Phase 2: Service Updates

#### 2.1 Update STT Service

**File**: `src/services/stt/service.rs`

**Changes**:
- `init()` method: Accept `language: &str` parameter
- Dynamic file name resolution based on language
- Support multilingual Whisper models (no language suffix)
- Update `WhisperConfig` to use provided language

**Key Changes**:
```rust
// Before:
language: "en".to_string(),
encoder: "tiny.en-encoder.int8.onnx"

// After:
language: language.to_string(),
encoder: config.get_encoder_filename() // e.g., "tiny-encoder.int8.onnx" for multilingual
```

#### 2.2 Update TTS Service
e
- `init()` method: Accept `language: &str` parameter
- Dynamic model file selection based on language
- Support language-specific model directories
- Reinitialize engine when language changes

**Key Changes**:
```rust
// Before:
let model_file = self.model_manager.get_model_file(&ModelType::Tts, "en_US-amy-low.onnx");

// After:
let config = model_registry.get_config(ModelType::Tts, language)?;
let model_file = self.model_manager.get_model_file_for_language(&ModelType::Tts, language, &config.model_file);
```

### Phase 3: Model Registry Implementation

**New File**: `src/domain/model_registry.rs`

**Purpose**: Centralized configuration of available models

**Implementation Options**:

**Option A: Hardcoded Registry (Simple)**
```rust
pub fn default_registry() -> ModelRegistry {
    let mut registry = ModelRegistry::new();
    
    // TTS Models
    registry.register(ModelType::Tts, "en", ModelConfig {
        model_name: "vits-piper-en_US-amy-low",
        model_url: "https://.../vits-piper-en_US-amy-low.tar.bz2",
        required_files: vec!["en_US-amy-low.onnx", "tokens.txt"],
    });
    
    registry.register(ModelType::Tts, "es", ModelConfig {
        model_name: "vits-piper-es_ES-shared-medium",
        model_url: "https://.../vits-piper-es_ES-shared-medium.tar.bz2",
        required_files: vec!["es_ES-shared-medium.onnx", "tokens.txt"],
    });
    
    // STT Models
    registry.register(ModelType::Whisper, "en", ModelConfig {
        model_name: "sherpa-onnx-whisper-tiny.en",
        model_url: "https://.../sherpa-onnx-whisper-tiny.en.tar.bz2",
        required_files: vec!["tiny.en-encoder.int8.onnx", "tiny.en-decoder.int8.onnx", "tiny.en-tokens.txt"],
    });
    
    registry.register(ModelType::Whisper, "multilingual", ModelConfig {
        model_name: "sherpa-onnx-whisper-tiny",
        model_url: "https://.../sherpa-onnx-whisper-tiny.tar.bz2",
        required_files: vec!["tiny-encoder.int8.onnx", "tiny-decoder.int8.onnx", "tiny-tokens.txt"],
    });
    
    registry
}
```

**Option B: JSON Configuration File (Flexible)**
```json
{
  "tts": {
    "en": {
      "model_name": "vits-piper-en_US-amy-low",
      "model_url": "https://...",
      "required_files": ["en_US-amy-low.onnx", "tokens.txt"]
    },
    "es": { ... }
  },
  "whisper": {
    "en": { ... },
    "multilingual": { ... }
  }
}
```

**Recommendation**: Start with Option A (hardcoded), migrate to Option B later if needed.

### Phase 4: Language Parameter Propagation

#### 4.1 Update Service Initialization

**Files**: `src/services/stt/service.rs`, `src/services/tts/service.rs`

**Changes**:
- Store language in service state
- Pass language to `init()` method
- Update `set_language()` to reinitialize with new model

#### 4.2 Update DBus Interface

**File**: `src/daemon/service.rs`

**Changes**:
- Language already passed in DBus methods (`Tts(text, language)`, `Stt(language, ...)`)
- Ensure language is propagated to service initialization
- Add validation for supported languages

#### 4.3 Add Language Validation

**New Function**: `validate_language(model_type: ModelType, language: &str) -> Result<()>`

**Purpose**: Check if language is supported before attempting download

---

## 🔧 Implementation Steps

### Step 1: Create Model Configuration System
1. Create `src/domain/model_config.rs` with `ModelConfig` struct
2. Create `src/domain/model_registry.rs` with `ModelRegistry`
3. Populate registry with English models (maintain backward compatibility)
4. Update `ModelType` to use registry

### Step 2: Update ModelManager
1. Add `ensure_models_present_for_language(model_type, language)` method
2. Update directory structure to include language subdirectories
3. Update file path resolution to be language-aware

### Step 3: Update STT Service
1. Add `language` parameter to `init()`
2. Update file name resolution to use `ModelConfig`
3. Support both language-specific and multilingual models
4. Update `start_listening()` to use language from state

### Step 4: Update TTS Service
1. Add `language` parameter to `init()`
2. Update model file selection to use `ModelConfig`
3. Make `set_language()` reinitialize the engine

### Step 5: Add New Languages
1. Research available models on [sherpa-onnx releases](https://github.com/k2-fsa/sherpa-onnx/releases)
2. Add model configurations to registry
3. Test download and initialization

### Step 6: Testing
1. Test English (backward compatibility)
2. Test new languages
3. Test language switching
4. Test model download for new languages

---

## 📋 Available Models (sherpa-onnx)

### TTS Models (VITS-Piper)

**English:**
- `vits-piper-en_US-amy-low` ✅ (currently used)
- `vits-piper-en_US-amy-medium`
- `vits-piper-en_US-amy-high`
- `vits-piper-en_US-lessac-medium`
- `vits-piper-en_US-libritts-high`

**Spanish:**
- `vits-piper-es_ES-shared-medium`
- `vits-piper-es_ES-dfx-medium`

**French:**
- `vits-piper-fr_FR-upmc-medium`
- `vits-piper-fr_FR-siwis-medium`

**German:**
- `vits-piper-de_DE-thorsten-medium`

**And many more...** (See [sherpa-onnx releases](https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models))

### STT Models (Whisper)

**🎯 Key Insight: Whisper supports 57-99 languages!**

Whisper models come in two flavors:

#### 1. Language-Specific Models (English-Optimized)
These models are optimized for a single language and typically perform better for that language:

- `sherpa-onnx-whisper-tiny.en` ✅ (currently used - English only)
- `sherpa-onnx-whisper-base.en` (English only)
- `sherpa-onnx-whisper-small.en` (English only)
- `sherpa-onnx-whisper-tiny.es` (Spanish)
- `sherpa-onnx-whisper-tiny.fr` (French)
- `sherpa-onnx-whisper-tiny.de` (German)
- `sherpa-onnx-whisper-tiny.zh` (Chinese)
- And more language-specific variants...

**Pros:**
- Smaller file size (~75MB for tiny.en vs ~75MB for multilingual tiny)
- Slightly better accuracy for the target language
- Faster inference (no language detection overhead)

**Cons:**
- Only supports one language
- Need to download multiple models for multi-language support

#### 2. Multilingual Models (All Languages in One!)
These models support **57-99 languages** in a single model:

- `sherpa-onnx-whisper-tiny` (supports 99 languages, ~75MB)
- `sherpa-onnx-whisper-base` (supports 99 languages, ~150MB)
- `sherpa-onnx-whisper-small` (supports 99 languages, ~500MB)
- `sherpa-onnx-whisper-medium` (supports 99 languages, ~1.5GB)
- `sherpa-onnx-whisper-large-v3` (supports 99 languages, ~3GB)

**Pros:**
- **One model for all languages!** 🎉
- Can auto-detect language
- Can translate to English from any supported language
- No need to download multiple models

**Cons:**
- Slightly larger (but same size as language-specific for tiny/base)
- May be slightly slower due to language detection

#### Supported Languages (57 with good quality, 99 total)

The multilingual Whisper models support these languages:

**European Languages:**
- English, Spanish, French, German, Italian, Portuguese, Russian, Polish, Dutch, Greek, Czech, Romanian, Hungarian, Bulgarian, Croatian, Serbian, Slovak, Slovenian, Ukrainian, Swedish, Norwegian, Danish, Finnish, Estonian, Latvian, Lithuanian, Icelandic, Irish, Welsh, Catalan, Galician, Basque, Maltese

**Asian Languages:**
- Chinese (Mandarin), Japanese, Korean, Hindi, Bengali, Tamil, Telugu, Marathi, Gujarati, Kannada, Malayalam, Urdu, Punjabi, Thai, Vietnamese, Indonesian, Malay, Filipino/Tagalog, Nepali, Sinhala, Burmese, Khmer, Lao

**Middle Eastern & African:**
- Arabic, Hebrew, Persian (Farsi), Turkish, Azerbaijani, Armenian, Georgian, Swahili, Afrikaans

**And more...** (Total: 99 languages trained, 57 with WER < 50%)

#### Recommendation for Your Project

**Best Approach: Use Multilingual Whisper Model**

Instead of `tiny.en`, use `tiny` (multilingual):
- ✅ Same file size (~75MB)
- ✅ Supports 99 languages in one model
- ✅ No need to download multiple models
- ✅ Can handle language switching automatically
- ✅ Can translate to English from any language

**Migration Path:**
1. Change default from `tiny.en` to `tiny` (multilingual)
2. Update file names: `tiny-encoder.int8.onnx` (no `.en` suffix)
3. Set language in config: `language: "en"` (or any other supported language)
4. The model will handle the rest!

**Other Architectures:**
- `sherpa-onnx-paraformer-zh-2023-03-14` (Chinese, Paraformer - specialized for Chinese)
- `sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20` (Chinese-English, streaming)

---

## 🎨 Design Considerations

### Model Size vs Quality Trade-off

**TTS:**
- `low`: ~10-20MB, faster, lower quality
- `medium`: ~30-50MB, balanced
- `high`: ~50-100MB, slower, higher quality

**STT (Whisper):**
- `tiny`: ~75MB, fastest, lowest accuracy
- `base`: ~150MB, balanced
- `small`: ~500MB, better accuracy
- `medium`: ~1.5GB, high accuracy
- `large-v3`: ~3GB, best accuracy, slowest

### Recommendation

1. **Default**: Use smallest models (current: `tiny`, `low`)
2. **Optional**: Allow users to configure model size
3. **Future**: Auto-select based on available resources

### Language Detection

**Option 1**: Use multilingual Whisper model ⭐ **RECOMMENDED**
- ✅ **One model supports 99 languages!**
- ✅ Same file size as language-specific (`tiny` = `tiny.en` in size)
- ✅ Can auto-detect language
- ✅ Can translate to English from any language
- ✅ No need to download/store multiple models
- ⚠️ Slightly slower due to language detection (negligible for most use cases)

**Option 2**: Language-specific models
- ✅ Slightly better accuracy for target language
- ✅ Slightly faster (no language detection)
- ❌ Only supports one language
- ❌ Need to download multiple models for multi-language support
- ❌ Must manually specify language

**Recommendation**: 
- **Default to multilingual Whisper model** (`tiny` instead of `tiny.en`)
- Keep language-specific models as optional for users who want maximum English performance
- This gives you 99 languages out of the box with minimal changes!

---

## 🚀 Migration Path

### Backward Compatibility

1. **Default Language**: If no language specified, use "en" (current behavior)
2. **Default Models**: Keep current models as defaults
3. **Gradual Migration**: Add new languages incrementally

### Breaking Changes

**Minimal**: Only if you change the default model or remove English support.

**Safe Migration**:
- Keep English as default
- Add new languages as opt-in
- Maintain current API surface

---

## 📝 Summary

### Current State
- ✅ English-only support
- ✅ Automatic model download
- ✅ Single model per type
- ❌ No language switching
- ❌ No model selection

### Target State
- ✅ Multi-language support
- ✅ Language-aware model selection
- ✅ Configurable model registry
- ✅ Support for multiple model sizes
- ✅ Language switching at runtime

### Estimated Effort
- **Phase 1-2**: 2-3 days (core infrastructure)
- **Phase 3-4**: 1-2 days (service updates)
- **Phase 5-6**: 1-2 days (testing, new languages)
- **Total**: ~1 week for basic multi-language support

---

## 🔗 Resources

- [sherpa-onnx GitHub Releases](https://github.com/k2-fsa/sherpa-onnx/releases)
- [sherpa-onnx TTS Models](https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models)
- [sherpa-onnx ASR Models](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models)
- [Whisper Languages](https://github.com/openai/whisper/blob/main/whisper/tokenizer.py)


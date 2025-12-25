# Models Verification Status

## ✅ Confirmed Models

These models have been **verified** against the codebase and confirmed to exist:

### TTS Models (Confirmed from codebase)
- ✅ `vits-piper-en_US-amy-low` - Currently used in codebase
- ✅ URL pattern verified: `https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/{model}.tar.bz2`

### STT/Whisper Models (Confirmed from codebase)
- ✅ `sherpa-onnx-whisper-tiny.en` - Currently used in codebase
- ✅ URL pattern verified: `https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/{model}.tar.bz2`

### VAD Models (Confirmed from codebase)
- ✅ `silero_vad.onnx` - Referenced in codebase
- ✅ URL pattern verified: `https://github.com/k2-fsa/sherpa-onnx/releases/download/vad-models/{model}.onnx`

## 📋 Models in JSON File

### TTS Models (14 models)
- **English (US)**: 5 models (amy-low, amy-medium, amy-high, lessac-medium, libritts-high)
- **Spanish**: 2 models
- **French**: 2 models
- **German**: 1 model
- **Italian**: 1 model
- **Portuguese**: 1 model
- **Chinese**: 1 model
- **Japanese**: 1 model
- **Russian**: 1 model

**Status**: URLs follow confirmed pattern. Additional TTS models should be verified on GitHub releases page.

### STT/Whisper Models (7 models)
- **Multilingual**: 5 models (tiny, base, small, medium, large-v3) - Supports 99 languages
- **English-only**: 2 models (tiny.en, base.en)

**Status**: All URLs follow confirmed pattern. Multilingual models are recommended.

### VAD Models (1 model)
- **silero_vad**: Language-agnostic VAD model

**Status**: Confirmed from codebase.

## 🔍 Verification Method

1. **Codebase Analysis**: Checked `src/domain/models.rs` for existing model URLs
2. **URL Pattern**: All URLs follow the pattern from confirmed models in codebase
3. **GitHub Releases**: Pattern matches `k2-fsa/sherpa-onnx` release structure

## ⚠️ Recommended Next Steps

1. **Test Downloads**: Verify that all URLs in `models.json` actually work by attempting downloads
2. **Check GitHub Releases**: Visit https://github.com/k2-fsa/sherpa-onnx/releases to confirm all listed models exist
3. **Add More TTS Models**: Many more TTS models may be available - check the releases page

## 📝 Language Codes

All language codes in the JSON follow ISO 639-1/639-2 standards and match Whisper's supported language codes.

## 🎯 Usage

The `models.json` file can be used to:
- Generate model registry in code
- Validate model downloads
- Display available models to users
- Auto-configure model selection based on language


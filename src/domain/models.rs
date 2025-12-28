/// Model types supported by the application
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelType {
    /// Text-to-Speech model
    Tts,
    /// Speech-to-Text model (streaming)
    Stt,
    /// Whisper model (offline STT)
    Whisper,
    /// Voice Activity Detection model
    Vad,
}

impl ModelType {
    /// Get the subdirectory name for this model type
    pub(crate) fn subdirectory(&self) -> &'static str {
        match self {
            ModelType::Tts => "tts",
            ModelType::Stt => "stt",
            ModelType::Whisper => "whisper",
            ModelType::Vad => "vad",
        }
    }

    /// Get the default model name for this type
    pub(crate) fn default_model_name(&self) -> &'static str {
        match self {
            ModelType::Tts => "vits-piper-en_US-amy-low",
            ModelType::Stt => "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20",
            ModelType::Whisper => "sherpa-onnx-whisper-tiny.en",
            ModelType::Vad => "silero_vad",
        }
    }

    /// Get the default download URL for this model type
    pub(crate) fn default_url(&self) -> &'static str {
        match self {
            ModelType::Tts => {
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-en_US-amy-low.tar.bz2"
            }
            ModelType::Stt => {
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20.tar.bz2"
            }
            ModelType::Whisper => {
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-tiny.en.tar.bz2"
            }
            ModelType::Vad => {
                // VAD model URL - may need to be updated if this doesn't work
                // For now, using a placeholder - VAD is optional for STT
                "https://github.com/k2-fsa/sherpa-onnx/releases/download/vad-models/silero_vad.onnx"
            }
        }
    }

    /// Get the list of required files for this model type
    pub(crate) fn required_files(&self) -> Vec<&'static str> {
        match self {
            ModelType::Tts => vec!["en_US-amy-low.onnx", "tokens.txt"],
            ModelType::Stt => vec![
                "encoder-epoch-99-avg-1.onnx",
                "decoder-epoch-99-avg-1.onnx",
                "joiner-epoch-99-avg-1.onnx",
                "tokens.txt",
            ],
            ModelType::Whisper => vec![
                "tiny.en-encoder.int8.onnx",
                "tiny.en-decoder.int8.onnx",
                "tiny.en-tokens.txt",
            ],
            ModelType::Vad => vec!["silero_vad.onnx"],
        }
    }
}





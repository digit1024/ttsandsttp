//! Cloud provider clients and backend dispatch.
//!
//! - [`DashScopeClient`]: Alibaba DashScope (Qwen) ASR + TTS
//! - [`OpenAiClient`]: OpenAI-compatible transcription + speech endpoints
//!   (Groq, OpenRouter, custom servers)
//! - [`build_stt_backend`] / [`build_tts_backend`]: resolve the configured
//!   provider into a concrete backend

mod backend;
mod dashscope;
mod openai;

pub use backend::{
    build_stt_backend, build_tts_backend, provider_exists, provider_key_spec, provider_label,
    ResolvedStt, ResolvedTts, SttBackend, TtsBackend,
};
pub use dashscope::{DashScopeClient, DASHSCOPE_TTS_SAMPLE_RATE};
pub use openai::{
    decode_speech_audio, AudioFormat, OpenAiClient, SpeechAudio, SpeechError, SpeechRequest,
    OPENAI_TTS_PCM_SAMPLE_RATE,
};

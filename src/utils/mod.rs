//! Utility functions

pub mod audio_source;
pub mod beep;
pub mod lang;
pub mod onnx_metadata;
pub mod text_splitter;
pub mod tts_chunker;
pub mod wav;
pub mod wav_chunker;

pub use audio_source::DirectSampleSource;
pub use beep::{play_beep, play_beep_blocking, BEEP_HIGH_WAV, BEEP_LOW_WAV};
pub use lang::normalize_language_code;
pub use text_splitter::split_into_sentences;
pub use tts_chunker::{
    chunk_text_for_tts, DEFAULT_TTS_FIRST_CHUNK_LENGTH, DEFAULT_TTS_MAX_INPUT_LENGTH,
};
pub use wav::create_wav_buffer;
pub use wav_chunker::{chunk_wav, merge_transcripts};

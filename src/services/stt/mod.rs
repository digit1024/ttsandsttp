pub mod audio_processor;
pub mod audio_utils;
pub mod pause_detector;
pub mod service;

pub use audio_utils::{
    calculate_audio_stats, has_sufficient_amplitude, resample_linear, stereo_to_mono,
    validate_and_clean_audio, AudioStats, AudioValidationStats, TARGET_SAMPLE_RATE,
};
pub use service::SttService;




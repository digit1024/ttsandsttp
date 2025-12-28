//! Audio Processing
//!
//! Handles audio preprocessing: stereo-to-mono conversion and resampling.

use super::audio_utils::{resample_linear, stereo_to_mono, TARGET_SAMPLE_RATE};
use tracing;

/// Audio Processor
///
/// Processes raw audio samples by:
/// - Converting stereo to mono (if needed)
/// - Resampling to target sample rate (16kHz)
pub struct AudioProcessor {
    input_sample_rate: u32,
    channels: usize,
    needs_resampling: bool,
}

impl AudioProcessor {
    /// Create a new audio processor with the given input configuration
    pub fn new(input_sample_rate: u32, channels: usize) -> Self {
        let needs_resampling = input_sample_rate != TARGET_SAMPLE_RATE;
        
        if needs_resampling {
            tracing::warn!(
                "Audio input is {}Hz, resampling to {}Hz",
                input_sample_rate,
                TARGET_SAMPLE_RATE
            );
        }
        
        Self {
            input_sample_rate,
            channels,
            needs_resampling,
        }
    }

    /// Process audio samples: convert to mono and resample if needed
    pub fn process(&self, samples: Vec<f32>) -> Vec<f32> {
        // Convert to mono if stereo
        let mono_samples = if self.channels > 1 {
            stereo_to_mono(&samples, self.channels)
        } else {
            samples
        };

        // Resample to 16kHz if needed
        if self.needs_resampling {
            resample_linear(&mono_samples, self.input_sample_rate, TARGET_SAMPLE_RATE)
        } else {
            mono_samples
        }
    }

    /// Get the target sample rate
    pub fn target_sample_rate() -> u32 {
        TARGET_SAMPLE_RATE
    }
}


//! Custom rodio source for direct sample playback
//!
//! Allows feeding audio samples directly to rodio without WAV encoding/decoding.

use rodio::Source;
use std::time::Duration;

/// A rodio source that plays samples directly from a Vec<i16>
/// Converts i16 samples to f32 on-the-fly
pub struct DirectSampleSource {
    samples: Vec<i16>,
    sample_rate: u32,
    channels: u16,
    position: usize,
}

impl DirectSampleSource {
    /// Create a new source from i16 samples
    pub fn new(samples: Vec<i16>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
            channels: 1, // Mono
            position: 0,
        }
    }
}

impl Iterator for DirectSampleSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position < self.samples.len() {
            let sample_i16 = self.samples[self.position];
            self.position += 1;
            // Convert i16 to f32 in range [-1.0, 1.0]
            Some(sample_i16 as f32 / i16::MAX as f32)
        } else {
            None
        }
    }
}

impl Source for DirectSampleSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.samples.len().saturating_sub(self.position))
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        let total_samples = self.samples.len();
        let duration_secs = total_samples as f64 / self.sample_rate as f64;
        Some(Duration::from_secs_f64(duration_secs))
    }
}

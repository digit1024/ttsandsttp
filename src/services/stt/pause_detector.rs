//! Pause Detection
//!
//! Detects pauses in audio streams by tracking silence periods after speech detection.
//! Used to automatically trigger transcription when the user stops speaking.

use std::time::{Duration, Instant};

use crate::utils::format_timestamp;

// Audio processing constants
const AUDIO_ACTIVITY_THRESHOLD: f32 = 0.01; // Threshold for detecting speech
const SPEECH_THRESHOLD: f32 = 0.03; // Higher threshold for definite speech (reduces false positives from background noise)

/// Pause Detector
///
/// Tracks audio activity and detects when a pause (silence period) exceeds
/// the configured threshold, indicating the user has finished speaking.
pub struct PauseDetector {
    pause_duration: Duration,
    silence_start: Option<Instant>,
    has_detected_speech: bool,
}

impl PauseDetector {
    /// Create a new pause detector with the specified pause duration
    pub fn new(pause_duration: Duration) -> Self {
        Self {
            pause_duration,
            silence_start: None,
            has_detected_speech: false,
        }
    }

    /// Process a chunk of audio samples and detect speech/activity/silence
    /// 
    /// Returns:
    /// - `Some(true)` if a pause was detected (should stop recording)
    /// - `Some(false)` if speech/activity was detected (continue recording)
    /// - `None` if no significant change (continue monitoring)
    pub fn process_samples(&mut self, samples: &[f32]) -> Option<bool> {
        // Calculate maximum amplitude in this chunk
        let max_abs = samples.iter().fold(0.0f32, |acc, x| acc.max(x.abs()));
        
        // Check if this chunk contains speech
        let is_speech = max_abs > SPEECH_THRESHOLD;
        let has_activity = max_abs > AUDIO_ACTIVITY_THRESHOLD;
        
        if is_speech {
            // Definite speech detected - reset silence tracking
            self.silence_start = None;
            if !self.has_detected_speech {
                self.has_detected_speech = true;
                eprintln!(
                    "[{}] 🎤 Speech detected (amplitude: {:.4})",
                    format_timestamp(),
                    max_abs
                );
            }
            Some(false) // Continue recording
        } else if has_activity {
            // Low-level activity - don't reset silence, but mark that we've had activity
            if !self.has_detected_speech {
                self.has_detected_speech = true;
                self.silence_start = None; // Reset silence on first activity
                eprintln!(
                    "[{}] 🎤 Audio activity detected (amplitude: {:.4})",
                    format_timestamp(),
                    max_abs
                );
            }
            // If we already detected speech, low activity doesn't reset silence
            Some(false) // Continue recording
        } else {
            // Silence detected - start or continue tracking silence period
            if self.has_detected_speech {
                if self.silence_start.is_none() {
                    self.silence_start = Some(Instant::now());
                }
                
                // Check if silence duration exceeds pause threshold
                if let Some(silence_start_time) = self.silence_start {
                    let silence_duration = silence_start_time.elapsed();
                    if silence_duration > self.pause_duration {
                        eprintln!(
                            "[{}] ⏸️  Pause detected: {:.2}s of silence",
                            format_timestamp(),
                            silence_duration.as_secs_f64()
                        );
                        return Some(true); // Pause detected - should stop
                    }
                }
            }
            None // Continue monitoring, no significant change
        }
    }

    /// Reset the detector state (for new recording session)
    pub fn reset(&mut self) {
        self.silence_start = None;
        self.has_detected_speech = false;
    }
}


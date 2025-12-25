/// Audio processing utilities
/// 
/// This module provides reusable audio processing functions following
/// the Single Responsibility Principle - each function has one clear purpose.

/// Target sample rate for STT models (16kHz)
pub const TARGET_SAMPLE_RATE: u32 = 16000;

/// Convert stereo/interleaved audio to mono
/// 
/// # Arguments
/// * `samples` - Interleaved audio samples (e.g., [L, R, L, R, ...])
/// * `channels` - Number of channels in the input
/// 
/// # Returns
/// Mono audio samples (averaged across channels)
pub fn stereo_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }

    samples
        .chunks(channels)
        .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Simple linear resampling
/// 
/// Uses linear interpolation to resample audio from one sample rate to another.
/// This is a basic implementation suitable for real-time processing.
/// 
/// # Arguments
/// * `samples` - Input audio samples
/// * `from_rate` - Source sample rate
/// * `to_rate` - Target sample rate
/// 
/// # Returns
/// Resampled audio samples
pub fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return samples.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (samples.len() as f64 / ratio).round() as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_pos = i as f64 * ratio;
        let src_idx = src_pos as usize;
        let frac = src_pos - src_idx as f64;

        if src_idx + 1 < samples.len() {
            // Linear interpolation
            let value = samples[src_idx] * (1.0 - frac as f32) 
                      + samples[src_idx + 1] * frac as f32;
            output.push(value);
        } else if src_idx < samples.len() {
            output.push(samples[src_idx]);
        } else {
            output.push(0.0);
        }
    }

    output
}

/// Audio validation and cleaning statistics
#[derive(Debug, Clone, Default)]
pub struct AudioValidationStats {
    pub has_invalid: bool,
    pub max_abs_before: f32,
    pub max_abs_after: f32,
    pub clamped_count: usize,
}

/// Validate and clean audio samples
/// 
/// Removes NaN/Inf values and clamps samples to [-1.0, 1.0] range.
/// This prevents C++ exceptions in the underlying STT library.
/// 
/// # Arguments
/// * `samples` - Mutable reference to audio samples to clean
/// 
/// # Returns
/// Statistics about the cleaning operation
pub fn validate_and_clean_audio(samples: &mut [f32]) -> AudioValidationStats {
    let mut stats = AudioValidationStats::default();
    
    for sample in samples.iter_mut() {
        let abs = sample.abs();
        if abs > stats.max_abs_before {
            stats.max_abs_before = abs;
        }
        
        if !sample.is_finite() {
            stats.has_invalid = true;
            *sample = 0.0;
        } else if abs > 1.0 {
            *sample = sample.signum() * 1.0;
            stats.clamped_count += 1;
            stats.max_abs_after = 1.0;
        } else if abs > stats.max_abs_after {
            stats.max_abs_after = abs;
        }
    }
    
    stats
}

/// Check if audio has sufficient amplitude for recognition
/// 
/// # Arguments
/// * `samples` - Audio samples to check
/// * `threshold` - Minimum amplitude threshold (default: 0.001)
/// 
/// # Returns
/// True if audio has sufficient amplitude
pub fn has_sufficient_amplitude(samples: &[f32], threshold: f32) -> bool {
    samples.iter().any(|&s| s.abs() > threshold)
}

/// Get audio statistics for debugging
#[derive(Debug, Clone)]
pub struct AudioStats {
    pub sample_count: usize,
    pub non_zero_count: usize,
    pub min_val: f32,
    pub max_val: f32,
    pub max_abs: f32,
}

/// Calculate audio statistics
pub fn calculate_audio_stats(samples: &[f32]) -> AudioStats {
    let sample_count = samples.len();
    let non_zero_count = samples.iter().filter(|&&s| s.abs() > 0.0001).count();
    let max_abs = samples.iter().fold(0.0f32, |acc, &x| acc.max(x.abs()));
    let min_val = samples.iter().fold(0.0f32, |acc, &x| acc.min(x));
    let max_val = samples.iter().fold(0.0f32, |acc, &x| acc.max(x));
    
    AudioStats {
        sample_count,
        non_zero_count,
        min_val,
        max_val,
        max_abs,
    }
}


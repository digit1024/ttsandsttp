//! WAV file utility functions

use anyhow::Result;
use std::io::Write;

/// Create a simple WAV buffer from i16 samples
/// 
/// Creates a PCM WAV file (16-bit, mono) from the given samples.
/// 
/// # Arguments
/// * `samples` - Audio samples as 16-bit integers
/// * `sample_rate` - Sample rate in Hz (e.g., 22050, 44100)
/// 
/// # Returns
/// A `Vec<u8>` containing the complete WAV file data
pub fn create_wav_buffer(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    
    // WAV header
    buffer.write_all(b"RIFF")?;
    let data_size = (samples.len() * 2 + 36) as u32;
    buffer.write_all(&data_size.to_le_bytes())?;
    buffer.write_all(b"WAVE")?;
    
    // fmt chunk
    buffer.write_all(b"fmt ")?;
    buffer.write_all(&16u32.to_le_bytes())?; // fmt chunk size
    buffer.write_all(&1u16.to_le_bytes())?; // audio format (PCM)
    buffer.write_all(&1u16.to_le_bytes())?; // num channels (mono)
    buffer.write_all(&sample_rate.to_le_bytes())?; // sample rate
    let byte_rate = sample_rate * 2; // sample_rate * num_channels * bits_per_sample / 8
    buffer.write_all(&byte_rate.to_le_bytes())?;
    buffer.write_all(&2u16.to_le_bytes())?; // block align
    buffer.write_all(&16u16.to_le_bytes())?; // bits per sample
    
    // data chunk
    buffer.write_all(b"data")?;
    let data_chunk_size = (samples.len() * 2) as u32;
    buffer.write_all(&data_chunk_size.to_le_bytes())?;
    
    // Write samples
    for &sample in samples {
        buffer.write_all(&sample.to_le_bytes())?;
    }
    
    Ok(buffer)
}


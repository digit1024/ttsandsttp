use anyhow::Result;
use std::io::Cursor;

// Embed beep audio files (paths are relative to source file)
pub const BEEP_HIGH_WAV: &[u8] = include_bytes!("../../beep-high.wav");
pub const BEEP_LOW_WAV: &[u8] = include_bytes!("../../beep-low.wav");

/// Play a beep sound from embedded WAV data (async version)
/// 
/// Uses spawn_blocking to avoid blocking the async runtime.
pub async fn play_beep(wav_data: &'static [u8]) -> Result<()> {
    use rodio::{Decoder, OutputStream, Sink};

    tokio::task::spawn_blocking(move || -> Result<()> {
        let (_stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| anyhow::anyhow!("Failed to create audio output stream: {}", e))?;

        let sink = Sink::try_new(&stream_handle)
            .map_err(|e| anyhow::anyhow!("Failed to create audio sink: {}", e))?;

        let cursor = Cursor::new(wav_data);
        let source = Decoder::new(cursor)
            .map_err(|e| anyhow::anyhow!("Failed to create audio decoder: {}", e))?;

        sink.append(source);
        sink.sleep_until_end();

        Ok(())
    })
    .await??;

    Ok(())
}

/// Play a beep sound from embedded WAV data (blocking version)
/// 
/// Note: Prefer using `play_beep()` async version when possible.
/// This blocking version is kept for cases where async is not available (e.g., sync functions).
pub fn play_beep_blocking(wav_data: &'static [u8]) -> Result<()> {
    use rodio::{Decoder, OutputStream, Sink};

    let (_stream, stream_handle) = OutputStream::try_default()
        .map_err(|e| anyhow::anyhow!("Failed to create audio output stream: {}", e))?;

    let sink = Sink::try_new(&stream_handle)
        .map_err(|e| anyhow::anyhow!("Failed to create audio sink: {}", e))?;

    let cursor = Cursor::new(wav_data);
    let source = Decoder::new(cursor)
        .map_err(|e| anyhow::anyhow!("Failed to create audio decoder: {}", e))?;

    sink.append(source);
    sink.sleep_until_end();

    Ok(())
}


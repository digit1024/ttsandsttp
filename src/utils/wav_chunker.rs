//! WAV chunking and transcript merging for long recordings.
//!
//! Port of mbdictate's `WavChunker`: splits PCM16 WAV audio into
//! provider-sized chunks (with a small overlap so words on the boundary are
//! heard twice) and merges the transcripts back together, dropping the
//! duplicated overlap words.

struct WavInfo {
    audio_format: u16,
    channels: u16,
    sample_rate: u32,
    byte_rate: u32,
    block_align: u16,
    bits_per_sample: u16,
    data_offset: usize,
    data_size: usize,
}

fn ascii(bytes: &[u8], offset: usize, len: usize) -> &'static str {
    match &bytes[offset..offset + len] {
        b"RIFF" => "RIFF",
        b"WAVE" => "WAVE",
        b"fmt " => "fmt ",
        b"data" => "data",
        _ => "",
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn parse_wav(bytes: &[u8]) -> Option<WavInfo> {
    if bytes.len() < 44 {
        return None;
    }
    if ascii(bytes, 0, 4) != "RIFF" || ascii(bytes, 8, 4) != "WAVE" {
        return None;
    }

    let mut info: Option<(u16, u16, u32, u32, u16, u16)> = None;
    let mut data: Option<(usize, usize)> = None;

    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let id = ascii(bytes, offset, 4);
        let size = u32_at(bytes, offset + 4) as usize;
        let body = offset + 8;
        if id == "fmt " && body + 16 <= bytes.len() {
            info = Some((
                u16_at(bytes, body),
                u16_at(bytes, body + 2),
                u32_at(bytes, body + 4),
                u32_at(bytes, body + 8),
                u16_at(bytes, body + 12),
                u16_at(bytes, body + 14),
            ));
        } else if id == "data" {
            data = Some((body, size.min(bytes.len() - body)));
        }
        let padded = if size.is_multiple_of(2) { size } else { size + 1 };
        offset = body + padded;
    }

    let (audio_format, channels, sample_rate, byte_rate, block_align, bits_per_sample) = info?;
    let (data_offset, data_size) = data?;
    if data_size == 0 {
        return None;
    }
    Some(WavInfo {
        audio_format,
        channels,
        sample_rate,
        byte_rate,
        block_align,
        bits_per_sample,
        data_offset,
        data_size,
    })
}

fn build_wav(info: &WavInfo, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&info.audio_format.to_le_bytes());
    out.extend_from_slice(&info.channels.to_le_bytes());
    out.extend_from_slice(&info.sample_rate.to_le_bytes());
    out.extend_from_slice(&info.byte_rate.to_le_bytes());
    out.extend_from_slice(&info.block_align.to_le_bytes());
    out.extend_from_slice(&info.bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// Split a PCM WAV buffer into chunks of at most `max_seconds` each,
/// overlapping by `overlap_seconds`. Returns the original buffer unchanged
/// when it already fits or cannot be parsed.
pub fn chunk_wav(bytes: &[u8], max_seconds: u32, overlap_seconds: f64) -> Vec<Vec<u8>> {
    if max_seconds == 0 {
        return vec![bytes.to_vec()];
    }
    let Some(info) = parse_wav(bytes) else {
        return vec![bytes.to_vec()];
    };
    let frame_size = info.channels as usize * (info.bits_per_sample as usize / 8);
    if frame_size == 0 || info.byte_rate == 0 {
        return vec![bytes.to_vec()];
    }

    let max_bytes = max_seconds as usize * info.byte_rate as usize;
    if info.data_size <= max_bytes {
        return vec![bytes.to_vec()];
    }

    let overlap_raw = (overlap_seconds * info.byte_rate as f64) as usize;
    let overlap_bytes = (overlap_raw / frame_size) * frame_size;

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < info.data_size {
        let end = (start + max_bytes).min(info.data_size);
        let aligned_length = ((end - start) / frame_size) * frame_size;
        if aligned_length == 0 {
            break;
        }
        let aligned_end = start + aligned_length;
        let data = &bytes[info.data_offset + start..info.data_offset + aligned_end];
        chunks.push(build_wav(&info, data));
        if aligned_end >= info.data_size {
            break;
        }
        let next = aligned_end.saturating_sub(overlap_bytes);
        start = if next > start { next } else { aligned_end };
    }
    if chunks.is_empty() {
        vec![bytes.to_vec()]
    } else {
        chunks
    }
}

/// Merge per-chunk transcripts, removing the words duplicated by the overlap.
pub fn merge_transcripts(parts: &[String]) -> String {
    let pieces: Vec<Vec<String>> = parts
        .iter()
        .map(|p| {
            p.split_whitespace()
                .map(|w| w.to_string())
                .collect::<Vec<_>>()
        })
        .filter(|words: &Vec<String>| !words.is_empty())
        .collect();
    if pieces.is_empty() {
        return String::new();
    }
    if pieces.len() == 1 {
        return pieces[0].join(" ");
    }

    let mut words: Vec<String> = pieces[0].clone();
    for next in pieces.iter().skip(1) {
        let max_overlap = words.len().min(next.len());
        let mut overlap = 0usize;
        for k in (1..=max_overlap).rev() {
            let tail: String = words[words.len() - k..]
                .iter()
                .map(|w| w.to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            let head: String = next[..k]
                .iter()
                .map(|w| w.to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            if tail == head {
                overlap = k;
                break;
            }
        }
        words.extend(next[overlap..].iter().cloned());
    }
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::create_wav_buffer;

    fn wav_of_seconds(seconds: usize) -> Vec<u8> {
        let samples = vec![0i16; 16_000 * seconds];
        create_wav_buffer(&samples, 16_000).unwrap()
    }

    #[test]
    fn short_audio_is_not_chunked() {
        let wav = wav_of_seconds(10);
        let chunks = chunk_wav(&wav, 45, 1.0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], wav);
    }

    #[test]
    fn long_audio_is_chunked_with_overlap() {
        let wav = wav_of_seconds(100);
        let chunks = chunk_wav(&wav, 45, 1.0);
        assert_eq!(chunks.len(), 3); // 0-45, 44-89, 88-100
        for chunk in &chunks {
            let info = parse_wav(chunk).expect("valid wav chunk");
            assert!(info.data_size <= 45 * 32_000);
            assert_eq!(info.sample_rate, 16_000);
        }
    }

    #[test]
    fn invalid_input_passes_through() {
        let chunks = chunk_wav(b"not a wav", 45, 1.0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], b"not a wav");
    }

    #[test]
    fn merge_dedups_overlap_words() {
        let parts = vec![
            "hello world how".to_string(),
            "how are you".to_string(),
        ];
        assert_eq!(merge_transcripts(&parts), "hello world how are you");
    }

    #[test]
    fn merge_without_overlap_concatenates() {
        let parts = vec!["first part".to_string(), "second part".to_string()];
        assert_eq!(merge_transcripts(&parts), "first part second part");
    }

    #[test]
    fn merge_normalizes_whitespace() {
        let parts = vec!["  a   b ".to_string(), " c ".to_string()];
        assert_eq!(merge_transcripts(&parts), "a b c");
        assert_eq!(merge_transcripts(&[]), "");
    }
}

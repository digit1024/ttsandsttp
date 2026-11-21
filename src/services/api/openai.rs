//! OpenAI-compatible audio API client.
//!
//! Supports the two endpoints used by Groq, OpenRouter and any
//! OpenAI-compatible server:
//! - STT: multipart `POST /audio/transcriptions` (port of mbdictate's
//!   `SttClient.transcribeOpenAi`)
//! - TTS: JSON `POST /audio/speech` (port of LunaAI's
//!   `RemoteTtsProvider._synthesizeOpenAi`, including response-format
//!   negotiation, audio sniffing and PCM→WAV handling)

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};

use crate::secrets::{KeySpec, Secrets};
use crate::services::stt::stereo_to_mono;

/// Sample rate of raw PCM returned by OpenAI-compatible TTS endpoints.
pub const OPENAI_TTS_PCM_SAMPLE_RATE: u32 = 24_000;

/// Detected audio container of a synthesis response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Wav,
    Mp3,
    Pcm,
}

/// A synthesis failure, classified for retry by the playback pipeline.
#[derive(Debug)]
pub struct SpeechError {
    pub message: String,
    pub retryable: bool,
    pub status: Option<u16>,
    pub body: String,
}

impl std::fmt::Display for SpeechError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SpeechError {}

impl SpeechError {
    fn new(message: impl Into<String>, retryable: bool) -> Self {
        Self {
            message: message.into(),
            retryable,
            status: None,
            body: String::new(),
        }
    }

    /// True when the provider rejected the requested `response_format`,
    /// meaning a retry with another format may succeed.
    pub fn is_format_error(&self) -> bool {
        if self.status != Some(400) {
            return false;
        }
        let body = self.body.to_lowercase();
        body.contains("response_format")
            || body.contains("format")
            || body.contains("pcm")
            || body.contains("mp3")
    }
}

/// Decoded synthesis output ready for playback.
pub struct SpeechAudio {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
}

/// One synthesis request.
pub struct SpeechRequest<'a> {
    pub model: &'a str,
    pub text: &'a str,
    pub voice: &'a str,
    pub instructions: Option<&'a str>,
    /// Format requested first (from the model catalog); may be negotiated.
    pub preferred_format: &'a str,
    pub extra: &'a Value,
}

/// Construction parameters for [`OpenAiClient`].
pub struct OpenAiClientConfig {
    pub stt_endpoint: String,
    pub tts_endpoint: String,
    /// None for key-less servers (`requires_api_key = false`).
    pub key_spec: Option<KeySpec>,
    pub timeout_secs: u64,
}

/// Client for OpenAI-compatible transcription and speech endpoints.
pub struct OpenAiClient {
    http: Client,
    stt_endpoint: String,
    tts_endpoint: String,
    api_key: Option<SecretString>,
    /// Remembered working `response_format` per `endpoint|model` so a
    /// provider that rejects the preferred format is not probed per chunk.
    format_cache: Mutex<HashMap<String, String>>,
}

impl std::fmt::Debug for OpenAiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiClient")
            .field("stt_endpoint", &self.stt_endpoint)
            .field("tts_endpoint", &self.tts_endpoint)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl OpenAiClient {
    /// Build a client, resolving the API key via [`Secrets`] when required.
    pub async fn new(cfg: OpenAiClientConfig) -> Result<Self> {
        let api_key = match &cfg.key_spec {
            Some(spec) => Some(Secrets::resolve_spec(spec).await?),
            None => None,
        };
        let http = Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs.max(5)))
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self {
            http,
            stt_endpoint: cfg.stt_endpoint,
            tts_endpoint: cfg.tts_endpoint,
            api_key,
            format_cache: Mutex::new(HashMap::new()),
        })
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(key) => req.bearer_auth(key.expose_secret()),
            None => req,
        }
    }

    /// Transcribe a WAV buffer via multipart `/audio/transcriptions`.
    pub async fn transcribe(
        &self,
        model: &str,
        wav: &[u8],
        language: Option<&str>,
        hints: &[String],
        extra: &Value,
    ) -> Result<String> {
        if self.stt_endpoint.trim().is_empty() {
            bail!("OpenAI-compatible STT endpoint is not configured");
        }

        let mut form = Form::new()
            .text("model", model.to_string())
            .text("response_format", "json".to_string())
            .text("temperature", "0".to_string());
        if let Some(lang) = language.filter(|l| !l.trim().is_empty()) {
            form = form.text("language", lang.to_string());
        }
        if !hints.is_empty() {
            form = form.text(
                "prompt",
                format!("Expected vocabulary: {}", hints.join(", ")),
            );
        }
        if let Some(obj) = extra.as_object() {
            for (key, value) in obj {
                match value {
                    Value::String(s) => form = form.text(key.clone(), s.clone()),
                    Value::Number(n) => form = form.text(key.clone(), n.to_string()),
                    Value::Bool(b) => form = form.text(key.clone(), b.to_string()),
                    _ => {}
                }
            }
        }
        form = form.part(
            "file",
            Part::bytes(wav.to_vec())
                .file_name("audio.wav")
                .mime_str("audio/wav")
                .context("Invalid audio MIME type")?,
        );

        let response = self
            .auth(self.http.post(&self.stt_endpoint).multipart(form))
            .send()
            .await
            .context("OpenAI-compatible STT request failed")?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("STT request failed ({}): {}", status, summarize(&body));
        }

        let value: Value =
            serde_json::from_str(&body).context("STT returned invalid JSON")?;
        Ok(value
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string())
    }

    /// Synthesize speech via JSON `/audio/speech`, negotiating the response
    /// format and decoding the result into playable samples.
    pub async fn speech(
        &self,
        req: &SpeechRequest<'_>,
    ) -> std::result::Result<SpeechAudio, SpeechError> {
        if self.tts_endpoint.trim().is_empty() {
            return Err(SpeechError::new(
                "OpenAI-compatible TTS endpoint is not configured",
                false,
            ));
        }

        let cache_key = format!("{}|{}", self.tts_endpoint, req.model);
        let preferred = self
            .extra_format(req.extra)
            .or_else(|| self.format_cache.lock().unwrap().get(&cache_key).cloned())
            .unwrap_or_else(|| req.preferred_format.to_string());

        let formats: Vec<String> = if preferred == "pcm" {
            vec!["pcm".to_string()]
        } else {
            vec![preferred.clone(), "pcm".to_string()]
        };

        let mut last_error: Option<SpeechError> = None;
        for (index, format) in formats.iter().enumerate() {
            match self.post_speech(req, format).await {
                Ok((bytes, detected)) => {
                    if let Ok(mut cache) = self.format_cache.lock() {
                        cache.insert(cache_key.clone(), format.clone());
                    }
                    tracing::debug!(
                        "TTS: {} bytes format={:?} (requested {})",
                        bytes.len(),
                        detected,
                        format
                    );
                    return decode_speech_audio(bytes, detected);
                }
                Err(error) => {
                    let try_next = error.is_format_error() && index + 1 < formats.len();
                    tracing::debug!(
                        "TTS: request with format '{}' failed: {} (retry with pcm: {})",
                        format,
                        error.message,
                        try_next
                    );
                    if !try_next {
                        return Err(error);
                    }
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            SpeechError::new("TTS synthesis failed after format negotiation", false)
        }))
    }

    fn extra_format(&self, extra: &Value) -> Option<String> {
        extra
            .get("response_format")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    async fn post_speech(
        &self,
        req: &SpeechRequest<'_>,
        format: &str,
    ) -> std::result::Result<(Vec<u8>, AudioFormat), SpeechError> {
        let mut body = json!({
            "model": req.model,
            "input": req.text,
            "voice": req.voice,
        });
        if let Some(instructions) = req.instructions.filter(|s| !s.trim().is_empty()) {
            body["instructions"] = json!(instructions);
        }
        if let Some(obj) = req.extra.as_object() {
            for (key, value) in obj {
                body[key] = value.clone();
            }
        }
        // The negotiated format always wins over `extra`.
        body["response_format"] = json!(format);

        let response = self
            .auth(self.http.post(&self.tts_endpoint).json(&body))
            .send()
            .await
            .map_err(|e| SpeechError {
                message: format!("TTS request failed: {}", e),
                retryable: e.is_timeout() || e.is_connect(),
                status: None,
                body: String::new(),
            })?;

        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(SpeechError {
                message: format!("TTS error {} - {}", status, summarize(&text)),
                retryable: is_retryable_status(status.as_u16()),
                status: Some(status.as_u16()),
                body: text,
            });
        }

        if content_type.contains("json") {
            let text = response.text().await.unwrap_or_default();
            return Err(SpeechError {
                message: format!("non-audio response - {}", summarize(&text)),
                retryable: false,
                status: Some(status.as_u16()),
                body: text,
            });
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| SpeechError {
                message: format!("failed to read audio response: {}", e),
                retryable: false,
                status: None,
                body: String::new(),
            })?
            .to_vec();

        let detected = sniff_format(&content_type, &bytes, format);
        Ok((bytes, detected))
    }
}

/// Classify HTTP statuses the same way LunaAI does.
pub fn is_retryable_status(status: u16) -> bool {
    status == 408 || status == 425 || status == 429 || status >= 500
}

/// Determine the audio container from content-type, then magic bytes,
/// falling back to the requested format.
pub fn sniff_format(content_type: &str, bytes: &[u8], requested: &str) -> AudioFormat {
    let ct = content_type.to_lowercase();
    if ct.contains("mpeg") || ct.contains("mp3") {
        return AudioFormat::Mp3;
    }
    if ct.contains("wav") || ct.contains("wave") || ct.contains("x-wav") {
        return AudioFormat::Wav;
    }
    if ct.contains("pcm") {
        return AudioFormat::Pcm;
    }
    if bytes.len() > 12 && bytes.starts_with(b"RIFF") {
        return AudioFormat::Wav;
    }
    if bytes.len() > 3 && (bytes.starts_with(&[0x49, 0x44, 0x33]) // ID3
        || (bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0))
    // MPEG frame sync
    {
        return AudioFormat::Mp3;
    }
    match requested {
        "wav" => AudioFormat::Wav,
        "pcm" => AudioFormat::Pcm,
        _ => AudioFormat::Mp3,
    }
}

/// Decode synthesis output into mono i16 samples.
///
/// Raw PCM is assumed to be 24 kHz mono 16-bit LE (what OpenRouter-style
/// providers return); WAV/MP3 are decoded via rodio (symphonia).
pub fn decode_speech_audio(
    bytes: Vec<u8>,
    format: AudioFormat,
) -> std::result::Result<SpeechAudio, SpeechError> {
    if bytes.is_empty() {
        return Err(SpeechError::new("empty audio response", false));
    }
    if format == AudioFormat::Pcm {
        let samples: Vec<i16> = bytes
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        return Ok(SpeechAudio {
            samples,
            sample_rate: OPENAI_TTS_PCM_SAMPLE_RATE,
        });
    }

    let decoder = {
        use rodio::Source as _;
        let decoder = rodio::Decoder::new(std::io::Cursor::new(bytes))
            .map_err(|e| SpeechError::new(format!("audio decode failed: {}", e), false))?;
        let sample_rate = decoder.sample_rate();
        let channels = decoder.channels() as usize;
        let samples: Vec<f32> = decoder.collect();
        (samples, channels, sample_rate)
    };
    let (samples, channels, sample_rate) = decoder;
    let mono = if channels > 1 {
        stereo_to_mono(&samples, channels)
    } else {
        samples
    };
    let pcm: Vec<i16> = mono
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();
    if pcm.is_empty() {
        return Err(SpeechError::new("decoded audio is empty", false));
    }
    Ok(SpeechAudio {
        samples: pcm,
        sample_rate,
    })
}

/// Keep error bodies short and never echo secrets.
fn summarize(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "<empty response>".to_string();
    }
    if trimmed.len() > 300 {
        format!("{}…", &trimmed[..300])
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_by_content_type() {
        assert_eq!(sniff_format("audio/mpeg", &[], "pcm"), AudioFormat::Mp3);
        assert_eq!(sniff_format("audio/mp3", &[], "pcm"), AudioFormat::Mp3);
        assert_eq!(sniff_format("audio/wav", &[], "mp3"), AudioFormat::Wav);
        assert_eq!(sniff_format("audio/x-wav", &[], "mp3"), AudioFormat::Wav);
        assert_eq!(sniff_format("audio/pcm", &[], "mp3"), AudioFormat::Pcm);
    }

    #[test]
    fn sniff_by_magic_bytes() {
        let mut riff = vec![0u8; 16];
        riff[..4].copy_from_slice(b"RIFF");
        assert_eq!(sniff_format("application/octet-stream", &riff, "mp3"), AudioFormat::Wav);

        let id3 = [0x49, 0x44, 0x33, 0x03, 0x00, 0x00];
        assert_eq!(sniff_format("", &id3, "pcm"), AudioFormat::Mp3);

        let sync = [0xFF, 0xFB, 0x90, 0x00, 0x00];
        assert_eq!(sniff_format("", &sync, "pcm"), AudioFormat::Mp3);

        let raw = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];
        assert_eq!(sniff_format("application/octet-stream", &raw, "pcm"), AudioFormat::Pcm);
        assert_eq!(sniff_format("", &raw, "mp3"), AudioFormat::Mp3);
    }

    #[test]
    fn pcm_decodes_as_24k_mono() {
        let bytes: Vec<u8> = [0x00, 0x00, 0xFF, 0x7F, 0x01, 0x80].to_vec();
        let audio = decode_speech_audio(bytes, AudioFormat::Pcm).unwrap();
        assert_eq!(audio.sample_rate, 24_000);
        assert_eq!(audio.samples, vec![0i16, i16::MAX, i16::MIN + 1]);
    }

    #[test]
    fn wav_decodes_via_rodio() {
        let samples: Vec<i16> = (0..4800).collect::<Vec<_>>();
        let wav = crate::utils::create_wav_buffer(&samples, 24_000).unwrap();
        let audio = decode_speech_audio(wav, AudioFormat::Wav).unwrap();
        assert_eq!(audio.sample_rate, 24_000);
        assert!(!audio.samples.is_empty());
    }

    #[test]
    fn empty_audio_is_error() {
        assert!(decode_speech_audio(Vec::new(), AudioFormat::Wav).is_err());
    }

    #[test]
    fn format_error_detection() {
        let err = SpeechError {
            message: "x".into(),
            retryable: false,
            status: Some(400),
            body: r#"{"error":"unsupported response_format value"}"#.into(),
        };
        assert!(err.is_format_error());

        let other = SpeechError {
            message: "x".into(),
            retryable: false,
            status: Some(400),
            body: r#"{"error":"invalid model"}"#.into(),
        };
        assert!(!other.is_format_error());
    }

    #[test]
    fn retryable_statuses() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(503));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(404));
    }
}

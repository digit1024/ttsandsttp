//! DashScope (Alibaba Cloud Model Studio) API client.
//!
//! Supports:
//! - `qwen3-asr-flash` synchronous speech recognition (base64 Data URL input)
//! - `qwen3-tts-flash` speech synthesis (SSE streaming PCM, plus a
//!   non-streaming URL fallback)

use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use futures::StreamExt;
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::config::ApiConfig;
use crate::secrets::Secrets;

/// Sample rate of the PCM returned by DashScope streaming TTS.
pub const DASHSCOPE_TTS_SAMPLE_RATE: u32 = 24_000;

/// DashScope API client
pub struct DashScopeClient {
    http: Client,
    base_url: String,
    api_key: SecretString,
}

impl std::fmt::Debug for DashScopeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashScopeClient")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl DashScopeClient {
    /// Build a client, resolving the API key via [`Secrets`].
    pub async fn new(api: &ApiConfig) -> Result<Self> {
        let api_key = Secrets::resolve(api).await?;
        let http = Client::builder()
            .timeout(Duration::from_secs(api.timeout_secs.max(5)))
            .build()
            .context("Failed to build HTTP client")?;
        let base_url = api.effective_base_url().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            bail!("DashScope base_url is empty (set [api] base_url or choose a region)");
        }
        Ok(Self {
            http,
            base_url,
            api_key,
        })
    }

    fn generation_url(&self) -> String {
        format!("{}/services/aigc/multimodal-generation/generation", self.base_url)
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(self.api_key.expose_secret())
    }

    /// Transcribe a WAV buffer with `qwen3-asr-flash`.
    ///
    /// `language` is an optional DashScope language hint (e.g. `"en"`, `"zh"`).
    pub async fn transcribe(&self, model: &str, wav: &[u8], language: Option<&str>) -> Result<String> {
        let data_url = format!("data:audio/wav;base64,{}", BASE64.encode(wav));

        let mut parameters = serde_json::Map::new();
        if let Some(lang) = language.filter(|l| !l.is_empty()) {
            parameters.insert("asr_options".to_string(), json!({ "language": lang }));
        }

        let body = json!({
            "model": model,
            "input": {
                "messages": [{
                    "role": "user",
                    "content": [{ "audio": data_url }]
                }]
            },
            "parameters": Value::Object(parameters),
        });

        let response = self
            .auth(self.http.post(self.generation_url()))
            .json(&body)
            .send()
            .await
            .context("DashScope ASR request failed")?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            bail!("DashScope ASR error ({}): {}", status, summarize_error(&text));
        }

        let value: Value = response
            .json()
            .await
            .context("Failed to parse DashScope ASR response")?;

        Ok(extract_asr_text(&value).unwrap_or_default())
    }

    /// Synthesize speech, streaming PCM chunks (i16, 24 kHz mono) to `on_chunk`.
    ///
    /// Returns once the stream finishes or `token` is cancelled.
    pub async fn synthesize_pcm_stream<F>(
        &self,
        model: &str,
        text: &str,
        voice: &str,
        language_type: &str,
        token: &CancellationToken,
        mut on_chunk: F,
    ) -> Result<()>
    where
        F: FnMut(Vec<i16>),
    {
        let body = json!({
            "model": model,
            "input": {
                "text": text,
                "voice": voice,
                "language_type": language_type,
            }
        });

        let request = self
            .auth(self.http.post(self.generation_url()))
            .header("X-DashScope-SSE", "enable")
            .json(&body)
            .send();

        let response = tokio::select! {
            result = request => result.context("DashScope TTS request failed")?,
            _ = token.cancelled() => return Ok(()),
        };

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            bail!("DashScope TTS error ({}): {}", status, summarize_error(&text));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        loop {
            if token.is_cancelled() {
                tracing::debug!("DashScope TTS stream cancelled");
                return Ok(());
            }

            let next = tokio::select! {
                chunk = stream.next() => chunk,
                _ = token.cancelled() => return Ok(()),
            };

            let Some(chunk) = next else {
                break;
            };
            let chunk = chunk.context("DashScope TTS stream error")?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete lines, retaining any partial trailing line.
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim_end_matches('\r').to_string();
                buffer.drain(..=pos);
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data.is_empty() || data == "[DONE]" {
                        continue;
                    }
                    if let Some(pcm) = decode_tts_chunk(data) {
                        on_chunk(pcm);
                    }
                }
            }
        }

        Ok(())
    }

    /// Non-streaming synthesis: returns raw audio bytes from the returned URL.
    pub async fn synthesize_bytes(
        &self,
        model: &str,
        text: &str,
        voice: &str,
        language_type: &str,
    ) -> Result<Vec<u8>> {
        let body = json!({
            "model": model,
            "input": {
                "text": text,
                "voice": voice,
                "language_type": language_type,
            }
        });

        let response = self
            .auth(self.http.post(self.generation_url()))
            .json(&body)
            .send()
            .await
            .context("DashScope TTS request failed")?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            bail!("DashScope TTS error ({}): {}", status, summarize_error(&text));
        }

        let value: Value = response
            .json()
            .await
            .context("Failed to parse DashScope TTS response")?;
        let url = value
            .pointer("/output/audio/url")
            .and_then(|v| v.as_str())
            .context("DashScope TTS response did not include an audio URL")?;

        let audio = self
            .http
            .get(url)
            .send()
            .await
            .context("Failed to download synthesized audio")?;
        let status = audio.status();
        if !status.is_success() {
            bail!("Failed to download synthesized audio ({})", status);
        }
        Ok(audio.bytes().await.context("Failed to read audio bytes")?.to_vec())
    }
}

/// Extract recognized text from any of the DashScope ASR response shapes.
fn extract_asr_text(value: &Value) -> Option<String> {
    // Standard multimodal shape (qwen3-asr-flash)
    if let Some(text) = value
        .pointer("/output/choices/0/message/content/0/text")
        .and_then(|v| v.as_str())
    {
        return Some(text.trim().to_string());
    }
    // Qwen-Audio-3.x / Fun-ASR sync shape
    if let Some(text) = value
        .pointer("/output/output/sentence/text")
        .and_then(|v| v.as_str())
    {
        return Some(text.trim().to_string());
    }
    if let Some(text) = value.pointer("/output/text").and_then(|v| v.as_str()) {
        return Some(text.trim().to_string());
    }
    None
}

/// Decode a streaming TTS SSE JSON payload into PCM i16 samples.
fn decode_tts_chunk(data: &str) -> Option<Vec<i16>> {
    let value: Value = serde_json::from_str(data).ok()?;
    let encoded = value.pointer("/output/audio/data")?.as_str()?;
    let bytes = BASE64.decode(encoded).ok()?;
    Some(
        bytes
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect(),
    )
}

/// Keep error bodies short and never echo secrets.
fn summarize_error(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "<empty response>".to_string();
    }
    let snippet = if trimmed.len() > 400 {
        format!("{}…", &trimmed[..400])
    } else {
        trimmed.to_string()
    };
    // DashScope error bodies are JSON; surface just code/message when possible.
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let code = value.get("code").and_then(|v| v.as_str()).unwrap_or("");
        let message = value.get("message").and_then(|v| v.as_str()).unwrap_or("");
        if !code.is_empty() || !message.is_empty() {
            return format!("{} {}", code, message).trim().to_string();
        }
    }
    snippet
}

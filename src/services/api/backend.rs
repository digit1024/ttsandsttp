//! Cloud backend resolution and dispatch.
//!
//! Maps the configured provider id (catalog preset or user-defined
//! `[providers.<id>]` section) onto a concrete client for each direction,
//! replacing the old hardcoded DashScope branches.

use std::sync::Arc;

use anyhow::{bail, Result};
use serde_json::Value;

use crate::config::{
    language_type_for_code, ApiConfig, ApiRegion, AppConfig, ProviderCatalog, ProviderKind,
    TtsLanguageConfig,
};
use crate::secrets::KeySpec;
use crate::utils::{chunk_wav, merge_transcripts, normalize_language_code};

use super::dashscope::DashScopeClient;
use super::openai::{OpenAiClient, OpenAiClientConfig, SpeechAudio, SpeechError};

/// Everything the STT service needs to run one transcription.
#[derive(Debug, Clone)]
pub struct ResolvedStt {
    pub provider_id: String,
    pub provider_label: String,
    pub model: String,
    /// Effective language hint; empty means auto-detect.
    pub language: String,
    /// Vocabulary hints (already filtered by model support).
    pub hints: Vec<String>,
    pub max_audio_seconds: u32,
    pub extra_params: Value,
}

/// Everything the TTS service needs to run one synthesis.
#[derive(Debug, Clone)]
pub struct ResolvedTts {
    pub provider_id: String,
    pub provider_label: String,
    pub model: String,
    pub voice: String,
    pub instructions: Option<String>,
    pub max_chars: usize,
    pub response_format: String,
    /// DashScope SSE streaming playback (ignored by OpenAI-compatible backends).
    pub stream: bool,
    /// DashScope `language_type` value.
    pub language_type: String,
    pub extra_params: Value,
}

/// A resolved cloud STT backend.
pub enum SttBackend {
    DashScope(Arc<DashScopeClient>),
    OpenAi(Arc<OpenAiClient>),
}

impl SttBackend {
    /// Transcribe a WAV buffer, chunking it to the model's request limit and
    /// merging the transcripts (port of mbdictate's chunked transcription).
    pub async fn transcribe(&self, wav: &[u8], resolved: &ResolvedStt) -> Result<String> {
        let chunks = chunk_wav(wav, resolved.max_audio_seconds, 1.0);
        let mut parts: Vec<String> = Vec::new();
        for chunk in &chunks {
            let language = if resolved.language.trim().is_empty() {
                None
            } else {
                Some(resolved.language.as_str())
            };
            let text = match self {
                SttBackend::DashScope(client) => {
                    client
                        .transcribe(&resolved.model, chunk, language, &resolved.hints)
                        .await?
                }
                SttBackend::OpenAi(client) => {
                    client
                        .transcribe(
                            &resolved.model,
                            chunk,
                            language,
                            &resolved.hints,
                            &resolved.extra_params,
                        )
                        .await?
                }
            };
            if !text.trim().is_empty() {
                parts.push(text);
            }
        }
        Ok(merge_transcripts(&parts))
    }
}

/// A resolved cloud TTS backend.
pub enum TtsBackend {
    DashScope(Arc<DashScopeClient>),
    OpenAi(Arc<OpenAiClient>),
}

impl TtsBackend {
    pub fn as_dashscope(&self) -> Option<&Arc<DashScopeClient>> {
        match self {
            TtsBackend::DashScope(client) => Some(client),
            TtsBackend::OpenAi(_) => None,
        }
    }

    pub fn as_openai(&self) -> Option<&Arc<OpenAiClient>> {
        match self {
            TtsBackend::DashScope(_) => None,
            TtsBackend::OpenAi(client) => Some(client),
        }
    }

    /// Synthesize one text chunk into playable samples (OpenAI-compatible
    /// backends only).
    pub async fn synthesize_chunk(
        &self,
        text: &str,
        resolved: &ResolvedTts,
    ) -> std::result::Result<SpeechAudio, SpeechError> {
        match self {
            TtsBackend::OpenAi(client) => {
                let request = super::openai::SpeechRequest {
                    model: &resolved.model,
                    text,
                    voice: &resolved.voice,
                    instructions: resolved.instructions.as_deref(),
                    preferred_format: &resolved.response_format,
                    extra: &resolved.extra_params,
                };
                client.speech(&request).await
            }
            TtsBackend::DashScope(_) => Err(SpeechError {
                message: "synthesize_chunk is not supported by the DashScope backend".to_string(),
                retryable: false,
                status: None,
                body: String::new(),
            }),
        }
    }
}

/// Provider metadata resolved from catalog + user config.
struct ResolvedProvider {
    id: String,
    label: String,
    kind: ProviderKind,
    stt_endpoint: String,
    tts_endpoint: String,
    timeout_secs: u64,
    key_spec: Option<KeySpec>,
    /// DashScope clients are built from this.
    api_config: ApiConfig,
}

fn resolve_provider(config: &AppConfig, id: &str) -> Result<ResolvedProvider> {
    let catalog = ProviderCatalog::bundled();
    let def = catalog.provider(id);
    let pc = config.provider_config(id);

    let kind = match (def, pc) {
        (Some(def), _) => def.kind,
        (None, Some(pc)) => match pc.kind.as_str() {
            "openai" => ProviderKind::Openai,
            "dashscope" => ProviderKind::Dashscope,
            other => bail!(
                "Provider '{}' is not in the bundled catalog; set kind = \"openai\" or \
                 \"dashscope\" in [providers.{}] (found '{}')",
                id,
                id,
                other
            ),
        },
        (None, None) => bail!(
            "Unknown provider '{}': not in the bundled catalog and no [providers.{}] \
             section in config.toml",
            id,
            id
        ),
    };

    let label = pc
        .map(|p| p.label.clone())
        .filter(|l| !l.is_empty())
        .or_else(|| def.map(|d| d.label.clone()))
        .unwrap_or_else(|| id.to_string());

    let base_url = pc
        .map(|p| p.base_url.clone())
        .filter(|b| !b.trim().is_empty())
        .or_else(|| def.map(|d| d.base_url.clone()))
        .unwrap_or_default();
    let base_url = base_url.trim_end_matches('/').to_string();

    let stt_override = pc.map(|p| p.stt_endpoint.clone()).unwrap_or_default();
    let tts_override = pc.map(|p| p.tts_endpoint.clone()).unwrap_or_default();

    let (stt_endpoint, tts_endpoint) = match kind {
        ProviderKind::Openai => (
            if !stt_override.trim().is_empty() {
                stt_override.trim().to_string()
            } else if !base_url.is_empty() {
                format!("{}/audio/transcriptions", base_url)
            } else {
                String::new()
            },
            if !tts_override.trim().is_empty() {
                tts_override.trim().to_string()
            } else if !base_url.is_empty() {
                format!("{}/audio/speech", base_url)
            } else {
                String::new()
            },
        ),
        ProviderKind::Dashscope => (stt_override.trim().to_string(), tts_override.trim().to_string()),
    };

    let timeout_secs = pc.map(|p| p.timeout_secs).filter(|t| *t > 0).unwrap_or(60);
    let needs_key = pc.map(|p| p.needs_key()).unwrap_or(true);

    let key_spec = if needs_key {
        Some(match kind {
            ProviderKind::Dashscope => KeySpec::dashscope(&dashscope_api(config, id)),
            ProviderKind::Openai => KeySpec::provider(
                id,
                &label,
                def.map(|d| d.env.as_str()).unwrap_or(""),
                pc.map(|p| p.api_key_env.as_str()).unwrap_or(""),
                pc.map(|p| p.key_file.as_str()).unwrap_or(""),
                pc.map(|p| p.key_source).unwrap_or_default(),
            ),
        })
    } else {
        None
    };

    Ok(ResolvedProvider {
        id: id.to_string(),
        label,
        kind,
        stt_endpoint,
        tts_endpoint,
        timeout_secs,
        key_spec,
        api_config: dashscope_api(config, id),
    })
}

/// ApiConfig for a dashscope-kind provider (preset or custom).
fn dashscope_api(config: &AppConfig, id: &str) -> ApiConfig {
    if id == "dashscope" {
        return config.dashscope_api();
    }
    let mut api = ApiConfig::default();
    if let Some(pc) = config.provider_config(id) {
        api.region = pc.region.unwrap_or(if pc.base_url.trim().is_empty() {
            ApiRegion::Singapore
        } else {
            ApiRegion::Custom
        });
        api.base_url = pc.base_url.clone();
        api.api_key_env = pc.api_key_env.clone();
        api.key_source = pc.key_source;
        api.key_file = pc.key_file.clone();
        if pc.timeout_secs > 0 {
            api.timeout_secs = pc.timeout_secs;
        }
    }
    api
}

async fn build_client(provider: &ResolvedProvider) -> Result<CloudClient> {
    match provider.kind {
        ProviderKind::Openai => {
            if provider.stt_endpoint.is_empty() && provider.tts_endpoint.is_empty() {
                bail!(
                    "Provider '{}' needs a base_url or endpoint overrides in [providers.{}]",
                    provider.id,
                    provider.id
                );
            }
            let client = OpenAiClient::new(OpenAiClientConfig {
                stt_endpoint: provider.stt_endpoint.clone(),
                tts_endpoint: provider.tts_endpoint.clone(),
                key_spec: provider.key_spec.clone(),
                timeout_secs: provider.timeout_secs,
            })
            .await?;
            Ok(CloudClient::OpenAi(Arc::new(client)))
        }
        ProviderKind::Dashscope => {
            let client = DashScopeClient::new_with_endpoints(
                &provider.api_config,
                Some(provider.stt_endpoint.clone()).filter(|s| !s.is_empty()),
                Some(provider.tts_endpoint.clone()).filter(|s| !s.is_empty()),
            )
            .await?;
            Ok(CloudClient::DashScope(Arc::new(client)))
        }
    }
}

enum CloudClient {
    DashScope(Arc<DashScopeClient>),
    OpenAi(Arc<OpenAiClient>),
}

/// Build the STT backend for the configured provider.
/// Returns `None` when the local provider is selected.
///
/// `request_lang` is the per-call language (e.g. from the DBus method); a
/// matching `[stt.<lang>]` override section is merged over the global
/// `[stt]` config, and the configured fixed language hint wins over the
/// request language.
pub async fn build_stt_backend(
    config: &AppConfig,
    request_lang: &str,
) -> Result<Option<(SttBackend, ResolvedStt)>> {
    let stt = config.stt.for_language(request_lang);
    if stt.is_local() {
        return Ok(None);
    }
    let id = stt.provider.trim().to_string();
    let catalog = ProviderCatalog::bundled();
    let provider = resolve_provider(config, &id)?;

    let model = stt.cloud_model();
    if model.is_empty() {
        bail!("STT provider '{}' has no model configured", id);
    }

    let model_def = catalog.stt_model(&id, &model);
    let max_audio_seconds = model_def.map(|m| m.max_audio_seconds).unwrap_or(60);
    let requires_language = model_def.map(|m| m.requires_language).unwrap_or(false);
    let supports_hints = model_def.map(|m| m.supports_vocabulary_hint).unwrap_or(true);
    if model_def.is_none() {
        tracing::warn!(
            "STT model '{}' for provider '{}' is not in the bundled catalog; passing it through",
            model,
            id
        );
    }

    let fixed = stt.effective_language();
    let language = if fixed.is_empty() {
        normalize_language_code(request_lang)
    } else {
        fixed
    };
    if requires_language && language.trim().is_empty() {
        bail!(
            "STT model '{}' requires an explicit language; set [stt] language or pass one \
             to the DBus call",
            model
        );
    }

    let hints = if supports_hints {
        stt.hints.clone()
    } else if !stt.hints.is_empty() {
        tracing::warn!(
            "STT model '{}' does not support vocabulary hints; ignoring them",
            model
        );
        Vec::new()
    } else {
        Vec::new()
    };

    let backend = match build_client(&provider).await? {
        CloudClient::DashScope(client) => SttBackend::DashScope(client),
        CloudClient::OpenAi(client) => SttBackend::OpenAi(client),
    };

    Ok(Some((
        backend,
        ResolvedStt {
            provider_id: id,
            provider_label: provider.label,
            model,
            language,
            hints,
            max_audio_seconds,
            extra_params: stt.extra_params.clone(),
        },
    )))
}

/// Build the TTS backend for one language's configuration.
/// Returns `None` when the local provider is selected.
pub async fn build_tts_backend(
    config: &AppConfig,
    lang_code: &str,
    lang_cfg: &TtsLanguageConfig,
) -> Result<Option<(TtsBackend, ResolvedTts)>> {
    if lang_cfg.is_local() {
        return Ok(None);
    }
    let id = lang_cfg.provider.trim().to_string();
    let catalog = ProviderCatalog::bundled();
    let provider = resolve_provider(config, &id)?;

    let model = lang_cfg.cloud_model();
    if model.is_empty() {
        bail!("TTS provider '{}' for language '{}' has no model configured", id, lang_code);
    }

    let model_def = catalog.tts_model(&id, &model);
    if model_def.is_none() {
        tracing::warn!(
            "TTS model '{}' for provider '{}' is not in the bundled catalog; passing it through",
            model,
            id
        );
    }
    let max_chars = model_def.map(|m| m.max_chars).unwrap_or(600).max(1);
    let response_format = model_def
        .map(|m| m.response_format.clone())
        .unwrap_or_else(|| "mp3".to_string());

    let voice = {
        let configured = lang_cfg.voice.trim();
        if !configured.is_empty() {
            configured.to_string()
        } else if let Some(def) = model_def {
            if !def.default_voice.is_empty() {
                def.default_voice.clone()
            } else {
                def.voices.first().cloned().unwrap_or_default()
            }
        } else if provider.kind == ProviderKind::Dashscope {
            "Cherry".to_string()
        } else {
            String::new()
        }
    };

    let instructions = {
        let trimmed = lang_cfg.instructions.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };

    let language_type = if lang_cfg.language_type.trim().is_empty() {
        language_type_for_code(&normalize_language_code(lang_code)).to_string()
    } else {
        lang_cfg.language_type.clone()
    };

    let backend = match build_client(&provider).await? {
        CloudClient::DashScope(client) => TtsBackend::DashScope(client),
        CloudClient::OpenAi(client) => TtsBackend::OpenAi(client),
    };

    Ok(Some((
        backend,
        ResolvedTts {
            provider_id: id,
            provider_label: provider.label,
            model,
            voice,
            instructions,
            max_chars,
            response_format,
            stream: lang_cfg.stream,
            language_type,
            extra_params: lang_cfg.extra_params.clone(),
        },
    )))
}

/// Convenience for tests and CLI: does this provider id resolve at all?
pub fn provider_exists(config: &AppConfig, id: &str) -> bool {
    ProviderCatalog::bundled().provider(id).is_some() || config.provider_config(id).is_some()
}

/// Build the API key spec for a provider (`None` when it needs no key).
pub fn provider_key_spec(config: &AppConfig, id: &str) -> Result<Option<KeySpec>> {
    resolve_provider(config, id).map(|p| p.key_spec)
}

/// Resolve a provider's label for CLI output.
pub fn provider_label(config: &AppConfig, id: &str) -> String {
    resolve_provider(config, id)
        .map(|p| p.label)
        .unwrap_or_else(|_| id.to_string())
}

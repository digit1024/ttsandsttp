//! A/B benchmark harness: local Whisper vs the configured cloud STT provider,
//! plus a cloud TTS latency benchmark.

use anyhow::{bail, Context, Result};
use rodio::Source;
use std::time::Instant;

use crate::config::{AppConfig, ConfigLoader, ConfigValidator};
use crate::services::api::{build_stt_backend, build_tts_backend, TtsBackend};
use crate::services::stt::{resample_linear, stereo_to_mono, TARGET_SAMPLE_RATE};
use crate::services::ModelManager;
use crate::utils::{chunk_text_for_tts, create_wav_buffer, DEFAULT_TTS_FIRST_CHUNK_LENGTH};

/// Transcribe one WAV file with the configured backend (and the local model
/// for comparison when a cloud provider is configured).
///
/// `provider`/`model` optionally override the configured STT selection.
pub async fn bench_stt(
    wav_path: &str,
    lang: &str,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<()> {
    let mut config = ConfigLoader::load_or_create().context("Failed to load config")?;
    if provider.is_some() || model.is_some() {
        // Explicit CLI selection wins over per-language [stt.<lang>] overrides.
        config.stt.languages.clear();
    }
    if let Some(p) = provider {
        config.stt.provider = p.to_string();
    }
    if let Some(m) = model {
        config.stt.model = m.to_string();
        config.stt.api_model = String::new();
    }

    let samples = decode_wav_mono_16k(wav_path)?;
    let duration = samples.len() as f64 / TARGET_SAMPLE_RATE as f64;

    println!(
        "Audio: {} ({:.2}s @ {}Hz, {} samples)",
        wav_path,
        duration,
        TARGET_SAMPLE_RATE,
        samples.len()
    );
    println!();

    let effective = config.stt.for_language(lang);
    if !effective.is_local() {
        run_cloud(&config, &samples, lang, duration).await?;
        println!();
        if let Err(e) = run_local(&config, &samples, lang, duration) {
            println!("Local Whisper: skipped ({:#})", e);
        }
    } else {
        run_local(&config, &samples, lang, duration)?;
        println!();
        println!(
            "(provider is 'local'; set [stt] provider = \"groq\" (or use --provider) to compare cloud)"
        );
    }

    Ok(())
}

async fn run_cloud(config: &AppConfig, samples: &[f32], lang: &str, duration: f64) -> Result<()> {
    let (backend, resolved) = build_stt_backend(config, lang)
        .await?
        .context("Cloud STT provider is not configured")?;

    let pcm: Vec<i16> = samples
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();
    let wav = create_wav_buffer(&pcm, TARGET_SAMPLE_RATE)?;

    let start = Instant::now();
    let text = backend.transcribe(&wav, &resolved).await?;
    let elapsed = start.elapsed().as_secs_f64();

    println!(
        "{} {} (provider '{}'):",
        resolved.provider_label, resolved.model, resolved.provider_id
    );
    println!("  text: {}", display(&text));
    println!(
        "  time: {:.2}s (RTF {:.2})",
        elapsed,
        rtf(elapsed, duration)
    );
    Ok(())
}

fn run_local(config: &AppConfig, samples: &[f32], lang: &str, duration: f64) -> Result<()> {
    let registry = ConfigValidator::get_registry()?;
    let model_id = config.stt.for_language(lang).local_model();
    let model_info = registry
        .get_stt_model(&model_id)
        .with_context(|| format!("STT model '{}' not found in registry", model_id))?;
    let manager = ModelManager::new()?;
    let dir = manager.get_stt_model_path(&model_id);

    let find = |needle: &str| -> Result<String> {
        model_info
            .required_files
            .iter()
            .find(|f| f.contains(needle) && f.ends_with(".onnx"))
            .map(|f| dir.join(f).to_string_lossy().to_string())
            .with_context(|| format!("No {} file for model {}", needle, model_id))
    };
    let tokens = model_info
        .required_files
        .iter()
        .find(|f| f.ends_with("tokens.txt"))
        .map(|f| dir.join(f).to_string_lossy().to_string())
        .with_context(|| format!("No tokens file for model {}", model_id))?;

    let language = if model_info.language_code == "multilingual" {
        lang.to_string()
    } else {
        model_info.language_code.clone()
    };

    let cfg = sherpa_rs::whisper::WhisperConfig {
        encoder: find("encoder")?,
        decoder: find("decoder")?,
        tokens,
        language,
        bpe_vocab: None,
        tail_paddings: None,
        provider: None,
        num_threads: Some(1),
        debug: false,
    };

    let mut recognizer = sherpa_rs::whisper::WhisperRecognizer::new(cfg)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let start = Instant::now();
    let text = recognizer.transcribe(TARGET_SAMPLE_RATE, samples).text;
    let elapsed = start.elapsed().as_secs_f64();

    println!("Local Whisper {}:", model_id);
    println!("  text: {}", display(&text));
    println!(
        "  time: {:.2}s (RTF {:.2})",
        elapsed,
        rtf(elapsed, duration)
    );
    Ok(())
}

fn display(text: &str) -> &str {
    if text.is_empty() {
        "(empty)"
    } else {
        text
    }
}

/// Synthesize `text` with a cloud TTS provider, reporting latency and
/// optionally writing the audio to a WAV file.
pub async fn bench_tts(
    text: &str,
    lang: &str,
    provider: Option<&str>,
    model: Option<&str>,
    voice: Option<&str>,
    out_path: Option<&str>,
) -> Result<()> {
    let mut config = ConfigLoader::load_or_create().context("Failed to load config")?;
    let lang_code = crate::utils::normalize_language_code(lang);

    // Resolve like the daemon: own [tts.<lang>] section, else global fallback.
    let mut lang_cfg = config
        .tts
        .effective_language_config(&lang_code)
        .unwrap_or_default();
    if let Some(p) = provider {
        lang_cfg.provider = p.to_string();
    }
    if let Some(m) = model {
        lang_cfg.model = m.to_string();
        lang_cfg.api_model = String::new();
    }
    if let Some(v) = voice {
        lang_cfg.voice = v.to_string();
    }
    if lang_cfg.is_local() {
        bail!(
            "bench-tts needs a cloud provider; set a global [tts] provider, a [tts.{}] \
             section, or pass --provider",
            lang_code
        );
    }
    // Silence unused-mut warnings when config is only read below.
    let _ = &mut config;
    let (backend, resolved) = build_tts_backend(&config, &lang_code, &lang_cfg)
        .await?
        .context("Cloud TTS provider is not configured")?;

    println!(
        "TTS: {} '{}' voice '{}' (provider '{}')",
        resolved.provider_label, resolved.model, resolved.voice, resolved.provider_id
    );

    let chunks = chunk_text_for_tts(text, resolved.max_chars, Some(DEFAULT_TTS_FIRST_CHUNK_LENGTH));
    if chunks.is_empty() {
        bail!("Nothing to synthesize");
    }

    let start = Instant::now();
    let mut all_samples: Vec<i16> = Vec::new();
    let mut sample_rate = 0u32;

    match &backend {
        TtsBackend::OpenAi(_) => {
            for (i, chunk) in chunks.iter().enumerate() {
                let chunk_start = Instant::now();
                let audio = backend
                    .synthesize_chunk(chunk, &resolved)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                println!(
                    "  chunk {}/{} ({} chars): {:.2}s",
                    i + 1,
                    chunks.len(),
                    chunk.chars().count(),
                    chunk_start.elapsed().as_secs_f64()
                );
                sample_rate = audio.sample_rate;
                all_samples.extend_from_slice(&audio.samples);
            }
        }
        TtsBackend::DashScope(client) => {
            for (i, chunk) in chunks.iter().enumerate() {
                let chunk_start = Instant::now();
                let bytes = client
                    .synthesize_bytes(
                        &resolved.model,
                        chunk,
                        &resolved.voice,
                        &resolved.language_type,
                    )
                    .await?;
                println!(
                    "  chunk {}/{} ({} chars): {:.2}s, {} bytes",
                    i + 1,
                    chunks.len(),
                    chunk.chars().count(),
                    chunk_start.elapsed().as_secs_f64(),
                    bytes.len()
                );
                let audio = crate::services::api::decode_speech_audio(
                    bytes,
                    crate::services::api::AudioFormat::Wav,
                )
                .map_err(|e| anyhow::anyhow!("{}", e))?;
                sample_rate = audio.sample_rate;
                all_samples.extend_from_slice(&audio.samples);
            }
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let audio_secs = if sample_rate > 0 {
        all_samples.len() as f64 / sample_rate as f64
    } else {
        0.0
    };
    println!(
        "  total: {:.2}s for {:.2}s of audio @ {}Hz (RTF {:.2})",
        elapsed,
        audio_secs,
        sample_rate,
        if audio_secs > 0.0 { elapsed / audio_secs } else { 0.0 }
    );

    if let Some(path) = out_path {
        if sample_rate == 0 || all_samples.is_empty() {
            bail!("No audio was synthesized; nothing to write");
        }
        let wav = create_wav_buffer(&all_samples, sample_rate)?;
        std::fs::write(path, &wav)
            .with_context(|| format!("Failed to write {}", path))?;
        println!("  wrote {} ({} bytes)", path, wav.len());
    }
    Ok(())
}

fn rtf(elapsed: f64, duration: f64) -> f64 {
    if duration > 0.0 {
        elapsed / duration
    } else {
        0.0
    }
}

/// Decode any audio file rodio understands into mono f32 at 16 kHz.
fn decode_wav_mono_16k(path: &str) -> Result<Vec<f32>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open audio file: {}", path))?;
    let decoder = rodio::Decoder::new(std::io::BufReader::new(file))
        .map_err(|e| anyhow::anyhow!("Failed to decode audio: {}", e))?;
    let channels = decoder.channels() as usize;
    let rate = decoder.sample_rate();
    let samples: Vec<f32> = decoder.collect();
    let mono = stereo_to_mono(&samples, channels);
    Ok(if rate == TARGET_SAMPLE_RATE {
        mono
    } else {
        resample_linear(&mono, rate, TARGET_SAMPLE_RATE)
    })
}

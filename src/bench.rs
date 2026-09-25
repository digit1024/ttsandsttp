//! A/B benchmark harness: local Whisper vs DashScope Qwen ASR.

use anyhow::{Context, Result};
use rodio::Source;
use std::time::Instant;

use crate::config::{AppConfig, ConfigLoader, ConfigValidator};
use crate::services::api::DashScopeClient;
use crate::services::stt::{resample_linear, stereo_to_mono, TARGET_SAMPLE_RATE};
use crate::services::ModelManager;
use crate::utils::create_wav_buffer;

/// Transcribe one WAV file with the configured backend (and the local model
/// for comparison when the provider is DashScope).
pub async fn bench_stt(wav_path: &str, lang: &str) -> Result<()> {
    let config = ConfigLoader::load_or_create().context("Failed to load config")?;
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

    if config.stt.provider.is_dashscope() {
        run_cloud(&config, &samples, lang, duration).await?;
        println!();
        if let Err(e) = run_local(&config, &samples, lang, duration) {
            println!("Local Whisper: skipped ({:#})", e);
        }
    } else {
        run_local(&config, &samples, lang, duration)?;
        println!();
        println!("(provider is 'local'; set [stt] provider = \"dashscope\" to compare cloud)");
    }

    Ok(())
}

async fn run_cloud(config: &AppConfig, samples: &[f32], lang: &str, duration: f64) -> Result<()> {
    let client = DashScopeClient::new(&config.api)
        .await
        .context("DashScope client unavailable")?;
    let language = if config.stt.api_language.trim().is_empty() {
        lang
    } else {
        config.stt.api_language.as_str()
    };

    let pcm: Vec<i16> = samples
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();
    let wav = create_wav_buffer(&pcm, TARGET_SAMPLE_RATE)?;

    let start = Instant::now();
    let text = client
        .transcribe(&config.stt.api_model, &wav, Some(language))
        .await?;
    let elapsed = start.elapsed().as_secs_f64();

    println!("DashScope {}:", config.stt.api_model);
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
    let model_info = registry
        .get_stt_model(&config.stt.model_id)
        .with_context(|| format!("STT model '{}' not found in registry", config.stt.model_id))?;
    let manager = ModelManager::new()?;
    let dir = manager.get_stt_model_path(&config.stt.model_id);

    let find = |needle: &str| -> Result<String> {
        model_info
            .required_files
            .iter()
            .find(|f| f.contains(needle) && f.ends_with(".onnx"))
            .map(|f| dir.join(f).to_string_lossy().to_string())
            .with_context(|| format!("No {} file for model {}", needle, config.stt.model_id))
    };
    let tokens = model_info
        .required_files
        .iter()
        .find(|f| f.ends_with("tokens.txt"))
        .map(|f| dir.join(f).to_string_lossy().to_string())
        .with_context(|| format!("No tokens file for model {}", config.stt.model_id))?;

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

    println!("Local Whisper {}:", config.stt.model_id);
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

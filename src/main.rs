use anyhow::{bail, Context, Result};
use ttsandsttp::{
    bench, config::{is_known_voice, load_voices}, secrets::Secrets, ConfigLoader,
    ConfigModelDownloader, ConfigValidator, TtsSttService,
};

/// TTSandSTTP Daemon - DBus service for Text-to-Speech and Speech-to-Text
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("daemon");

    match command {
        "daemon" | "serve" => run_daemon().await,
        "bench-stt" => cmd_bench_stt(&args[2..]).await,
        "set-key" => cmd_set_key().await,
        "del-key" | "delete-key" => cmd_del_key().await,
        "voices" => cmd_voices(&args[2..]),
        "-h" | "--help" | "help" => {
            print_help();
            Ok(())
        }
        other => bail!("Unknown command '{}'. Run `ttsandsttp --help`.", other),
    }
}

/// Run the DBus daemon (default command).
async fn run_daemon() -> Result<()> {
    tracing::info!("Starting DBus daemon service...");

    // Load and validate configuration
    tracing::info!("Loading configuration...");
    let config = ConfigLoader::load_or_create()
        .context("Failed to load or create configuration")?;

    tracing::info!("Configuration loaded from: {}", ConfigLoader::config_path()?.display());

    // Validate configuration
    tracing::info!("Validating configuration...");
    ConfigValidator::validate(&config)
        .context("Configuration validation failed")?;
    tracing::info!("Configuration is valid");

    // Warn (non-fatally) about unknown cloud voices
    warn_unknown_voices(&config);

    // Download required models
    tracing::info!("Checking and downloading required models...");
    ConfigModelDownloader::download_required_models(&config).await
        .context("Failed to download required models")?;
    tracing::info!("All required models are ready");

    let service = TtsSttService::new()?;

    // Preload models
    tracing::info!("Preloading models...");
    service.preload_models().await?;

    // Start serving DBus requests
    service.serve().await?;

    Ok(())
}

fn warn_unknown_voices(config: &ttsandsttp::AppConfig) {
    for (lang, cfg) in &config.tts.languages {
        if cfg.provider.is_dashscope()
            && !cfg.voice.trim().is_empty()
            && !is_known_voice(&cfg.voice)
        {
            tracing::warn!(
                "TTS voice '{}' for language '{}' is not in the bundled list; \
                 passing it through to DashScope verbatim",
                cfg.voice,
                lang
            );
        }
    }
}

/// Store the DashScope API key in the Secret Service keyring.
async fn cmd_set_key() -> Result<()> {
    let config = ConfigLoader::load_or_create().context("Failed to load config")?;
    let key = rpassword::prompt_password(format!(
        "DashScope API key for region '{}': ",
        config.api.region.slug()
    ))
    .context("Failed to read API key")?;

    Secrets::store(&config.api, &key).await?;
    println!(
        "Stored DashScope API key for region '{}' in the Secret Service keyring.",
        config.api.region.slug()
    );
    Ok(())
}

/// Remove the DashScope API key from the Secret Service keyring.
async fn cmd_del_key() -> Result<()> {
    let config = ConfigLoader::load_or_create().context("Failed to load config")?;
    Secrets::delete(&config.api).await?;
    println!(
        "Removed DashScope API key for region '{}' from the keyring.",
        config.api.region.slug()
    );
    Ok(())
}

/// List bundled Qwen-TTS voices.
fn cmd_voices(args: &[String]) -> Result<()> {
    let mut lang: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lang" | "-l" => {
                i += 1;
                lang = args.get(i).cloned();
            }
            other => bail!("Unexpected argument '{}' for `voices`", other),
        }
        i += 1;
    }

    let voices = load_voices();
    if voices.is_empty() {
        bail!("No bundled voices found");
    }

    match &lang {
        Some(l) => println!("Qwen-TTS voices (language '{}'):\n", l),
        None => println!("Qwen-TTS voices:\n"),
    }
    for v in voices {
        println!("  {:<14} {:<7} {}", v.id, v.gender, v.description);
    }
    println!();
    println!("Select one with `[tts.<lang>] voice = \"...\"` (region-scoped; any string is accepted).");
    Ok(())
}

/// A/B benchmark a WAV file: local Whisper vs DashScope Qwen ASR.
async fn cmd_bench_stt(args: &[String]) -> Result<()> {
    let mut wav: Option<String> = None;
    let mut lang = "en".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lang" | "-l" => {
                i += 1;
                lang = args.get(i).cloned().unwrap_or_else(|| "en".to_string());
            }
            other if wav.is_none() => wav = Some(other.to_string()),
            other => bail!("Unexpected argument '{}' for `bench-stt`", other),
        }
        i += 1;
    }

    let wav = wav.context("Usage: ttsandsttp bench-stt <wav> [--lang en]")?;
    bench::bench_stt(&wav, &lang).await
}

fn print_help() {
    println!(
        "ttsandsttp - Text-to-Speech and Speech-to-Text service\n\n\
         Usage: ttsandsttp [COMMAND]\n\n\
         Commands:\n\
           daemon              Run the DBus daemon (default)\n\
           bench-stt <wav>     A/B test local Whisper vs DashScope ASR\n\
                               [--lang en]\n\
           set-key             Store the DashScope API key in the keyring\n\
           del-key             Remove the DashScope API key from the keyring\n\
           voices [--lang en]  List available Qwen-TTS voices\n\
           help                Show this help\n"
    );
}

use anyhow::{bail, Context, Result};
use ttsandsttp::{
    bench,
    config::{is_known_voice, load_voices, ProviderCatalog},
    secrets::Secrets,
    services::api::{provider_exists, provider_key_spec, provider_label},
    ConfigLoader, ConfigModelDownloader, ConfigValidator, TtsSttService,
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
        "bench-tts" => cmd_bench_tts(&args[2..]).await,
        "set-key" => cmd_set_key(&args[2..]).await,
        "del-key" | "delete-key" => cmd_del_key(&args[2..]).await,
        "providers" => cmd_providers(),
        "models" => cmd_models(&args[2..]),
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
    let catalog = ProviderCatalog::bundled();
    let check = |provider: &str, model: &str, voice: &str, lang: &str| {
        if voice.is_empty() {
            return;
        }
        if let Some(def) = catalog.tts_model(provider, model) {
            if !def.voices.is_empty() && !def.voices.iter().any(|v| v.eq_ignore_ascii_case(voice)) {
                tracing::warn!(
                    "TTS voice '{}' for language '{}' is not in the catalog list for '{}' \
                     (provider '{}'); passing it through verbatim",
                    voice,
                    lang,
                    model,
                    provider
                );
            }
        } else if provider == "dashscope" && !is_known_voice(voice) {
            tracing::warn!(
                "TTS voice '{}' for language '{}' is not in the bundled list; \
                 passing it through to DashScope verbatim",
                voice,
                lang
            );
        }
    };

    if let Some(fallback) = config.tts.global_fallback() {
        let model = fallback.cloud_model();
        check(&fallback.provider, &model, fallback.voice.trim(), "<fallback>");
    }
    for (lang, cfg) in &config.tts.languages {
        if cfg.is_local() {
            continue;
        }
        let model = cfg.cloud_model();
        check(&cfg.provider, &model, cfg.voice.trim(), lang);
    }
}

/// Store a provider's API key in the Secret Service keyring.
async fn cmd_set_key(args: &[String]) -> Result<()> {
    let config = ConfigLoader::load_or_create().context("Failed to load config")?;
    let provider = args.first().cloned().unwrap_or_else(|| "dashscope".to_string());
    if !provider_exists(&config, &provider) {
        bail!(
            "Unknown provider '{}'. Run `ttsandsttp providers` to list available providers.",
            provider
        );
    }

    let Some(spec) = provider_key_spec(&config, &provider)
        .with_context(|| format!("Failed to resolve key storage for '{}'", provider))?
    else {
        println!("Provider '{}' is configured to work without an API key.", provider);
        return Ok(());
    };

    let key = rpassword::prompt_password(format!("{} API key: ", spec.label))
        .context("Failed to read API key")?;

    Secrets::store_spec(&spec, &key).await?;
    println!(
        "Stored {} API key (provider '{}') in the Secret Service keyring.",
        spec.label, provider
    );
    Ok(())
}

/// Remove a provider's API key from the Secret Service keyring.
async fn cmd_del_key(args: &[String]) -> Result<()> {
    let config = ConfigLoader::load_or_create().context("Failed to load config")?;
    let provider = args.first().cloned().unwrap_or_else(|| "dashscope".to_string());
    if !provider_exists(&config, &provider) {
        bail!(
            "Unknown provider '{}'. Run `ttsandsttp providers` to list available providers.",
            provider
        );
    }

    let Some(spec) = provider_key_spec(&config, &provider)
        .with_context(|| format!("Failed to resolve key storage for '{}'", provider))?
    else {
        println!("Provider '{}' does not use an API key.", provider);
        return Ok(());
    };

    Secrets::delete_spec(&spec).await?;
    println!(
        "Removed {} API key (provider '{}') from the keyring.",
        spec.label, provider
    );
    Ok(())
}

/// List all known providers (catalog presets + user-defined).
fn cmd_providers() -> Result<()> {
    let config = ConfigLoader::load_or_create().context("Failed to load config")?;
    let catalog = ProviderCatalog::bundled();

    println!("Providers:\n");
    let mut ids: Vec<String> = catalog.providers.keys().cloned().collect();
    for id in config.providers.keys() {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    ids.sort();

    for id in ids {
        let label = provider_label(&config, &id);
        let (kind, caps) = match catalog.provider(&id) {
            Some(def) => {
                let stt = def.capabilities.stt.map(|_| "STT").unwrap_or("-");
                let tts = def.capabilities.tts.map(|_| "TTS").unwrap_or("-");
                (
                    match def.kind {
                        ttsandsttp::config::ProviderKind::Openai => "openai",
                        ttsandsttp::config::ProviderKind::Dashscope => "dashscope",
                    },
                    format!("{}/{}", stt, tts),
                )
            }
            None => match config.provider_config(&id) {
                Some(pc) => (pc.kind.as_str(), "custom".to_string()),
                None => ("?", "?".to_string()),
            },
        };
        let configured = if config.provider_config(&id).is_some() {
            " [configured]"
        } else {
            ""
        };
        println!("  {:<18} {:<10} {:<12} {}{}", id, kind, caps, label, configured);
    }
    println!();
    println!("Select one with `[stt] provider = \"...\"` or `[tts.<lang>] provider = \"...\"`.");
    println!("Store its key with `ttsandsttp set-key <provider>`.");
    Ok(())
}

/// List catalog models for STT and/or TTS.
fn cmd_models(args: &[String]) -> Result<()> {
    let mut cap: Option<String> = None;
    let mut provider: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cap" | "-c" => {
                i += 1;
                cap = args.get(i).cloned();
            }
            "--provider" | "-p" => {
                i += 1;
                provider = args.get(i).cloned();
            }
            other => bail!("Unexpected argument '{}' for `models`", other),
        }
        i += 1;
    }
    let cap = cap.unwrap_or_else(|| "all".to_string());
    if !matches!(cap.as_str(), "stt" | "tts" | "all") {
        bail!("--cap must be 'stt', 'tts' or 'all'");
    }

    let catalog = ProviderCatalog::bundled();
    let matches = |p: &str| provider.as_deref().map(|want| want == p).unwrap_or(true);

    if cap == "stt" || cap == "all" {
        println!("STT models:\n");
        for m in &catalog.stt_models {
            if !matches(&m.provider) {
                continue;
            }
            let mut flags = Vec::new();
            if m.supports_vocabulary_hint {
                flags.push("hints");
            }
            if m.requires_language {
                flags.push("explicit language");
            }
            println!(
                "  {:<14} {:<42} max {:>3}s  {}",
                m.provider,
                if m.model.is_empty() { "<custom>" } else { &m.model },
                m.max_audio_seconds,
                flags.join(", ")
            );
        }
        println!();
    }
    if cap == "tts" || cap == "all" {
        println!("TTS models:\n");
        for m in &catalog.tts_models {
            if !matches(&m.provider) {
                continue;
            }
            let instr = if m.supports_instructions { ", instructions" } else { "" };
            println!(
                "  {:<14} {:<42} max {:>4} chars, {:>3} voices, default '{}'{}{}",
                m.provider,
                if m.model.is_empty() { "<custom>" } else { &m.model },
                m.max_chars,
                m.voices.len(),
                m.default_voice,
                if m.response_format == "wav" { ", wav" } else { "" },
                instr
            );
        }
        println!();
    }
    println!("Select with `provider` + `model` in [stt] or [tts.<lang>].");
    Ok(())
}

/// List available TTS voices.
fn cmd_voices(args: &[String]) -> Result<()> {
    let mut lang: Option<String> = None;
    let mut provider: Option<String> = None;
    let mut model: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lang" | "-l" => {
                i += 1;
                lang = args.get(i).cloned();
            }
            "--provider" | "-p" => {
                i += 1;
                provider = args.get(i).cloned();
            }
            "--model" | "-m" => {
                i += 1;
                model = args.get(i).cloned();
            }
            other => bail!("Unexpected argument '{}' for `voices`", other),
        }
        i += 1;
    }

    let catalog = ProviderCatalog::bundled();

    // Catalog voices for a specific provider/model.
    if let Some(provider_id) = &provider {
        let models: Vec<_> = match &model {
            Some(m) => catalog
                .tts_model(provider_id, m)
                .map(|def| vec![def])
                .unwrap_or_default(),
            None => catalog.tts_models_for(provider_id),
        };
        if models.is_empty() {
            bail!(
                "No TTS model '{}' found for provider '{}'. Run `ttsandsttp models --cap tts --provider {}`.",
                model.as_deref().unwrap_or("<any>"),
                provider_id,
                provider_id
            );
        }
        for def in models {
            println!("{} '{}' voices:", provider_id, def.model);
            for voice in &def.voices {
                let marker = if *voice == def.default_voice { " (default)" } else { "" };
                println!("  {}{}", voice, marker);
            }
            println!();
        }
        println!("Select one with `[tts.<lang>] voice = \"...\"` (any string is accepted).");
        return Ok(());
    }

    // Legacy default: bundled Qwen-TTS voices.
    let voices = load_voices();
    if voices.is_empty() {
        bail!("No bundled voices found");
    }

    match &lang {
        Some(l) => println!("Qwen-TTS voices (language '{}'):\n", l),
        None => println!("Qwen-TTS voices (provider 'dashscope'):\n"),
    }
    for v in voices {
        println!("  {:<14} {:<7} {}", v.id, v.gender, v.description);
    }
    println!();
    println!("Select one with `[tts.<lang>] voice = \"...\"` (any string is accepted).");
    println!("For other providers: `ttsandsttp voices --provider <id> [--model <model>]`.");
    Ok(())
}

/// A/B benchmark a WAV file: local Whisper vs a cloud STT provider.
async fn cmd_bench_stt(args: &[String]) -> Result<()> {
    let mut wav: Option<String> = None;
    let mut lang = "en".to_string();
    let mut provider: Option<String> = None;
    let mut model: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lang" | "-l" => {
                i += 1;
                lang = args.get(i).cloned().unwrap_or_else(|| "en".to_string());
            }
            "--provider" | "-p" => {
                i += 1;
                provider = args.get(i).cloned();
            }
            "--model" | "-m" => {
                i += 1;
                model = args.get(i).cloned();
            }
            other if wav.is_none() => wav = Some(other.to_string()),
            other => bail!("Unexpected argument '{}' for `bench-stt`", other),
        }
        i += 1;
    }

    let wav = wav.context(
        "Usage: ttsandsttp bench-stt <wav> [--lang en] [--provider groq] [--model whisper-large-v3]",
    )?;
    bench::bench_stt(&wav, &lang, provider.as_deref(), model.as_deref()).await
}

/// Synthesize text with a cloud TTS provider and report latency.
async fn cmd_bench_tts(args: &[String]) -> Result<()> {
    let mut text: Option<String> = None;
    let mut lang = "en".to_string();
    let mut provider: Option<String> = None;
    let mut model: Option<String> = None;
    let mut voice: Option<String> = None;
    let mut out: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lang" | "-l" => {
                i += 1;
                lang = args.get(i).cloned().unwrap_or_else(|| "en".to_string());
            }
            "--provider" | "-p" => {
                i += 1;
                provider = args.get(i).cloned();
            }
            "--model" | "-m" => {
                i += 1;
                model = args.get(i).cloned();
            }
            "--voice" | "-v" => {
                i += 1;
                voice = args.get(i).cloned();
            }
            "--out" | "-o" => {
                i += 1;
                out = args.get(i).cloned();
            }
            other if text.is_none() => text = Some(other.to_string()),
            other => bail!("Unexpected argument '{}' for `bench-tts`", other),
        }
        i += 1;
    }

    let text = text.context(
        "Usage: ttsandsttp bench-tts <text> [--lang en] [--provider <id>] [--model <model>] [--voice <voice>] [--out out.wav]",
    )?;
    bench::bench_tts(
        &text,
        &lang,
        provider.as_deref(),
        model.as_deref(),
        voice.as_deref(),
        out.as_deref(),
    )
    .await
}

fn print_help() {
    println!(
        "ttsandsttp - Text-to-Speech and Speech-to-Text service\n\n\
         Usage: ttsandsttp [COMMAND]\n\n\
         Commands:\n\
           daemon                    Run the DBus daemon (default)\n\
           bench-stt <wav>           A/B test local Whisper vs a cloud STT provider\n\
                                     [--lang en] [--provider <id>] [--model <model>]\n\
           bench-tts <text>          Synthesize with a cloud TTS provider, report latency\n\
                                     [--lang en] [--provider <id>] [--model <model>]\n\
                                     [--voice <voice>] [--out out.wav]\n\
           set-key [provider]        Store a provider's API key in the keyring\n\
           del-key [provider]        Remove a provider's API key from the keyring\n\
           providers                 List known providers (catalog + configured)\n\
           models                    List catalog models [--cap stt|tts] [--provider <id>]\n\
           voices                    List Qwen-TTS voices [--lang en], or catalog voices\n\
                                     with [--provider <id>] [--model <model>]\n\
           help                      Show this help\n"
    );
}

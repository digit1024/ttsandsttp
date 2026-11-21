# TTS and STT Service

A lightweight Rust-based Text-to-Speech (TTS) and Speech-to-Text (STT) service with DBus daemon mode support.

## Features

- **Text-to-Speech**: Convert text to natural speech using Sherpa models
- **Speech-to-Text**: Transcribe audio to text with multiple language support
- **Cloud providers**: Groq, OpenRouter, Qwen/DashScope and any OpenAI-compatible endpoint for both STT and TTS (local offline remains the default)
- **DBus Integration**: Exposes service methods for easy integration
- **SystemD Support**: Runs as a user service
- **Configurable**: TOML-based configuration with sensible defaults

## Quick Start

### Debian Package

```bash
# Install the package
sudo dpkg -i ttsandsttp_0.1.0-1_*.deb
sudo apt-get install -f

# Start the service
systemctl --user enable ttsandsttp.service
systemctl --user start ttsandsttp.service
```

### DBus Usage

The service exposes methods at `com.github.digit1024.ttsstt`. The `language`
argument is optional in meaning: pass `""` and cloud models will auto-detect
the spoken language (STT) or the configured `[tts] default` is used (TTS).

```bash
# Text-to-Speech ("" = configured default language)
dbus-send --session --dest=com.github.digit1024.ttsstt --type=method_call \
  /com/github/digit1024/ttsstt com.github.digit1024.ttsstt.Service.Tts \
  string:"Hello world" string:""

# Speech-to-Text ("" = auto-detect)
dbus-send --session --dest=com.github.digit1024.ttsstt --type=method_call \
  /com/github/digit1024/ttsstt com.github.digit1024.ttsstt.Service.Stt \
  string:"" double:2.0

# Speech-to-Text with keyboard typing (forced language example)
dbus-send --session --dest=com.github.digit1024.ttsstt --type=method_call \
  /com/github/digit1024/ttsstt com.github.digit1024.ttsstt.Service.SttType \
  string:"pl-PL" double:2.0
```

## Desktop Integration

### Cosmic DE (and other desktop environments)

The `scripts/` folder contains ready-to-use wrappers:
- `stt [lang]` - One-click speech-to-text dictation (no lang = auto-detect)
- `tts [lang]` - Read clipboard content aloud (no lang = configured default)

**Pro tip**: Add the STT script as a keyboard shortcut in Cosmic Settings → Keyboard → Custom Shortcuts. This lets you dictate into any application with a simple key combo.

## Configuration

Configuration lives at `~/.config/ttsandsttp/config.toml` (created from
`config.toml.default` on first run).

### Providers

STT and TTS each pick a provider independently: `local` (offline, default) or
any cloud provider from the bundled catalog:

| Provider | Kind | STT models | TTS models |
| --- | --- | --- | --- |
| `dashscope` | DashScope | qwen3-asr-flash | qwen3-tts-flash (50 voices) |
| `groq` | OpenAI-compatible | whisper-large-v3, whisper-large-v3-turbo | canopylabs/orpheus-v1-english, orpheus-arabic-saudi |
| `openrouter` | OpenAI-compatible | elevenlabs/scribe-v2, mistralai/voxtral-small-transcribe-3-0, openai/whisper-large-v3, deepgram/nova-3 | elevenlabs/eleven-v4-turbo, google/gemini-3.8-flash-tts, deepgram/aura-2, minimax/speech-2.8-turbo, hexgrad/kokoro-82m, … (14 models) |
| `custom_openai` / `custom_dashscope` | user endpoint | any | any |
| your own `[providers.<id>]` | `kind = "openai"` or `"dashscope"` | any | any |

Explore the catalog:

```bash
ttsandsttp providers                       # all known providers
ttsandsttp models --cap stt                # STT models (or --cap tts)
ttsandsttp models --cap tts --provider openrouter
```

### Language handling

Language is **optional** — cloud models detect it themselves (ASR language
identification; multilingual TTS voices read any language from the text):

- `[stt]` applies to every language. `language = ""` means auto-detect; the
  DBus caller can still pass a language per request.
- The `[tts]` top-level keys (`provider`, `model`, `voice`, …) form a global
  fallback that speaks **any** language without per-language sections.

Per-language configuration is available when you want it:

```toml
[stt]
provider = "groq"                    # any language, auto-detected
model    = "whisper-large-v3-turbo"

[stt.pl]                             # Polish override (empty fields inherit)
provider = "local"
model_id = "sherpa-onnx-whisper-small"

[tts]
default  = "en"
provider = "openrouter"              # global fallback: speaks every language
model    = "elevenlabs/eleven-v4-turbo"
voice    = "sarah"

[tts.en]                             # per-language section wins when present
enabled  = true
provider = "local"
model_id = "vits-piper-en_US-amy-low"
```

`[tts.<lang>]` sections are used as-is (no field merging); `[stt.<lang>]`
overrides merge field-by-field with the global `[stt]`.

Example — Groq STT + OpenRouter TTS:

```toml
[stt]
provider = "groq"
model    = "whisper-large-v3-turbo"
language = ""                 # "" = auto-detect
hints    = ["Filip"]          # vocabulary hints (providers that support them)

[tts.en]
enabled  = true
provider = "openrouter"
model    = "elevenlabs/eleven-v4-turbo"
voice    = "sarah"
```

Example — Qwen / Alibaba Cloud Model Studio (the `[api]` section of older
versions is migrated automatically to `[providers.dashscope]`):

```toml
[providers.dashscope]
region = "singapore"        # singapore | beijing | custom

[stt]
provider = "dashscope"
model    = "qwen3-asr-flash"

[tts.en]
provider = "dashscope"
model    = "qwen3-tts-flash"
voice    = "Cherry"
```

Example — any OpenAI-compatible server on your LAN (vLLM, llama.cpp,
LM Studio, OpenAI):

```toml
[providers.vllm-lan]
kind             = "openai"
base_url         = "http://192.168.1.20:8000/v1"
requires_api_key = false

[stt]
provider = "vllm-lan"
model    = "whisper-large-v3"
```

Long recordings are automatically chunked to each model's request limit and
the transcripts merged; long TTS texts are chunked, synthesized with prefetch
and played gaplessly. When a cloud provider is active, audio/text is uploaded
to that provider. The local provider never leaves your machine.

### API key storage

Keys are stored **per provider** and never written to `config.toml`.
Resolution order (`key_source = "auto"`) is:

1. systemd `$CREDENTIALS_DIRECTORY` (`LoadCredential=<provider>:<path>`;
   DashScope also accepts `dashscope-<region>`)
2. freedesktop Secret Service keyring (GNOME Keyring / KWallet)
3. a `0600` file at `~/.config/ttsandsttp/<provider>.key`
   (DashScope: `dashscope.<region>.key`)
4. the provider's env var (`GROQ_API_KEY`, `OPENROUTER_API_KEY`,
   `DASHSCOPE_API_KEY`, …; development only)

Store a key in the keyring:

```bash
ttsandsttp set-key groq       # prompts; writes to the Secret Service keyring
ttsandsttp del-key groq       # removes it
ttsandsttp set-key            # defaults to dashscope
```

### Voice selection

```bash
ttsandsttp voices                                   # bundled Qwen-TTS voices
ttsandsttp voices --provider openrouter             # all OpenRouter TTS voices
ttsandsttp voices --provider groq --model canopylabs/orpheus-v1-english
```

Set the chosen voice with `[tts.<lang>] voice = "..."`. Any voice string is
accepted (unknown names are passed through with a warning).

### Comparing quality

```bash
ttsandsttp bench-stt recording.wav --lang en
ttsandsttp bench-stt recording.wav --provider groq --model whisper-large-v3-turbo
```

Runs the same audio through the selected (or configured) cloud backend and the
local Whisper model, printing the transcript and latency for each.

```bash
ttsandsttp bench-tts "Hello world" --provider openrouter \
    --model elevenlabs/eleven-v4-turbo --voice sarah --out hello.wav
```

Synthesizes text with a cloud TTS provider (chunked to the model's request
budget), printing per-chunk and total latency, and optionally writing the
audio to a WAV file.

## Requirements

- Linux with PulseAudio or ALSA
- Rust 1.70+ (for building from source)
- SystemD (for service management)
- wl-clipboard (for TTS script)
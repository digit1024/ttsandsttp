# TTS and STT Service

A lightweight Rust-based Text-to-Speech (TTS) and Speech-to-Text (STT) service with DBus daemon mode support.

## Features

- **Text-to-Speech**: Convert text to natural speech using Sherpa models
- **Speech-to-Text**: Transcribe audio to text with multiple language support
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

The service exposes methods at `com.github.digit1024.ttsstt`:

```bash
# Text-to-Speech
dbus-send --session --dest=com.github.digit1024.ttsstt --type=method_call \
  /com/github/digit1024/ttsstt com.github.digit1024.ttsstt.Service.Tts \
  string:"Hello world" string:"en-US"

# Speech-to-Text
dbus-send --session --dest=com.github.digit1024.ttsstt --type=method_call \
  /com/github/digit1024/ttsstt com.github.digit1024.ttsstt.Service.Stt \
  string:"en-US" double:2.0

# Speech-to-Text with keyboard typing
dbus-send --session --dest=com.github.digit1024.ttsstt --type=method_call \
  /com/github/digit1024/ttsstt com.github.digit1024.ttsstt.Service.SttType \
  string:"en-US" double:2.0
```

## Desktop Integration

### Cosmic DE (and other desktop environments)

The `scripts/` folder contains ready-to-use wrappers:
- `stt` - One-click speech-to-text dictation
- `tts` - Read clipboard content aloud

**Pro tip**: Add the STT script as a keyboard shortcut in Cosmic Settings → Keyboard → Custom Shortcuts. This lets you dictate into any application with a simple key combo.

## Configuration

Configuration lives at `~/.config/ttsandsttp/config.toml` (created from
`config.toml.default` on first run).

### Cloud providers (Qwen / Alibaba Cloud Model Studio)

Cloud STT/TTS is **opt-in** and **local remains the default**. To use it, set
`provider = "dashscope"` on `[stt]` or `[tts.<lang>]`:

```toml
[api]
region = "singapore"        # singapore | beijing | custom

[stt]
provider  = "dashscope"
api_model = "qwen3-asr-flash"

[tts.en]
provider  = "dashscope"
api_model = "qwen3-tts-flash"
voice     = "Cherry"
```

When the DashScope provider is active, dictated audio is uploaded to Alibaba
Cloud. The local provider never leaves your machine.

### API key storage

The key is never written to `config.toml`. Resolution order
(`key_source = "auto"`) is:

1. systemd `$CREDENTIALS_DIRECTORY` (`LoadCredential=dashscope-<region>:<path>`)
2. freedesktop Secret Service keyring (GNOME Keyring / KWallet)
3. a `0600` file at `~/.config/ttsandsttp/dashscope.<region>.key`
4. `$DASHSCOPE_API_KEY` (development only)

Store it in the keyring:

```bash
ttsandsttp set-key     # prompts; writes to the Secret Service keyring
ttsandsttp del-key     # removes it
```

### Voice selection

```bash
ttsandsttp voices              # list bundled Qwen-TTS voices
ttsandsttp voices --lang en
```

Set the chosen voice with `[tts.<lang>] voice = "Cherry"`. Any voice string is
accepted (unknown names are passed through with a warning).

### Comparing quality

```bash
ttsandsttp bench-stt recording.wav --lang en
```

Runs the same audio through the configured backend and the local Whisper model,
printing the transcript and latency for each.
## Requirements

- Linux with PulseAudio or ALSA
- Rust 1.70+ (for building from source)
- SystemD (for service management)
- wl-clipboard (for TTS script)
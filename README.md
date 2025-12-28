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



## Requirements

- Linux with PulseAudio or ALSA
- Rust 1.70+ (for building from source)
- SystemD (for service management)
- wl-clipboard (for TTS script)
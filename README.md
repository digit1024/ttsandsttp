# TTS and STT Service

A Rust-based Text-to-Speech (TTS) and Speech-to-Text (STT) service with DBus daemon mode support.

## Installation

### Debian Package (Recommended)

```bash
# Install the package
sudo dpkg -i ttsandsttp_0.1.0-1_*.deb

# Fix any missing dependencies
sudo apt-get install -f
```

### SystemD Service

```bash
# Enable and start the service
systemctl --user enable ttsandsttp.service
systemctl --user start ttsandsttp.service

# Check status
systemctl --user status ttsandsttp.service
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
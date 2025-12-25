# TTS and STT Service

A Rust CLI application implementing Text-to-Speech (TTS) and Speech-to-Text (STT) functionality using sherpa-rs, with DBus daemon mode support.

## Installation

### Option 1: Install from Debian Package (Recommended)

1. **Build the Debian package:**
   
   **Option A: Using the build script (recommended if you have Rust via rustup):**
   ```bash
   # Install debhelper (if not already installed)
   sudo apt-get update
   sudo apt-get install build-essential debhelper libasound2-dev pkg-config
   
   # Run the build script (handles rustup-installed Rust automatically)
   ./build-deb.sh
   ```
   
   **Option B: Manual build (if you have Rust system packages):**
   ```bash
   # Install build dependencies
   sudo apt-get update
   sudo apt-get install debhelper cargo rustc libasound2-dev pkg-config
   
   # Build the package
   dpkg-buildpackage -b -uc -us
   ```
   
   **Note:** If you have Rust installed via `rustup` (the standard method), use Option A or add the `-d` flag to skip dependency checks:
   ```bash
   dpkg-buildpackage -b -uc -us -d
   ```

2. **Install the package:**
   ```bash
   sudo dpkg -i ../ttsandsttp_0.1.0-1_*.deb
   
   # If there are missing dependencies, fix them with:
   sudo apt-get install -f
   ```

3. **Enable and start the systemd user service:**
   ```bash
   systemctl --user enable ttsandsttp.service
   systemctl --user start ttsandsttp.service
   ```

4. **Check service status:**
   ```bash
   systemctl --user status ttsandsttp.service
   ```

### Option 2: Build from Source

1. **Install Rust:**
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Install system dependencies:**
   ```bash
   sudo apt-get update
   sudo apt-get install libasound2-dev pkg-config wtype
   ```

3. **Build the project:**
   ```bash
   unset ARGV0 && cargo build --release
   ```

4. **Run the daemon manually:**
   ```bash
   ./target/release/ttsandsttp daemon
   ```

## Systemd User Service

The package includes a systemd user service that automatically starts the DBus daemon on login.

### Service Management

**Enable service (starts on login):**
```bash
systemctl --user enable ttsandsttp.service
```

**Start service immediately:**
```bash
systemctl --user start ttsandsttp.service
```

**Stop service:**
```bash
systemctl --user stop ttsandsttp.service
```

**Disable service (prevents auto-start):**
```bash
systemctl --user disable ttsandsttp.service
```

**View service logs:**
```bash
journalctl --user -u ttsandsttp.service -f
```

**Check service status:**
```bash
systemctl --user status ttsandsttp.service
```

### Service Configuration

The service file is located at `/usr/lib/systemd/user/ttsandsttp.service`. To customize it:

1. Copy the service file to your user config:
   ```bash
   mkdir -p ~/.config/systemd/user
   cp /usr/lib/systemd/user/ttsandsttp.service ~/.config/systemd/user/
   ```

2. Edit `~/.config/systemd/user/ttsandsttp.service` as needed

3. Reload systemd:
   ```bash
   systemctl --user daemon-reload
   systemctl --user restart ttsandsttp.service
   ```

## Structure

```
src/
├── main.rs                 # CLI entry point
├── lib.rs                  # Library exports
└── ui/
    └── audio/
        ├── mod.rs          # Module exports
        ├── model_manager.rs # Model download and management
        ├── tts_service.rs  # TTS service implementation
        └── stt_service.rs  # STT service implementation
```

## Features

### Model Manager (`ModelManager`)
- ✅ Automatic model download from GitHub releases
- ✅ Model verification (checks for required files)
- ✅ Progress bars for downloads
- ✅ Models stored in `{app_data_dir}/stttts/`
- ✅ Supports TTS, STT, and VAD models

### TTS Service (`TtsService`)
- ✅ Service structure with state management
- ✅ Language support tracking
- ✅ Automatic model download on initialization
- ✅ Integrated with `sherpa_rs::tts::VitsTts`
- ✅ Audio playback using `rodio`

### STT Service (`SttService`)
- ✅ Service structure with state management
- ✅ Callback system for results, pause detection, and errors
- ✅ Automatic model download on initialization (STT + VAD)
- ✅ Integrated with Whisper recognizer
- ✅ Audio input capture using `cpal`
- ✅ Pause detection for automatic stopping

### DBus Daemon Mode
- ✅ Preloads TTS and STT models at startup
- ✅ Exposes DBus interface for remote method calls
- ✅ TTS method for text-to-speech conversion
- ✅ STT method for speech-to-text conversion
- ✅ STT Type method for speech-to-text with automatic keyboard typing
- ✅ Thread-safe channel-based architecture

## System Dependencies

### Linux (Ubuntu/Debian/Pop OS)

For DBus daemon mode with STT Type functionality (keyboard typing), install based on your display server:

#### Wayland (default on Pop OS, Ubuntu 22.04+)

```bash
sudo apt-get update
sudo apt-get install wtype
```

#### X11 (legacy X server)

```bash
sudo apt-get update
sudo apt-get install libxdo-dev
```

The application automatically detects your display server:
- If `WAYLAND_DISPLAY` is set → uses `wtype` command
- If `DISPLAY` is set (and not Wayland) → uses `enigo` library (requires `libxdo-dev`)

### Other Dependencies

The following are typically already installed on most Linux systems:
- `libasound2-dev` (for audio I/O via ALSA)
- DBus libraries (usually pre-installed)

## Usage

### TTS (Text-to-Speech)
```bash
cargo run -- tts "Hello, world!" --language en-US
```

### STT (Speech-to-Text)
```bash
cargo run -- stt --language en-US --pause-duration 2.0
```

### DBus Daemon Mode

The DBus daemon service can be started in two ways:

**If installed via package (systemd service):**
The service starts automatically when enabled. See [Systemd User Service](#systemd-user-service) section above.

**If running from source:**
```bash
cargo run -- daemon
# or
./target/release/ttsandsttp daemon
```

The daemon will:
1. Preload TTS and STT models at startup
2. Expose DBus service at `com.github.digit1024.ttsstt`
3. Wait for DBus method calls

#### DBus Methods

**Service:** `com.github.digit1024.ttsstt`  
**Object:** `/com/github/digit1024/ttsstt`  
**Interface:** `com.github.digit1024.ttsstt.Service`

Available methods:
- `Tts(text: String, language: String)` - Convert text to speech
- `Stt(language: String, pause_duration: f64)` - Convert speech to text, returns recognized text
- `SttType(language: String, pause_duration: f64)` - Convert speech to text and type it using keyboard simulation
- `Stop()` - Stop current TTS playback

Available signals:
- `StatusChanged(status: String)` - Emitted when status changes. Status can be:
  - `"idle"` - Service is idle, no active operation
  - `"speaking"` - TTS is currently speaking
  - `"listening"` - STT is currently listening/recording
  - `"processing"` - Service is processing (e.g., initializing models or processing STT results)

#### Example DBus Calls

Using `dbus-send`:

```bash
# TTS call
dbus-send --session \
  --dest=com.github.digit1024.ttsstt \
  --type=method_call \
  /com/github/digit1024/ttsstt \
  com.github.digit1024.ttsstt.Service.Tts \
  string:"Hello world" string:"en-US"

# STT call
dbus-send --session \
  --dest=com.github.digit1024.ttsstt \
  --type=method_call \
  /com/github/digit1024/ttsstt \
  com.github.digit1024.ttsstt.Service.Stt \
  string:"en-US" double:2.0

# STT Type call (types the result)
dbus-send --session \
  --dest=com.github.digit1024.ttsstt \
  --type=method_call \
  /com/github/digit1024/ttsstt \
  com.github.digit1024.ttsstt.Service.SttType \
  string:"en-US" double:2.0

# Stop current playback
dbus-send --session \
  --dest=com.github.digit1024.ttsstt \
  --type=method_call \
  /com/github/digit1024/ttsstt \
  com.github.digit1024.ttsstt.Service.Stop
```

#### Listening for Status Changes

To listen for status change signals using `dbus-monitor`:

```bash
dbus-monitor --session "interface='com.github.digit1024.ttsstt.Service',member='StatusChanged'"
```

Or using `gdbus`:

```bash
gdbus monitor --session --dest com.github.digit1024.ttsstt --object-path /com/github/digit1024/ttsstt
```

## Next Steps

### 1. Complete TTS Implementation

In `src/ui/audio/tts_service.rs`:

1. **Initialize VitsTts**:
   ```rust
   let engine = sherpa_rs::tts::VitsTts::new(
       "path/to/model.onnx",
       "path/to/config.json",
       // ... other parameters
   )?;
   ```

2. **Generate audio**:
   ```rust
   let audio_samples = engine.generate(text)?;
   ```

3. **Play audio** (add `rodio` or `cpal` dependency):
   ```rust
   // Use rodio or cpal to play audio_samples
   ```

### 2. Complete STT Implementation

In `src/ui/audio/stt_service.rs`:

1. **Choose recognizer type**:
   - `sherpa_rs::transducer::OnlineRecognizer`
   - `sherpa_rs::paraformer::OnlineRecognizer`
   - `sherpa_rs::zipformer::OnlineRecognizer`

2. **Initialize recognizer**:
   ```rust
   let config = sherpa_rs::transducer::OnlineRecognizerConfig {
       // Configure with model paths, sample rate, etc.
   };
   let recognizer = sherpa_rs::transducer::OnlineRecognizer::new(&config)?;
   ```

3. **Set up audio input** (add `cpal` dependency):
   ```rust
   // Use cpal to capture audio from microphone
   // Feed samples to recognizer.accept_waveform(sample_rate, samples)
   ```

4. **Process results**:
   ```rust
   if recognizer.is_ready() {
       let result = recognizer.get_result()?;
       // Call result_callback
   }
   ```

## Dependencies

### Rust Dependencies (Cargo.toml)

- `sherpa-rs` - TTS/STT engine
- `tokio` - Async runtime
- `clap` - CLI argument parsing
- `anyhow` - Error handling
- `reqwest` - HTTP client for model downloads
- `tar` + `bzip2` + `flate2` - Archive extraction
- `dirs` - Application data directory detection
- `indicatif` - Progress bars
- `futures` - Async utilities
- `rodio` - Audio playback (for TTS)
- `cpal` - Audio capture (for STT)
- `zbus` - DBus interface implementation
- `zvariant` - DBus data types
- `enigo` - Keyboard input simulation (for STT Type feature)
- `hound` - WAV file handling

## Model Files

Models are **automatically downloaded** on first use! No manual setup required.

### Model Storage
Models are stored in your application data directory:
- **Linux**: `~/.local/share/stttts/`
- **macOS**: `~/Library/Application Support/stttts/`
- **Windows**: `%APPDATA%\stttts\`

### Supported Models
- **TTS**: `vits-piper-en_US-amy-medium` (English)
- **STT**: `sherpa-onnx-zipformer-en-2024-04-23` (English)
- **VAD**: `silero_vad` (Voice Activity Detection)

### Manual Model Management
If you need to customize models, you can:
1. Modify `ModelType::default_url()` in `src/ui/audio/model_manager.rs`
2. Place models manually in the models directory
3. The service will verify required files are present

Models are downloaded from [sherpa-onnx GitHub releases](https://github.com/k2-fsa/sherpa-onnx/releases).

## Architecture

### DBus Daemon Mode

The DBus daemon uses a channel-based architecture to handle services that contain non-Send types (audio streams). TTS and STT services run in dedicated threads with their own tokio runtimes, and the DBus service communicates with them via message channels.

```
┌─────────────┐
│ DBus Client │
└──────┬──────┘
       │ Method Calls
       ▼
┌─────────────────────┐
│  DBus Service       │
│  (TtsSttService)    │
└──────┬──────────────┘
       │ Channels
       ├──────────────┐
       ▼              ▼
┌──────────┐    ┌──────────┐
│ TTS      │    │ STT      │
│ Thread   │    │ Thread   │
└──────────┘    └──────────┘
```

## Notes

- Models are automatically downloaded on first use
- DBus daemon mode STT Type functionality:
  - **Wayland**: Requires `wtype` command (install with `sudo apt install wtype`)
  - **X11**: Requires `libxdo-dev` (install with `sudo apt install libxdo-dev`)
- The application automatically detects your display server (Wayland or X11)
- All services handle initialization and model loading automatically
- The daemon keeps models in memory for fast response times

# Debugging STT Issues

## Quick Debug Run

```bash
./debug_stt.sh
```

This script:
- Sets `SHERPA_ONNX_LOG_LEVEL=DEBUG` for C++ logging
- Sets `RUST_BACKTRACE=full` for Rust stack traces
- Sets `RUST_LOG=debug` for Rust logging
- Optionally runs with `gdb` if available

## Manual Debugging

### 1. Enable C++ Logging
```bash
export SHERPA_ONNX_LOG_LEVEL=DEBUG
# Options: TRACE, DEBUG, INFO, WARNING, ERROR, FATAL
```

### 2. Enable Rust Backtrace
```bash
export RUST_BACKTRACE=full
# or
export RUST_BACKTRACE=1
```

### 3. Enable Rust Logging
```bash
export RUST_LOG=debug
```

### 4. Run with gdb
```bash
unset ARGV0
cargo build
gdb --args target/debug/ttsandsttp stt --language en-US --pause-duration 2.0
```

In gdb:
```
(gdb) run
# When it crashes:
(gdb) bt              # Show backtrace
(gdb) frame 0         # Go to frame 0 (top of stack)
(gdb) info registers  # Show CPU registers
(gdb) print <var>     # Print variable
```

### 5. Check Audio Data

The code now logs:
- Chunk size and sample rate
- Audio duration
- Sample value range
- First/last samples
- NaN/Inf detection
- Value range warnings

### 6. Common Issues

**Crash in C++ resampler:**
- The C++ code is trying to resample from 44100Hz to 16000Hz
- Our Rust code already resamples, but the C++ code doesn't know that
- Check if the audio values are in the expected range [-1.0, 1.0]

**Sample rate mismatch:**
- Verify resampling is working: check debug output for "Resampled X samples at YHz to Z samples at 16kHz"
- Ensure we're passing 16000 to decode()

**Invalid audio data:**
- Check for NaN or Inf values in debug output
- Verify sample values are in reasonable range

## Environment Variables Reference

| Variable | Values | Purpose |
|----------|--------|---------|
| `SHERPA_ONNX_LOG_LEVEL` | TRACE, DEBUG, INFO, WARNING, ERROR, FATAL | C++ library logging level |
| `RUST_BACKTRACE` | full, 1 | Rust stack trace detail |
| `RUST_LOG` | trace, debug, info, warn, error | Rust logging level |
| `SHERPA_ONNX_ABORT` | (any value) | Enable abort on fatal errors |

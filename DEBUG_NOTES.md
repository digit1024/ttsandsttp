# Debug Notes - STT Crash Issue

## Problem
The C++ code in `sherpa-onnx` crashes when trying to create a `LinearResample` object in `offline-stream.cc:170`.

## Root Cause Analysis

From the C++ source (`offline-stream.cc:159-174`):
```cpp
if (sampling_rate != config_.sampling_rate) {
    // Creates LinearResample and crashes here
    auto resampler = std::make_unique<LinearResample>(
        sampling_rate, config_.sampling_rate, lowpass_cutoff,
        lowpass_filter_width);
    // ...
}
```

**The issue**: Even though we're passing `16000` and the config expects `16000`, the C++ code is somehow detecting a mismatch and trying to resample. The crash happens in the `LinearResample` constructor or `Resample` method.

## Possible Causes

1. **Type mismatch**: We pass `u32` (16000), C++ expects `i32`. But 16000 fits in i32, so this shouldn't cause issues.

2. **C++ auto-detection**: The C++ code might be analyzing audio characteristics to detect sample rate, ignoring our parameter.

3. **Bug in LinearResample**: The C++ `LinearResample` class might have a bug when handling certain sample rate combinations or edge cases.

4. **Memory corruption**: The crash might be due to memory issues in the C++ code.

## Current Status

- ✅ Resampling in Rust is working correctly (44100Hz → 16000Hz)
- ✅ Audio data looks valid (range [-0.165, 0.476], no NaN/Inf)
- ✅ Sample rate parameter is 16000
- ❌ C++ code still crashes when creating resampler

## Next Steps to Try

1. **Check if first samples being zero causes issues**: Skip chunks with all zeros
2. **Try different chunk sizes**: Maybe smaller or larger chunks work better
3. **Check C++ LinearResample source**: See if there are known issues or requirements
4. **Try a different model**: Maybe this specific model has issues
5. **Report bug to sherpa-rs**: This appears to be a bug in the C++ library

## Workaround Ideas

1. **Use streaming API instead**: If available, streaming might not have this issue
2. **Pre-resample to exact 16kHz**: Use a better resampling library to ensure perfect 16kHz
3. **Accumulate full utterance**: Instead of chunking, wait for complete utterance before decoding
4. **Use different STT library**: Consider alternatives if this can't be fixed

## Environment Variables for Debugging

```bash
export SHERPA_ONNX_LOG_LEVEL=DEBUG  # C++ logging
export RUST_BACKTRACE=full           # Rust stack traces
export RUST_LOG=debug                # Rust logging
```

## Files to Check

- `target/debug/build/sherpa-rs-sys-*/out/sherpa-onnx/sherpa-onnx/csrc/offline-stream.cc:159-174`
- `target/debug/build/sherpa-rs-sys-*/out/sherpa-onnx/sherpa-onnx/csrc/resample.h` (if exists)

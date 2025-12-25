# Next Steps - Implementation Roadmap

## ✅ Completed

- [x] Project structure and module organization
- [x] TTS Service skeleton with state management
- [x] STT Service skeleton with callback system
- [x] Model Manager with automatic download
- [x] Model verification and path resolution
- [x] CLI interface
- [x] Error handling and progress indicators

## 🎯 Priority Tasks

### 1. Integrate TTS with sherpa-rs (High Priority)

**File**: `src/ui/audio/tts_service.rs`

#### Step 1.1: Add audio playback dependency
```toml
# Cargo.toml
[dependencies]
rodio = "0.17"  # or cpal for more control
```

#### Step 1.2: Update TTS service to use VitsTts

1. **Change engine type**:
   ```rust
   engine: Arc<Mutex<Option<sherpa_rs::tts::VitsTts>>>,
   ```

2. **Initialize VitsTts in `init()`**:
   ```rust
   let model_file = self.model_manager.get_model_file(&ModelType::Tts, "en_US-amy-low.onnx");
   let tokens_file = self.model_manager.get_model_file(&ModelType::Tts, "tokens.txt");
   let data_dir = actual_path.join("espeak-ng-data");
   
   // Check sherpa-rs docs for exact API
   let engine = sherpa_rs::tts::VitsTts::new(
       &model_file,
       &tokens_file,
       &data_dir,  // espeak-ng-data directory
       // ... other parameters
   )?;
   ```

3. **Implement `speak()` method**:
   ```rust
   pub async fn speak(&self, text: &str) -> Result<()> {
       // ... existing initialization check ...
       
       let engine_guard = self.engine.lock().unwrap();
       if let Some(ref engine) = *engine_guard {
           // Generate audio samples
           let audio = engine.generate(text)?;  // Check actual API method name
           
           // Play audio using rodio
           use rodio::{Decoder, OutputStream, Sink};
           let (_stream, stream_handle) = OutputStream::try_default()?;
           let sink = Sink::try_new(&stream_handle)?;
           
           // Convert audio samples to WAV or use raw samples
           // You may need to convert the audio format
           sink.append(Decoder::new(audio)?);
           sink.sleep_until_end();
       }
       
       // ... update state ...
       Ok(())
   }
   ```

**Resources**:
- Check `sherpa_rs::tts::VitsTts` documentation
- Look at examples in `target/debug/build/sherpa-rs-sys-*/out/sherpa-onnx/`
- Model files are at: `~/.local/share/stttts/tts/vits-piper-en_US-amy-low/`

---

### 2. Integrate STT with sherpa-rs (High Priority)

**File**: `src/ui/audio/stt_service.rs`

#### Step 2.1: Add audio capture dependency
```toml
# Cargo.toml
[dependencies]
cpal = "0.15"  # For audio input
```

#### Step 2.2: Choose recognizer type

Options:
- `sherpa_rs::transducer::OnlineRecognizer`
- `sherpa_rs::zipformer::OnlineRecognizer`
- `sherpa_rs::paraformer::OnlineRecognizer`

**Recommendation**: Start with `zipformer` (matches downloaded model)

#### Step 2.3: Update STT service

1. **Change recognizer type**:
   ```rust
   recognizer: Arc<Mutex<Option<sherpa_rs::zipformer::OnlineRecognizer>>>,
   ```

2. **Initialize recognizer in `init()`**:
   ```rust
   let encoder_file = self.model_manager.get_model_file(&ModelType::Stt, "encoder.onnx");
   let decoder_file = self.model_manager.get_model_file(&ModelType::Stt, "decoder.onnx");
   let joiner_file = self.model_manager.get_model_file(&ModelType::Stt, "joiner.onnx");
   let tokens_file = self.model_manager.get_model_file(&ModelType::Stt, "tokens.txt");
   
   // Check sherpa-rs docs for exact config structure
   let config = sherpa_rs::zipformer::OnlineRecognizerConfig {
       encoder: encoder_file.to_string_lossy().to_string(),
       decoder: decoder_file.to_string_lossy().to_string(),
       joiner: joiner_file.to_string_lossy().to_string(),
       tokens: tokens_file.to_string_lossy().to_string(),
       sample_rate: 16000,  // Typical for ASR
       // ... other config
   };
   
   let recognizer = sherpa_rs::zipformer::OnlineRecognizer::new(&config)?;
   ```

3. **Implement `start_listening()` with audio capture**:
   ```rust
   pub async fn start_listening(&self, lang: &str, pause_duration: Duration) -> Result<()> {
       // ... existing initialization ...
       
       // Start audio capture in a separate task
       let recognizer_clone = self.recognizer.clone();
       let result_cb = self.result_callback.clone();
       let pause_cb = self.pause_callback.clone();
       
       tokio::spawn(async move {
           use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
           
           let host = cpal::default_host();
           let device = host.default_input_device()?;
           let config = device.default_input_config()?;
           
           let stream = device.build_input_stream(
               &config.into(),
               move |data: &[f32], _: &cpal::InputCallbackInfo| {
                   // Feed samples to recognizer
                   if let Some(ref mut rec) = *recognizer_clone.lock().unwrap() {
                       rec.accept_waveform(sample_rate, data)?;
                       
                       if rec.is_ready() {
                           let result = rec.get_result()?;
                           // Call result callback
                           if let Some(ref cb) = *result_cb.lock().unwrap() {
                               cb(&result.text);
                           }
                       }
                   }
               },
               |err| eprintln!("Audio error: {}", err),
               None,
           )?;
           
           stream.play()?;
           // Keep running until stop_listening is called
       });
       
       Ok(())
   }
   ```

**Resources**:
- Check `sherpa_rs::zipformer::OnlineRecognizer` documentation
- Model files are at: `~/.local/share/stttts/stt/sherpa-onnx-streaming-zipformer-en-2024-03-15/`

---

### 3. Testing & Refinement

#### 3.1: Test TTS Pipeline
```bash
cargo run -- tts "Hello, world!"
# Should play audio through speakers
```

#### 3.2: Test STT Pipeline
```bash
cargo run -- stt --language en-US
# Should capture microphone input and print recognized text
```

#### 3.3: Error Handling
- Handle audio device not found
- Handle model loading errors
- Handle network errors during download
- Handle unsupported audio formats

#### 3.4: Performance Optimization
- Reuse audio streams
- Buffer audio samples efficiently
- Handle async operations properly

---

## 📚 Research Needed

### TTS Integration
1. **Check VitsTts API**:
   ```bash
   cargo doc --open --package sherpa-rs
   # Navigate to tts::VitsTts
   ```

2. **Check actual method names**:
   - Is it `generate()`, `synthesize()`, or `speak()`?
   - What format does it return? (WAV bytes, raw samples, etc.)
   - What parameters does `new()` take?

3. **Audio format**:
   - Sample rate?
   - Channels (mono/stereo)?
   - Bit depth?

### STT Integration
1. **Check OnlineRecognizer API**:
   ```bash
   cargo doc --open --package sherpa-rs
   # Navigate to zipformer::OnlineRecognizer
   ```

2. **Check method signatures**:
   - `accept_waveform(sample_rate, samples)` signature
   - `is_ready()` return type
   - `get_result()` return type
   - `reset()` method existence

3. **Audio requirements**:
   - Required sample rate
   - Required format (mono/stereo)
   - Buffer size recommendations

---

## 🔧 Quick Start Commands

### Explore sherpa-rs API
```bash
# Generate docs
cargo doc --open --package sherpa-rs

# Check available modules
cargo doc --package sherpa-rs --no-deps 2>&1 | grep "pub mod"
```

### Check Model Files
```bash
# List TTS model files
ls -la ~/.local/share/stttts/tts/vits-piper-en_US-amy-low/

# List STT model files (after download)
ls -la ~/.local/share/stttts/stt/
```

### Test Model Path Resolution
```rust
// In a test or main.rs
use ttsandsttp::ModelManager;
let mgr = ModelManager::new()?;
let model_file = mgr.get_model_file(&ModelType::Tts, "en_US-amy-low.onnx");
println!("Model file: {:?}", model_file);
```

---

## 🎨 Optional Enhancements

### After Core Implementation

1. **Audio Format Conversion**
   - Handle different sample rates
   - Convert between formats if needed

2. **Streaming TTS**
   - Stream audio as it's generated
   - Lower latency

3. **STT Improvements**
   - Real-time partial results
   - Better pause detection
   - VAD integration for automatic pause detection

4. **Language Support**
   - Multiple language models
   - Language switching

5. **Configuration**
   - Config file for model paths
   - Custom model URLs
   - Audio device selection

---

## 📝 Notes

- Model files are automatically downloaded to `~/.local/share/stttts/`
- TTS model: `vits-piper-en_US-amy-low` (English, ~64MB)
- STT model: `sherpa-onnx-streaming-zipformer-en-2024-03-15` (English)
- VAD model: `silero_vad.onnx` (for better STT performance)

All model paths are resolved automatically via `ModelManager::get_model_file()`.

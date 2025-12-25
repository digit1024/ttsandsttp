# Testing Polish TTS Model

## ✅ Current Status

- **Config**: Polish enabled in `~/.config/ttsandsttp/config.toml`
- **Model**: `vits-piper-pl_PL-zenski_wg_glos-medium`
- **Files**: All downloaded and ready
  - ✅ `pl_PL-zenski_wg_glos-medium.onnx` (61MB)
  - ✅ `pl_PL-zenski_wg_glos-medium.onnx.json` (7KB)
  - ✅ `tokens.txt` (422 bytes, 256 tokens)

## 🧪 Testing Steps

### 1. Start the Daemon

```bash
cd /home/digit1024/proj/ttsandsttp
cargo run -- daemon
```

The daemon will:
- Load config (Polish should be enabled)
- Download Polish model if needed (already done!)
- Start DBus service

### 2. Test Polish TTS via DBus

**In a new terminal**, test with a Polish phrase:

```bash
# Simple greeting
dbus-send --session \
  --dest=com.github.digit1024.ttsstt \
  --type=method_call \
  /com/github/digit1024/ttsstt \
  com.github.digit1024.ttsstt.Service.Tts \
  string:"Dzień dobry" string:"pl"

# Hello world
dbus-send --session \
  --dest=com.github.digit1024.ttsstt \
  --type=method_call \
  /com/github/digit1024/ttsstt \
  com.github.digit1024/ttsstt \
  com.github.digit1024.ttsstt.Service.Tts \
  string:"Witaj świecie" string:"pl"

# Test sentence
dbus-send --session \
  --dest=com.github.digit1024.ttsstt \
  --type=method_call \
  /com/github/digit1024/ttsstt \
  com.github.digit1024.ttsstt.Service.Tts \
  string:"To jest test polskiego modelu TTS" string:"pl"
```

### 3. Verify Model Files

```bash
# Check files exist
ls -lh ~/.local/share/stttts/tts/pl/

# Verify tokens.txt
head -20 ~/.local/share/stttts/tts/pl/tokens.txt
wc -l ~/.local/share/stttts/tts/pl/tokens.txt  # Should be 257 (256 tokens + empty line)
```

### 4. Check Daemon Logs

When you run the daemon, you should see:
```
📋 Loading configuration...
✅ Configuration loaded from: /home/digit1024/.config/ttsandsttp/config.toml
🔍 Validating configuration...
✅ Configuration is valid
📥 Checking and downloading required models...
✅ TTS model for pl already present
✅ TTS model for pl ready
```

## 📝 Polish Test Phrases

- `"Dzień dobry"` - Good morning
- `"Witaj świecie"` - Hello world
- `"To jest test"` - This is a test
- `"Jak się masz?"` - How are you?
- `"Dziękuję bardzo"` - Thank you very much

## 🔍 Troubleshooting

If TTS doesn't work:

1. **Check config is valid:**
   ```bash
   cat ~/.config/ttsandsttp/config.toml | grep -A 2 "\[tts.pl\]"
   ```

2. **Verify model files:**
   ```bash
   ls -lh ~/.local/share/stttts/tts/pl/
   ```

3. **Check daemon logs** for errors

4. **Verify tokens.txt format:**
   ```bash
   file ~/.local/share/stttts/tts/pl/tokens.txt
   head -5 ~/.local/share/stttts/tts/pl/tokens.txt
   ```

## 🎯 Expected Behavior

When you call TTS with `string:"pl"`:
- Daemon should use Polish model (`vits-piper-pl_PL-zenski_wg_glos-medium`)
- Should speak the Polish text
- Should use the male voice (zenski)


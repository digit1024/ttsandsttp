# Plan: Multi-provider TTS + STT for ttsandsttp

Status: FINAL · Date: 2026-11-20 · Version: ttsandsttp 1.3.0 → 1.4.0
Sources analysed: `/home/digit1024/proj/mbdictate` @ `1dcf4b7` (STT reference),
`/home/digit1024/proj/LunaAI/mobile_app` (TTS reference)

## 1. Goal & scope

Extend `ttsandsttp` so **every model and provider** catalogued in the two
sibling apps is selectable for both directions through one bigger config:

- **STT catalog source**: mbdictate — `lib/voice/models.dart`,
  `android/.../stt/SttClient.kt` (request shapes, chunking).
- **TTS catalog source**: LunaAI/mobile_app — `lib/voice/voice_catalog.dart`,
  `remote_tts_provider.dart`, `lib/tts/*` (mbdictate has no TTS).
- Local sherpa/piper stays the default; offline behavior unchanged.
- Out of scope: `builtin` device-TTS preset (local sherpa/piper already
  covers offline) and any LLM integration.

Resolved decisions: (1) catalog as bundled `providers.json`; (2) old `[api]`
section is **force-renamed** to `[providers.dashscope]` — one-shot silent
migration at load with a console notice, no permanent alias; (3) `symphonia`
accepted as the mp3 decode dependency.

## 2. Inventory to port

### 2.1 Protocols (4)

| protocol | used by | shape |
| --- | --- | --- |
| `openai_transcription` | groq, openrouter, custom STT | multipart `POST /audio/transcriptions` (`model`, `language`, `response_format`, `temperature`, `prompt`=vocabulary, extra fields) |
| `dashscope_asr` | qwen, custom STT | chat/completions + base64 `input_audio` data-URL, `asr_options {enable_lid \| language, enable_itn}`, vocabulary as system message |
| `openai_speech` | groq, openrouter, custom TTS | JSON `POST /audio/speech` (`model`, `input`, `voice`, `response_format`) |
| `dashscope_tts` | qwen TTS | multimodal-generation: existing SSE streaming + LunaAI non-streaming wav (base64 `output.audio.data` or `audio.url`) |

### 2.2 STT models (9 cloud + local)

- groq: `whisper-large-v3`, `whisper-large-v3-turbo` — 300 s, vocabulary hints
- qwen: `qwen3-asr-flash` — 300 s, hints, per-request LID (`enable_lid`)
- openrouter (45 s): `elevenlabs/scribe-v2`,
  `mistralai/voxtral-small-transcribe-3-0`, `openai/whisper-large-v3`,
  `deepgram/nova-3` (requires explicit language)
- custom_openai (60 s) / custom_dashscope (300 s) — user model strings
- local: unchanged sherpa whisper tiny → large-v3 (`models.json`)

### 2.3 TTS models (19 cloud + local)

- openrouter ×14: `elevenlabs/eleven-v4-turbo`,
  `elevenlabs/eleven-multilingual-v2`, `microsoft/mai-voice-2.1-flash`,
  `google/gemini-3.8-flash-tts`, `google/gemini-3.8-flash-lite-tts`,
  `canopylabs/orpheus-3b-0.1-ft`, `deepgram/aura-2`, `deepgram/flux-tts`,
  `minimax/speech-2.8-turbo`, `speechify/simba-3.5`,
  `assemblyai/assemblyai-tts-1`, `heygen/heygen-voice-1-turbo`,
  `x-ai/grok-voice-tts-1.0`, `hexgrad/kokoro-82m`,
  `mistralai/voxtral-mini-tts-3-0`
- groq ×2: `canopylabs/orpheus-v1-english`, `canopylabs/orpheus-arabic-saudi`
  (200 chars, wav)
- qwen: `qwen3-tts-flash` (50 voices)
- custom_openai / custom_dashscope
- local: vits-piper (`models.json`)
- per-model metadata: `max_chars` (200–3500), `voices[]` (~330 names, 12
  lists), `default_voice`, `supports_instructions`, `response_format`

### 2.4 Behaviors to port

- Format negotiation: preferred → `pcm` retry once on format-400, remembered
  per `endpoint|model` (LunaAI `_postWithFormatFallback`)
- Audio sniffing: content-type + magic bytes (RIFF/ID3); PCM→WAV wrap at
  24 kHz mono 16-bit; mp3 → decode via `symphonia`
- Text chunking at `max_chars` with a short first chunk; prefetch-2 / retry-2
  playback pipeline, gapless into the existing rodio/cpal sink
- Markdown/code stripping before speaking (`markdown_to_speech.dart`)
- WAV chunking with 1–2 s overlap for long STT audio (port from
  `audio_chunker.dart` only when recordings exceed `max_audio_seconds`)

## 3. Current state (gap)

- `src/config/models.rs`: `Provider {Local, Dashscope}` enum only; `[api]` +
  `ApiRegion` are DashScope-specific.
- ~10 `is_dashscope()` branches: `services/stt/service.rs:233,313,395`,
  `services/tts/service.rs:126,170,356`, `main.rs:71`; lazy
  `OnceCell<DashScopeClient>` per service.
- `secrets/mod.rs`: credential→keyring→file→env cascade scoped by region
  slug, not provider.
- `src/services/api/dashscope.rs`: SSE TTS + qwen3-asr only; no
  voices-per-model catalog, no OpenAI-compatible client, no chunk prefetch.

## 4. Architecture changes

1. **Data-driven catalog** — bundled `providers.json` (seed: LunaAI
   `VoiceCatalog` + mbdictate STT metadata), loaded like `models.json` in
   `src/config/`. Types: `ProviderDef {id, label, kind: openai|dashscope,
   stt:{endpoint, protocol}, tts:{endpoint, protocol}, requires_api_key,
   is_custom}`, `SttModelDef {model, max_audio_seconds,
   supports_vocabulary_hint, requires_language}`, `TtsModelDef {model,
   max_chars, voices[], default_voice, supports_instructions,
   response_format}`.
2. **Config schema** — provider-id strings resolved against the catalog;
   `[stt]`/`[tts.<lang>]` gain `model`, `voice`, `hints`, `instructions`,
   `extra_params`; `[providers.<id>]` tables for opt-in + custom entries.
   v1.4 loader: if `[api]` is present, rewrite it into
   `[providers.dashscope]`, print a one-time migration notice, save.
   Unknown provider/model/voice → validation error (extends `validator.rs`);
   voice validity checked per model's `voices[]` instead of the global
   `voices.json` warning (`main.rs:69`).
3. **Secrets per provider** — namespace by provider id (dashscope keeps the
   `dashscope-<region>` credential/file/env names for compatibility);
   `set-key/del-key [provider]`.
4. **API layer** (`src/services/api/`) — `SttBackend` / `TtsBackend` traits,
   dispatch built from catalog kind × protocol, per-provider client cache;
   remove all `is_dashscope()` branches. New `openai.rs`: multipart
   transcription + `/audio/speech` with negotiation/sniff/wrap.
   `dashscope.rs`: + non-streaming TTS fallback, + ASR vocabulary system
   message and `enable_lid`.
5. **Playback** (`services/tts/service.rs`, `utils/`) — LunaAI chunker +
   prefetch/retry pipeline + gapless sink; `text_splitter.rs` gains the
   markdown cleanup; new dep `symphonia`.
6. **CLI & docs** — `providers`, `models --cap stt|tts [--provider p]`,
   `voices --provider p`; `bench-stt --provider <id>`, new `bench-tts`;
   rewritten `config.toml.default` (the bigger config, full commented
   catalog) + README.

### 4.1 Config sketch

```toml
[providers.dashscope]           # replaces old [api]; region/base_url kept
region = "singapore"
[providers.openrouter]          # preset opt-in, STT+TTS
[providers.groq]                # preset opt-in, STT+TTS
[providers.vllm-lan]            # user-defined
kind     = "openai"
base_url = "http://192.168.1.20:8000/v1"

[stt]
provider = "groq"               # local | any configured provider
model    = "whisper-large-v3-turbo"
language = ""                   # "" = auto (enable_lid / omitted)
hints    = ["Filip"]            # vocabulary → prompt / system message

[tts.en]
provider     = "openrouter"
model        = "elevenlabs/eleven-v4-turbo"
voice        = "sarah"
instructions = "warm, calm"     # only for supports_instructions models
stream       = true
```

## 5. Steps (each lands green independently)

1. **Catalog & config layer** — `providers.json` + structs in
   `config/models.rs`, preset/override merge, validator, `[api]` →
   `[providers.dashscope]` migration. Tests: parse, migration notice +
   rewrite, unknown provider/model/voice errors.
2. **Secrets per provider** — provider-scoped cascade + CLI args. Tests:
   namespacing, keyring/file/env resolution per id.
3. **OpenAI-compat STT** — `api/openai.rs` transcription + dispatch wiring;
   vocabulary hints + DashScope `enable_lid` parity. Tests: multipart body,
   language/hints, error text.
4. **OpenAI-speech TTS** — synthesis request + format negotiation +
   content/magic sniffing + PCM→WAV + symphonia mp3 decode; wire
   `tts/service.rs`. Tests: JSON body, sniffer fixtures.
5. **Chunked playback pipeline** — text chunker, prefetch/retry worker,
   gapless sink, markdown cleanup. Tests: chunk sizing, retry policy.
6. **CLI + docs** — `providers`/`models`/`voices` commands, extended bench,
   rewritten `config.toml.default` + README.
7. **Hardening** — `cargo test`, `cargo clippy -- -D warnings`; smoke matrix:
   local / dashscope / openrouter / groq / custom OpenAI server in both
   directions; confirm offline default unchanged.

Estimated effort: ~5–7 dev-days (1–2 ≈ 2 d, 3–4 ≈ 2 d, 5 ≈ 1–2 d, 6–7 ≈ 1 d).

## 6. Risks

- OpenAI-speech `response_format` support varies per model/provider —
  negotiation + pcm/wav preference + mp3 decode covers LunaAI's observed set.
- openrouter model names/limits drift upstream — catalog is data
  (`providers.json`), so refresh needs no code change.
- DashScope compatible-mode vs native endpoints differ per region —
  presets encode both; custom section allows overrides.

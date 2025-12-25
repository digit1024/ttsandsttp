# Refactoring Plan: TTSandSTTP Daemon

## 📋 Executive Summary

This document outlines a comprehensive refactoring plan to:
1. **Remove CLI commands** (Tts, Stt) - keep only **Daemon** mode
2. **Simplify architecture** - adopt clean architecture principles
3. **Reduce complexity** - break down large functions into smaller, focused units
4. **Remove dead code** - eliminate unused code paths and dependencies
5. **Improve maintainability** - add comments, improve flow, and fix potential bugs

---

## 🏗️ Current Architecture Analysis

### Current Structure

```
src/
├── main.rs                 # CLI entry point (TO REMOVE - replace with daemon-only)
├── lib.rs                  # Library exports
├── dbus/
│   ├── mod.rs              # DBus module exports
│   └── service.rs          # TtsSttService - DBus interface
└── ui/
    └── audio/
        ├── mod.rs          # Module exports
        ├── model_manager.rs # Model download/management
        ├── tts_service.rs  # TTS service implementation
        ├── stt_service.rs  # STT service (COMPLEX - 890 lines!)
        └── audio_utils.rs  # Audio processing utilities (GOOD)
```

### Architecture Issues

#### 1. **Mixed Responsibilities**
- `main.rs` contains CLI logic that should be removed
- `dbus/service.rs` has business logic mixed with DBus interface
- `stt_service.rs` has too many responsibilities (audio capture, recognition, state management, callbacks)

#### 2. **Complex Functions**
- `SttService::audio_capture_loop()` - **300+ lines** with deeply nested conditionals
- `SttService::stop_listening()` - **100+ lines** with complex state synchronization
- `ModelManager::download_model()` - handles multiple model types with branching logic

#### 3. **Code Duplication**
- `format_timestamp()` duplicated in `main.rs` and `stt_service.rs`
- `play_beep_blocking()` and `play_beep()` - blocking variant may be dead code
- Multiple places handle audio thread cleanup

#### 4. **State Management Complexity**
- Multiple `Arc<Mutex<>>` nested structures (callback chains)
- State synchronization scattered across methods
- Potential race conditions in `stop_listening()` polling loop

#### 5. **Dead/Unused Code**
- CLI commands (`Commands::Tts`, `Commands::Stt`) - to be removed
- `ModelType::Stt` and `ModelType::Vad` - defined but only `Whisper` used
- `SttService::cancel()` - may be unused
- `play_beep_blocking()` - only used once, could use async version
- `TtsService::set_language()` - sets language but doesn't actually reinitialize properly
- `TtsService::stop()` - may be unused in daemon mode

---

## 🎯 Target Architecture (Clean Architecture)

### Proposed Structure

```
src/
├── main.rs                 # Daemon entry point only
├── lib.rs                  # Library exports (if needed)
├── daemon/
│   ├── mod.rs              # Daemon module exports
│   └── service.rs          # TtsSttDaemon - DBus service
├── domain/
│   ├── mod.rs              # Domain module exports
│   ├── models.rs           # Domain models (ModelType, etc.)
│   └── traits.rs           # Service traits/interfaces
├── services/
│   ├── mod.rs              # Services module exports
│   ├── tts/
│   │   ├── mod.rs
│   │   ├── service.rs      # TtsService (clean interface)
│   │   └── player.rs       # Audio playback abstraction
│   ├── stt/
│   │   ├── mod.rs
│   │   ├── service.rs      # SttService (clean interface)
│   │   ├── audio_capture.rs # Audio capture abstraction
│   │   ├── pause_detector.rs # Pause detection logic
│   │   └── decoder.rs      # Audio decoding/recognition
│   └── models/
│       ├── mod.rs
│       └── manager.rs      # ModelManager (domain service)
└── utils/
    ├── mod.rs              # Utils module exports
    ├── audio.rs            # Audio processing utilities
    ├── logging.rs          # Logging utilities (timestamp formatting)
    └── beep.rs             # Beep playback utilities
```

### Architecture Principles

1. **Separation of Concerns**
   - Domain layer: Core business logic and models
   - Service layer: Use cases and business services
   - Infrastructure layer: DBus, audio I/O, file I/O

2. **Dependency Inversion**
   - Services depend on traits, not concrete implementations
   - Easy to test and mock

3. **Single Responsibility**
   - Each module/struct has one clear purpose
   - Functions limited to ~50 lines max

4. **Clean Interfaces**
   - Services expose simple, focused APIs
   - Internal complexity hidden behind abstractions

---

## 🔍 Detailed Code Analysis

### Critical Complexity Issues

#### 1. `SttService::audio_capture_loop()` (Lines 274-565)

**Problems:**
- **300+ lines** - violates single responsibility
- **Deeply nested conditionals** (4-5 levels deep)
- **Multiple responsibilities:**
  - Audio device setup
  - Audio stream management
  - Sample processing
  - Silence detection
  - Pause detection
  - Speech recognition triggering
  - Thread lifecycle management
  - Error handling

**Complex Flow Issues:**
```rust
loop {
    if should_stop { ... }
    tokio::select! {
        samples = sample_rx.recv() => {
            if should_accumulate {
                let is_speech = ...;
                let has_activity = ...;
                if is_speech { ... }
                else if has_activity {
                    if !has_detected_speech { ... }
                } else {
                    if has_detected_speech {
                        if silence_start.is_none() { ... }
                        if let Some(silence_start_time) = silence_start {
                            if silence_duration > pause_duration { ... }
                        }
                    }
                }
            }
        }
    }
}
```

**Refactoring Plan:**
1. Extract `AudioCapture` struct - handles device/stream setup
2. Extract `PauseDetector` struct - handles silence/pause detection logic
3. Extract `AudioProcessor` - handles resampling, mono conversion
4. Main loop becomes simple state machine

#### 2. `SttService::stop_listening()` (Lines 734-830)

**Problems:**
- **100+ lines** with polling loop
- Complex state synchronization
- Potential race condition: drops recognizer while decode might be in progress
- Timeout-based polling instead of proper async coordination
- Duplicate cleanup code (appears in multiple places)

**Refactoring Plan:**
1. Use proper async signaling (oneshot channels) instead of polling
2. Coordinate decode completion before dropping recognizer
3. Extract cleanup logic to dedicated method

#### 3. `SttService::start_listening()` (Lines 192-271)

**Problems:**
- Spawns multiple tasks without proper coordination
- Recording start flag (`recording_started`) adds complexity
- Beep playback coordination is fragile

**Refactoring Plan:**
1. Extract `RecordingSession` struct to manage lifecycle
2. Use async coordination primitives (oneshot channels)
3. Simplify beep/recording coordination

#### 4. `dbus/service.rs` - TTS/STT Handler Threads (Lines 40-113)

**Problems:**
- Creates new runtime per thread (inefficient)
- Timeout logic in STT handler (check_count > 600) is magic number
- Empty callbacks in STT handler
- No proper error propagation
- Service instances created inside threads (makes testing hard)

**Refactoring Plan:**
1. Extract service handlers to separate modules
2. Use shared runtime or better thread management
3. Replace magic numbers with constants
4. Proper error handling and propagation

#### 5. `ModelManager` - Model Type Handling

**Problems:**
- `ModelType::Stt` and `ModelType::Vad` defined but never used
- Only `Whisper` and `Tts` are actually used
- Multiple model types handled with match statements (could use trait)

**Refactoring Plan:**
1. Remove unused model types
2. Or: Use trait-based approach for extensibility

---

## 🗑️ Dead Code Identification

### Code to Remove

1. **CLI Commands** (`src/main.rs`)
   - `Commands::Tts` variant and handler
   - `Commands::Stt` variant and handler
   - `format_timestamp()` in main.rs (use shared utility)
   - All CLI-specific code (keep only `Commands::Daemon`)

2. **Unused Model Types** (`src/ui/audio/model_manager.rs`)
   - `ModelType::Stt` - not used, only `Whisper` is used
   - `ModelType::Vad` - defined but never actually used

3. **Unused Methods**
   - `SttService::cancel()` - check if used (only in tests?)
   - `SttService::play_beep_blocking()` - use async version instead
   - `TtsService::stop()` - check if used in daemon
   - `TtsService::set_language()` - partial implementation, doesn't work properly

4. **Unused Dependencies** (confirmed unused in codebase)
   - `flate2` - in Cargo.toml but never imported/used (REMOVE)
   - `async-trait` - in Cargo.toml but never imported/used (REMOVE)
   - `hound` - in Cargo.toml but never imported/used (REMOVE)
   - `clap` - used for CLI parsing; keep if daemon needs command-line args, otherwise remove
   - `wrtype` - only used in `stt_type`, keep if that feature stays
   - `enigo` - only used in `stt_type`, keep if that feature stays

5. **Dead Code Paths**
   - `SttService::validate_model_files()` - only called once, could inline
   - Multiple state lock/unlock patterns that could be simplified

### Code to Simplify

1. **Callback System**
   - Current: `Arc<Mutex<Option<Box<dyn Fn(...)>>>>` - complex and hard to test
   - Better: Use channels or event emitter pattern
   - Or: Keep but add helper methods to reduce boilerplate

2. **State Management**
   - `SttState` and `TtsState` are simple - consider using `Arc<RwLock<>>` instead of `Arc<Mutex<>>` for read-heavy access
   - Or: Use `tokio::sync::RwLock` for async-friendly reads

---

## 🐛 Potential Bugs

### 1. Race Condition in `stop_listening()`

**Location:** `src/ui/audio/stt_service.rs:734-830`

**Issue:**
```rust
// Drops recognizer
{
    let mut recognizer_guard = self.recognizer.lock().unwrap();
    recognizer_guard.take();
}

// Then polls for decode result
while start_wait.elapsed() < max_wait {
    let state = self.state.lock().unwrap();
    text = state.current_text.clone();
    // ...
}
```

**Problem:** If decode is still in progress when recognizer is dropped, it could cause undefined behavior or panic.

**Fix:** Wait for decode completion signal before dropping recognizer.

### 2. Memory Leak in Audio Thread

**Location:** `src/ui/audio/stt_service.rs:318-359`

**Issue:** Audio thread uses `thread.park_timeout()` but thread handle stored in `Arc<Mutex<>>`. If service is dropped, thread might not be properly cleaned up.

**Fix:** Use proper cancellation token or ensure thread is joined on drop.

### 3. Timeout Logic Issue in DBus Handler

**Location:** `src/dbus/service.rs:98-102`

**Issue:**
```rust
check_count += 1;
if check_count > 600 {
    // Timeout after 600 * 100ms = 60 seconds
}
```

**Problem:** Magic number, no clear timeout constant, hard to understand.

**Fix:** Extract to constant: `const STT_TIMEOUT_MS: u64 = 60000; const STT_CHECK_INTERVAL_MS: u64 = 100;`

### 4. Double Beep Playback

**Location:** Multiple places play low beep
- `stt_service.rs:504` - in pause detection
- `stt_service.rs:758` - in `stop_listening()`

**Problem:** If pause is detected, low beep plays, then `stop_listening()` is called, which plays beep again.

**Fix:** Track if beep was already played, or extract beep playback to single method that checks state.

### 5. Incomplete Language Support

**Location:** `src/ui/audio/tts_service.rs:276-291`

**Issue:** `set_language()` marks service as uninitialized but doesn't actually reinitialize. Next call to `speak()` will reinit, but language change is not properly handled.

**Fix:** Either remove `set_language()` or implement proper reinitialization.

---

## 📝 Refactoring Steps (Execution Plan)

### Phase 1: Preparation & Dead Code Removal

1. **Remove CLI Commands**
   - [ ] Simplify `main.rs` to only handle `Daemon` command
   - [ ] Remove `Commands::Tts` and `Commands::Stt`
   - [ ] Remove CLI-specific imports and dependencies if not needed

2. **Remove Unused Model Types**
   - [ ] Audit usage of `ModelType::Stt` and `ModelType::Vad`
   - [ ] Remove if confirmed unused
   - [ ] Or document why they're kept for future use

3. **Remove Dead Methods**
   - [ ] Check `SttService::cancel()` usage - remove if unused
   - [ ] Remove `play_beep_blocking()`, use async version
   - [ ] Check `TtsService::stop()` and `set_language()` usage

### Phase 2: Extract Utilities & Shared Code

4. **Create Shared Utilities**
   - [ ] Create `src/utils/logging.rs` with `format_timestamp()`
   - [ ] Create `src/utils/beep.rs` for beep playback
   - [ ] Update all references to use shared utilities

5. **Extract Constants**
   - [ ] Extract magic numbers to named constants
   - [ ] Group related constants (audio thresholds, timeouts, etc.)

### Phase 3: Break Down Complex Functions

6. **Refactor `audio_capture_loop()`**
   - [ ] Extract `AudioCapture` struct (device/stream management)
   - [ ] Extract `PauseDetector` struct (silence detection logic)
   - [ ] Extract `AudioProcessor` (resampling, conversion)
   - [ ] Simplify main loop to orchestrate these components

7. **Refactor `stop_listening()`**
   - [ ] Replace polling with proper async coordination
   - [ ] Extract cleanup logic to `cleanup_audio_resources()`
   - [ ] Ensure proper sequencing (wait for decode, then cleanup)

8. **Refactor `start_listening()`**
   - [ ] Extract `RecordingSession` to manage lifecycle
   - [ ] Simplify beep/recording coordination
   - [ ] Improve error handling

### Phase 4: Improve State Management

9. **Simplify State Access**
   - [ ] Add helper methods to reduce lock boilerplate
   - [ ] Consider `RwLock` for read-heavy state access
   - [ ] Document state invariants

10. **Fix Callback System**
    - [ ] Add helper methods for callback invocation
    - [ ] Or: Refactor to use channels/events

### Phase 5: Fix Bugs & Improve Error Handling

11. **Fix Race Conditions**
    - [ ] Fix `stop_listening()` recognizer drop race
    - [ ] Fix audio thread cleanup
    - [ ] Fix double beep playback

12. **Improve Error Handling**
    - [ ] Add proper error types (instead of `anyhow::Error` everywhere)
    - [ ] Improve error messages
    - [ ] Add error recovery where possible

### Phase 6: Reorganize Architecture

13. **Reorganize Modules**
    - [ ] Create `daemon/` directory for DBus service
    - [ ] Create `services/` directory with subdirectories
    - [ ] Create `utils/` directory
    - [ ] Move code to appropriate locations

14. **Add Documentation**
    - [ ] Add module-level documentation
    - [ ] Add function documentation
    - [ ] Document complex algorithms (pause detection, etc.)
    - [ ] Add architecture decision records (ADRs)

### Phase 7: Testing & Validation

15. **Add Unit Tests**
    - [ ] Test extracted components (PauseDetector, AudioProcessor, etc.)
    - [ ] Test state management
    - [ ] Test error handling

16. **Integration Testing**
    - [ ] Test daemon startup and shutdown
    - [ ] Test DBus method calls
    - [ ] Test error recovery

---

## 📏 Code Complexity Limits

### Function Complexity Guidelines

- **Maximum function length:** 50 lines (excluding comments/blank lines)
- **Maximum nesting depth:** 3 levels (if/else/match)
- **Maximum parameters:** 5 parameters (use struct for more)
- **Cyclomatic complexity:** < 10

### Current Violations

1. `audio_capture_loop()` - **300+ lines**, **cyclomatic complexity ~25**
2. `stop_listening()` - **100+ lines**, **cyclomatic complexity ~12**
3. `decode_accumulated_audio()` - **90+ lines**, **cyclomatic complexity ~8**

---

## 🎨 Code Style Improvements

### Comments to Add

1. **Module-level documentation**
   - Purpose of each module
   - Key concepts and patterns used

2. **Function documentation**
   - All public functions should have doc comments
   - Document parameters, return values, errors
   - Document side effects

3. **Complex algorithm documentation**
   - Pause detection algorithm
   - Audio processing pipeline
   - State machine transitions

4. **TODO/FIXME cleanup**
   - Remove or address all TODO comments
   - Convert FIXMEs to issues or fix them

### Naming Improvements

1. Use more descriptive names:
   - `recording_started` → `recording_active_flag` or `is_recording_active`
   - `check_count` → `timeout_check_count` or `iteration_count`

2. Extract magic values to named constants:
   - `600` → `MAX_STT_TIMEOUT_CHECKS`
   - `100` → `STT_CHECK_INTERVAL_MS`

---

## 🔄 Migration Strategy

### Incremental Refactoring

1. **Start with low-risk changes**
   - Extract utilities
   - Remove dead code
   - Add constants

2. **Then tackle complex refactoring**
   - Break down large functions
   - Extract components

3. **Finally reorganize structure**
   - Move modules
   - Update imports

### Testing Strategy

- Run existing tests after each phase
- Add new tests for extracted components
- Manual testing of daemon after each phase

---

## ✅ Success Criteria

After refactoring, the codebase should:

1. ✅ **Only support Daemon mode** - no CLI commands
2. ✅ **All functions < 50 lines** - complex logic extracted
3. ✅ **No code duplication** - shared utilities used
4. ✅ **No dead code** - all code paths used
5. ✅ **Clear architecture** - easy to understand structure
6. ✅ **Well documented** - comments explain complex logic
7. ✅ **No known bugs** - race conditions fixed
8. ✅ **Testable** - components can be tested in isolation

---

## 📚 References

- Clean Architecture (Robert C. Martin)
- SOLID Principles
- Rust API Guidelines: https://rust-lang.github.io/api-guidelines/
- Clippy lints for complexity: `cargo clippy -- -W clippy::cognitive_complexity`

---

**Document Version:** 1.0  
**Last Updated:** 2024  
**Status:** Planning Phase


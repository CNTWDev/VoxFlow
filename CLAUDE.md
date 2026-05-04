# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Development (launches Tauri window with hot-reload)
cargo tauri dev
RUST_LOG=debug cargo tauri dev   # verbose logging

# Build all crates (compile check without Tauri)
cargo build --workspace

# Tests — no microphone required (run in CI)
cargo test --workspace

# Run a single test
cargo test -p vf-core test_initial_state

# Integration tests — require a physical microphone (run manually)
cargo test --workspace -- --ignored

# Production bundle
cargo tauri build
# Output: src-tauri/target/release/bundle/
```

No Node.js is required — the frontend is plain HTML served by WebKit.

## Architecture

Cargo workspace with five library crates plus the Tauri shell. Dependency order:

```
vf-config  ←  vf-core  →  vf-asr
                ↑               ↑
           vf-audio        vf-inject
                ↑
           src-tauri (Tauri shell + frontend HTML)
```

None of the `vf-*` crates depend on Tauri; they compile and test independently.

### Data flow

1. **vf-audio** (`crates/vf-audio/`) — captures raw PCM from the default microphone via `cpal`, resamples to 16 kHz mono f32 using `rubato`. Resampling is done after accumulation (not per-chunk) to avoid zero-padding artefacts.

2. **vf-asr** (`crates/vf-asr/`) — defines the `AsrBackend` trait (`prepare` / `transcribe`). `CloudBackend` calls the OpenAI Transcription API (`gpt-4o-transcribe`). Local Whisper.cpp backend is stubbed (Phase 7).

3. **vf-inject** (`crates/vf-inject/`) — writes text to the clipboard then simulates Cmd+V (macOS) / Ctrl+V (Windows) via `enigo`. Optionally restores the prior clipboard contents after injection.

4. **vf-config** (`crates/vf-config/`) — `AppConfig` holds `AppSettings`, `AudioConfig`, `InjectConfig`, and a `HashMap<String, LanguageProfile>`. Serialized as TOML to the OS config directory. Each `LanguageProfile` has a `ProfileBackendConfig` (Cloud or Local) plus optional `language_hint` and `prompt`.

5. **vf-core** (`crates/vf-core/`) — `VoxEngine` is the central orchestrator. It runs in a **dedicated OS thread** with a `current_thread` Tokio runtime and a `LocalSet`, which is required because `AudioCapture` is `!Send`. The public API is a cheap-to-clone handle that sends `EngineCommand` messages over an `mpsc` channel.

   State machine: `Idle → Recording → Processing → Injecting → Idle`. State is broadcast via a `tokio::sync::watch` channel; events (`Transcription`, `Error`) are broadcast via a `tokio::sync::broadcast` channel.

   The backend is cached per profile-id: switching the active profile invalidates the cache and rebuilds the backend on the next recording.

6. **src-tauri** (`src-tauri/`) — Tauri shell. `lib.rs` initialises `VoxEngine`, subscribes to its event broadcast, and forwards events to the WebKit frontend as Tauri events under the channel `vox://event`. Six Tauri commands in `commands.rs` wrap the `VoxEngine` public API.

   The frontend (`src/index.html`) is a single self-contained HTML file. It calls `invoke()` for commands and `listen('vox://event', ...)` for events, plus a 1.5 s poll as a fallback.

### Testing approach

`vf-core` tests use `MockAsrBackend` (see `engine.rs`) with `VoxEngine::new_for_test()`. Tests that only exercise the state machine (no audio device) run without `#[ignore]`. Tests that call `start_recording` (which opens the microphone) are marked `#[ignore]` and must be run manually.

Config round-trip and serde tests live in `vf-config/src/settings.rs`.

### Platform notes

- macOS requires Microphone and Accessibility permissions for audio capture and text injection respectively.
- The API key is stored in plain text in the TOML config until Phase 9 (Keychain integration).
- Unsigned builds on macOS may be blocked by Gatekeeper; run `xattr -cr "Vox Flow.app"` to bypass.
- Windows text injection via `enigo` does not work against elevated (admin) windows or anti-cheat processes — this is a known limitation.

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
           src-tauri (Tauri shell + frontend HTML + overlay + hotkeys + permissions)
```

None of the `vf-*` crates depend on Tauri; they compile and test independently.

### Data flow

1. **vf-audio** (`crates/vf-audio/`) — captures raw PCM from the default microphone via `cpal`, resamples to 16 kHz mono f32 using `rubato`. Resampling is done after accumulation (not per-chunk) to avoid zero-padding artefacts.

2. **vf-asr** (`crates/vf-asr/`) — defines the `AsrBackend` trait (`prepare` / `transcribe` / optional `start_stream`). Three concrete backends:
   - `CloudBackend` / `OpenAiBackend` — OpenAI Transcription API (`gpt-4o-transcribe`, etc.)
   - `VoxNexusBackend` — `https://api.voxnexus.ai/v1/stt`, supports timestamps and speaker diarization
   - Local Whisper.cpp — stubbed (Phase 7)
   
   `AsrProviderDescriptor` + `AsrCapabilities` describe each provider's feature set for the UI. `StreamingTranscriber` trait + `StreamEvent` enum are defined for future WebSocket streaming (not yet wired into `VoxEngine`).

3. **vf-inject** (`crates/vf-inject/`) — writes text to the clipboard then simulates Cmd+V (macOS) / Ctrl+V (Windows) via `enigo`. Optionally restores the prior clipboard contents after injection.

4. **vf-config** (`crates/vf-config/`) — `AppConfig` holds `AppSettings`, `AudioConfig`, `InjectConfig`, and a `HashMap<String, LanguageProfile>`. Serialized as TOML to the OS config directory. Each `LanguageProfile` has a `ProfileBackendConfig` — a tagged enum with variants `OpenAi` (alias `Cloud` for backwards compat), `VoxNexus`, and `Local`.

5. **vf-core** (`crates/vf-core/`) — `VoxEngine` is the central orchestrator. It runs in a **dedicated OS thread** with a `current_thread` Tokio runtime and a `LocalSet`, which is required because `AudioCapture` is `!Send`. The public API is a cheap-to-clone handle that sends `EngineCommand` messages over an `mpsc` channel.

   State machine: `Idle → Recording → Processing → Injecting → Idle`. State is broadcast via a `tokio::sync::watch` channel; events (`StateChanged`, `Transcription`, `Error`) are broadcast via a `tokio::sync::broadcast` channel. The ASR backend is cached per profile-id; switching profiles invalidates the cache.

6. **src-tauri** (`src-tauri/`) — Tauri shell with four internal modules:
   - `commands.rs` — ten Tauri commands wrapping `VoxEngine` plus `check_system_permissions` / `open_system_permission_settings`
   - `overlay.rs` — creates a transparent, always-on-top, non-focusable, click-through `WebviewWindow` ("overlay") positioned 96 px above the bottom of the working area
   - `platform_hotkey.rs` — macOS-only `CGEventTap` on `FlagsChanged` events to implement Fn-key hold detection (runs its own `CFRunLoop` thread); the standard `tauri_plugin_global_shortcut` handles all other hotkeys cross-platform
   - `permissions.rs` — checks macOS Input Monitoring (`CGPreflightListenEventAccess`) and Accessibility (`AXIsProcessTrusted`) via raw FFI; emits `vox://permissions` event on startup; `open_system_permission_settings` opens the correct System Settings pane

   All engine broadcast events are forwarded to the frontend as `vox://event` Tauri events. The main window (`src/index.html`) handles recording control; the overlay window (`src/overlay.html`) renders the status pill.

### Testing approach

`vf-core` tests use `MockAsrBackend` (see `engine.rs`) with `VoxEngine::new_for_test()`. Tests that only exercise the state machine (no audio device) run without `#[ignore]`. Tests that call `start_recording` (which opens the microphone) are marked `#[ignore]` and must be run manually.

Config round-trip and serde tests live in `vf-config/src/settings.rs`.

### Platform notes

- macOS requires **Microphone**, **Accessibility**, and (for Fn-key) **Input Monitoring** permissions.
- Using `hotkey = "Fn"` routes through the `CGEventTap` path; any other hotkey string goes through `tauri_plugin_global_shortcut`.
- The API key is stored in plain text in the TOML config until Phase 9 (Keychain integration).
- Unsigned builds on macOS may be blocked by Gatekeeper; run `xattr -cr "Vox Flow.app"` to bypass.
- Windows text injection via `enigo` does not work against elevated (admin) windows or anti-cheat processes — this is a known limitation.

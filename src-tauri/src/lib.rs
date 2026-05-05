use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter, Runtime};
use vf_config::{ActivationMode, AppConfig};
use vf_core::VoxEngine;

mod commands;
mod overlay;
mod permissions;
mod platform_hotkey;

pub struct AppState {
    pub engine: VoxEngine,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| {
                    tracing_subscriber::EnvFilter::new("info,vf_core=debug,vf_audio=debug")
                }),
        )
        .init();

    let config = AppConfig::load();
    let initial_hotkey = config.app.hotkey.clone();
    let engine = VoxEngine::new(config);

    // Subscribe to engine events BEFORE building the Tauri app so we don't miss any.
    let mut event_rx = engine.subscribe_events();

    // Stop flag shared between the Fn-hotkey thread and the window-close handler.
    let hotkey_stop: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let hotkey_stop_handler = Arc::clone(&hotkey_stop);
    let hotkey_stop_setup = Arc::clone(&hotkey_stop);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState { engine })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::get_config,
            commands::set_config,
            commands::get_asr_providers,
            commands::set_active_profile,
            commands::start_recording,
            commands::stop_recording,
            commands::cancel_recording,
            commands::check_system_permissions,
            commands::open_system_permission_settings,
        ])
        // Signal the Fn-hotkey thread to exit when the main window is destroyed.
        .on_window_event(move |window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::Destroyed = event {
                    hotkey_stop_handler.store(true, Ordering::Relaxed);
                }
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let hotkey = initial_hotkey.clone();

            let shortcut_builder =
                tauri_plugin_global_shortcut::Builder::new().with_handler(handle_global_shortcut);
            // "Fn" is handled exclusively by platform_hotkey (CGEventTap); skip plugin registration.
            let shortcut_plugin = if hotkey.to_lowercase() == "fn" {
                shortcut_builder.build()
            } else {
                match shortcut_builder.with_shortcut(hotkey.as_str()) {
                    Ok(builder) => builder.build(),
                    Err(e) => {
                        tracing::warn!("global hotkey '{hotkey}' could not be registered: {e}");
                        tauri_plugin_global_shortcut::Builder::new()
                            .with_handler(handle_global_shortcut)
                            .build()
                    }
                }
            };
            app.handle().plugin(shortcut_plugin)?;
            platform_hotkey::install_platform_hotkeys(app.handle().clone(), hotkey_stop_setup);
            overlay::install_overlay(app)?;
            permissions::emit_system_permissions(app.handle());

            // Forward engine events → Tauri frontend events
            tauri::async_runtime::spawn(async move {
                loop {
                    match event_rx.recv().await {
                        Ok(event) => {
                            match &event {
                                vf_core::EngineEvent::StateChanged { state } => {
                                    tracing::info!("emitting state changed: {state:?}");
                                }
                                vf_core::EngineEvent::Transcription { text, .. } => {
                                    tracing::info!("emitting transcription: {text:?}");
                                }
                                vf_core::EngineEvent::Error { message, .. } => {
                                    tracing::error!("emitting error: {message}");
                                }
                            }
                            overlay::forward_event_to_overlay(&handle, &event);
                            if let Err(e) = handle.emit_to("main", "vox://event", &event) {
                                tracing::error!("tauri emit failed: {e}");
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("event receiver lagged by {n}");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::warn!("event channel closed");
                            break;
                        }
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn handle_global_shortcut<R: Runtime>(
    app: &AppHandle<R>,
    _shortcut: &tauri_plugin_global_shortcut::Shortcut,
    event: tauri_plugin_global_shortcut::ShortcutEvent,
) {
    use tauri::Manager;
    use tauri_plugin_global_shortcut::ShortcutState;

    let engine = app.state::<AppState>().engine.clone();
    tauri::async_runtime::spawn(async move {
        let config = engine.get_config().await;
        match (config.app.activation_mode, event.state) {
            (ActivationMode::HoldKey, ShortcutState::Pressed) => {
                let _ = engine.start_recording().await;
            }
            (ActivationMode::HoldKey, ShortcutState::Released) => {
                let _ = engine.stop_recording().await;
            }
            (ActivationMode::ToggleKey, ShortcutState::Pressed) => {
                if matches!(engine.state(), vf_core::RecorderState::Idle) {
                    let _ = engine.start_recording().await;
                } else if matches!(engine.state(), vf_core::RecorderState::Recording) {
                    let _ = engine.stop_recording().await;
                }
            }
            _ => {}
        }
    });
}

use tauri::{AppHandle, Manager, Runtime};

use crate::AppState;

#[derive(Debug, Clone, Copy)]
enum PlatformHotkeyEvent {
    Pressed,
    Released,
}

pub fn install_platform_hotkeys<R: Runtime>(app: AppHandle<R>) {
    #[cfg(target_os = "macos")]
    install_macos_fn_hotkey(app);

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
}

#[cfg(target_os = "macos")]
fn install_macos_fn_hotkey<R: Runtime>(app: AppHandle<R>) {
    use core_foundation::runloop::CFRunLoop;
    use core_graphics::event::{
        CallbackResult, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions,
        CGEventTapPlacement, CGEventType, EventField,
    };
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    const MAC_FN_KEYCODE: i64 = 63;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PlatformHotkeyEvent>();
    let engine = app.state::<AppState>().engine.clone();

    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let config = engine.get_config().await;
            if config.app.hotkey.to_lowercase() != "fn" {
                continue;
            }
            match event {
                PlatformHotkeyEvent::Pressed => {
                    if matches!(engine.state(), vf_core::RecorderState::Idle) {
                        tracing::info!("macOS Fn pressed: starting recording");
                        let _ = engine.start_recording().await;
                    }
                }
                PlatformHotkeyEvent::Released => {
                    if matches!(engine.state(), vf_core::RecorderState::Recording) {
                        tracing::info!("macOS Fn released: stopping recording");
                        let _ = engine.stop_recording().await;
                    }
                }
            }
        }
    });

    std::thread::Builder::new()
        .name("vox-flow-macos-fn-hotkey".to_string())
        .spawn(move || {
            let pressed = Arc::new(AtomicBool::new(false));
            let callback_pressed = Arc::clone(&pressed);
            let callback_tx = tx.clone();

            let tap = CGEventTap::new(
                CGEventTapLocation::HID,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                vec![CGEventType::FlagsChanged],
                move |_proxy, event_type, event| {
                    if !matches!(event_type, CGEventType::FlagsChanged) {
                        return CallbackResult::Keep;
                    }

                    let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                    if keycode != MAC_FN_KEYCODE {
                        return CallbackResult::Keep;
                    }

                    let fn_is_down = event.get_flags().contains(CGEventFlags::CGEventFlagSecondaryFn);
                    let was_down = callback_pressed.swap(fn_is_down, Ordering::SeqCst);
                    match (was_down, fn_is_down) {
                        (false, true) => {
                            let _ = callback_tx.send(PlatformHotkeyEvent::Pressed);
                        }
                        (true, false) => {
                            let _ = callback_tx.send(PlatformHotkeyEvent::Released);
                        }
                        _ => {}
                    }

                    CallbackResult::Keep
                },
            );

            let Ok(event_tap) = tap else {
                tracing::warn!(
                    "macOS Fn event tap could not be installed; grant Input Monitoring permission or use another global hotkey"
                );
                return;
            };

            let Ok(loop_source) = event_tap.mach_port().create_runloop_source(0) else {
                tracing::warn!("macOS Fn event tap run loop source could not be created");
                return;
            };

            let run_loop = CFRunLoop::get_current();
            run_loop.add_source(
                &loop_source,
                unsafe { core_foundation::runloop::kCFRunLoopCommonModes },
            );
            event_tap.enable();
            tracing::info!("macOS Fn hold hotkey listener installed");
            CFRunLoop::run_current();
        })
        .map(|_| ())
        .unwrap_or_else(|e| tracing::warn!("failed to spawn macOS Fn hotkey listener: {e}"));
}

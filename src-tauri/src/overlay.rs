use tauri::{Manager, PhysicalPosition, Runtime, WebviewUrl, WebviewWindowBuilder};

const OVERLAY_WIDTH: f64 = 380.0;
const OVERLAY_HEIGHT: f64 = 108.0;
const OVERLAY_BOTTOM_MARGIN: i32 = 96;

pub fn install_overlay<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let overlay = WebviewWindowBuilder::new(
        app,
        "overlay",
        WebviewUrl::App("overlay.html".into()),
    )
    .title("Vox Flow Status")
    .inner_size(OVERLAY_WIDTH, OVERLAY_HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .focusable(false)
    .visible(true)
    .build()?;

    if let Err(e) = overlay.set_ignore_cursor_events(true) {
        tracing::warn!("overlay click-through could not be enabled: {e}");
    }

    position_overlay(&overlay);
    Ok(())
}

pub fn forward_event_to_overlay<R: Runtime>(handle: &tauri::AppHandle<R>, event: &vf_core::EngineEvent) {
    if let Some(overlay) = handle.get_webview_window("overlay") {
        position_overlay(&overlay);
    }
    let _ = event;
}

fn position_overlay<R: Runtime>(overlay: &tauri::WebviewWindow<R>) {
    let monitor = overlay
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| overlay.primary_monitor().ok().flatten());

    let Some(monitor) = monitor else {
        return;
    };

    let scale = monitor.scale_factor();
    let width = (OVERLAY_WIDTH * scale).round() as i32;
    let height = (OVERLAY_HEIGHT * scale).round() as i32;
    let work_area = monitor.work_area();
    let x = work_area.position.x + ((work_area.size.width as i32 - width) / 2);
    let y = work_area.position.y + work_area.size.height as i32 - height - OVERLAY_BOTTOM_MARGIN;

    if let Err(e) = overlay.set_position(PhysicalPosition::new(x, y)) {
        tracing::warn!("overlay position update failed: {e}");
    }
}

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

#[derive(Debug, Clone, Serialize)]
pub struct SystemPermissionStatus {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub granted: bool,
}

pub fn check_system_permissions() -> Vec<SystemPermissionStatus> {
    #[cfg(target_os = "macos")]
    {
        vec![
            SystemPermissionStatus {
                id: "input_monitoring",
                title: "输入监控",
                description: "用于在任何应用中监听 Fn 长按，开始和结束录音。",
                granted: macos::input_monitoring_granted(),
            },
            SystemPermissionStatus {
                id: "accessibility",
                title: "辅助功能",
                description: "用于把转写后的文字粘贴到当前焦点应用。",
                granted: macos::accessibility_granted(),
            },
        ]
    }

    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

pub fn emit_system_permissions<R: Runtime>(handle: &AppHandle<R>) {
    let statuses = check_system_permissions();
    if statuses.iter().any(|status| !status.granted) {
        tracing::warn!("system permissions missing: {statuses:?}");
    }
    if let Err(e) = handle.emit("vox://permissions", statuses) {
        tracing::warn!("permission event emit failed: {e}");
    }
}

pub fn open_system_permission_settings(permission: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::open_system_permission_settings(permission)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = permission;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightListenEventAccess() -> bool;
    }

    pub fn accessibility_granted() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    pub fn input_monitoring_granted() -> bool {
        unsafe { CGPreflightListenEventAccess() }
    }

    pub fn open_system_permission_settings(permission: &str) -> Result<(), String> {
        let pane = match permission {
            "accessibility" => "Privacy_Accessibility",
            "input_monitoring" => "Privacy_ListenEvent",
            other => return Err(format!("unknown permission: {other}")),
        };
        let url = format!("x-apple.systempreferences:com.apple.preference.security?{pane}");
        std::process::Command::new("open")
            .arg(url)
            .status()
            .map_err(|e| e.to_string())
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("open system settings failed with status {status}"))
                }
            })
    }
}

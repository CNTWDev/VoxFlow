use tauri::State;
use vf_asr::AsrProviderDescriptor;
use vf_config::AppConfig;
use vf_core::RecorderState;
use crate::{permissions, AppState};

#[tauri::command]
pub async fn get_state(state: State<'_, AppState>) -> Result<RecorderState, String> {
    Ok(state.engine.state())
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.engine.get_config().await)
}

#[tauri::command]
pub async fn set_config(
    new_config: AppConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.engine.set_config(new_config).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_asr_providers() -> Result<Vec<AsrProviderDescriptor>, String> {
    Ok(vf_asr::provider_descriptors())
}

#[tauri::command]
pub async fn set_active_profile(
    profile_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.engine.set_active_profile(profile_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_recording(state: State<'_, AppState>) -> Result<(), String> {
    state.engine.start_recording().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_recording(state: State<'_, AppState>) -> Result<(), String> {
    state.engine.stop_recording().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cancel_recording(state: State<'_, AppState>) -> Result<(), String> {
    state.engine.cancel().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_system_permissions() -> Result<Vec<permissions::SystemPermissionStatus>, String> {
    Ok(permissions::check_system_permissions())
}

#[tauri::command]
pub async fn open_system_permission_settings(permission: String) -> Result<(), String> {
    permissions::open_system_permission_settings(&permission)
}

use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use directories::ProjectDirs;
use crate::profile::{AsrTransportKind, LanguageProfile, ProfileBackendConfig, ValidationWarning};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationMode {
    HoldKey,
    ToggleKey,
}

impl Default for ActivationMode {
    fn default() -> Self { Self::HoldKey }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default = "default_max_recording_secs")]
    pub max_recording_secs: u32,
    #[serde(default = "default_trailing_silence_ms")]
    pub trailing_silence_ms: u32,
}

fn default_max_recording_secs() -> u32 { 120 }
fn default_trailing_silence_ms() -> u32 { 800 }

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            max_recording_secs: default_max_recording_secs(),
            trailing_silence_ms: default_trailing_silence_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectConfig {
    #[serde(default = "default_paste_delay_ms")]
    pub paste_delay_ms: u64,
    #[serde(default = "default_post_paste_delay_ms")]
    pub post_paste_delay_ms: u64,
    #[serde(default = "default_true")]
    pub restore_text_clipboard: bool,
}

fn default_paste_delay_ms() -> u64 { 50 }
fn default_post_paste_delay_ms() -> u64 { 150 }
fn default_true() -> bool { true }

impl Default for InjectConfig {
    fn default() -> Self {
        Self {
            paste_delay_ms: default_paste_delay_ms(),
            post_paste_delay_ms: default_post_paste_delay_ms(),
            restore_text_clipboard: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_activation_mode")]
    pub activation_mode: ActivationMode,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default)]
    pub launch_at_startup: bool,
    #[serde(default)]
    pub provider_keys: HashMap<String, String>,
    #[serde(default)]
    pub openai_api_key: String,
    #[serde(default)]
    pub voxnexus_api_key: String,
    #[serde(default)]
    pub global_api_key: String,
}

fn default_activation_mode() -> ActivationMode { ActivationMode::HoldKey }

#[cfg(target_os = "macos")]
fn default_hotkey() -> String {
    "Fn".to_string()
}

#[cfg(target_os = "windows")]
fn default_hotkey() -> String {
    "Ctrl+Alt+Space".to_string()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn default_hotkey() -> String {
    "Ctrl+Alt+Space".to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            activation_mode: default_activation_mode(),
            hotkey: default_hotkey(),
            launch_at_startup: false,
            provider_keys: HashMap::new(),
            openai_api_key: String::new(),
            voxnexus_api_key: String::new(),
            global_api_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_active_profile_id")]
    pub active_profile_id: String,
    #[serde(default = "default_profiles")]
    pub profiles: HashMap<String, LanguageProfile>,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub inject: InjectConfig,
    #[serde(default)]
    pub app: AppSettings,
}

fn default_active_profile_id() -> String { "auto".to_string() }

fn default_profiles() -> HashMap<String, LanguageProfile> {
    let mut map = HashMap::new();
        map.insert("auto".to_string(), LanguageProfile {
            id: "auto".to_string(),
            display_name: "Auto Detect".to_string(),
            language_hint: None,
            backend: ProfileBackendConfig::OpenAi {
                model: "gpt-4o-transcribe".to_string(),
                api_key: None,
                transport: AsrTransportKind::RestBatch,
            },
            prompt: None,
        });
        map.insert("zh_voxnexus".to_string(), LanguageProfile {
            id: "zh_voxnexus".to_string(),
            display_name: "中文 · VoxNexus".to_string(),
            language_hint: Some("zh-CN".to_string()),
            backend: ProfileBackendConfig::VoxNexus {
                model_id: "vn-stt-ultra".to_string(),
                api_key: None,
                transport: AsrTransportKind::RestBatch,
                enable_timestamps: false,
                enable_speaker_diarization: false,
            },
            prompt: None,
        });
        map
    }

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            active_profile_id: default_active_profile_id(),
            profiles: default_profiles(),
            audio: AudioConfig::default(),
            inject: InjectConfig::default(),
            app: AppSettings::default(),
        }
    }
}

impl AppConfig {
    pub fn config_path() -> Option<PathBuf> {
        ProjectDirs::from("com", "VoxFlow", "VoxFlow")
            .map(|d| d.config_dir().join("config.toml"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            tracing::warn!("cannot determine config directory, using defaults");
            return Self::default();
        };
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).map(|mut config: Self| {
                config.normalize();
                config
            }).unwrap_or_else(|e| {
                tracing::error!("config parse error: {e}, using defaults");
                Self::default()
            }),
            Err(e) => {
                tracing::error!("config read error: {e}, using defaults");
                Self::default()
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path().ok_or_else(|| anyhow::anyhow!("no config dir"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn active_profile(&self) -> Option<&LanguageProfile> {
        self.profiles.get(&self.active_profile_id)
    }

    pub fn migrate_legacy_fields(&mut self) {
        if self.app.openai_api_key.is_empty() && !self.app.global_api_key.is_empty() {
            self.app.openai_api_key = self.app.global_api_key.clone();
        }
        if !self.app.openai_api_key.is_empty() {
            self.app.provider_keys
                .entry("openai".to_string())
                .or_insert_with(|| self.app.openai_api_key.clone());
        }
        if !self.app.voxnexus_api_key.is_empty() {
            self.app.provider_keys
                .entry("voxnexus".to_string())
                .or_insert_with(|| self.app.voxnexus_api_key.clone());
        }
    }

    pub fn normalize(&mut self) {
        self.migrate_legacy_fields();
        if self.audio.max_recording_secs == 0 {
            tracing::warn!(
                "max_recording_secs was 0 in config; resetting to {}",
                default_max_recording_secs()
            );
            self.audio.max_recording_secs = default_max_recording_secs();
        }
        if self.audio.trailing_silence_ms == 0 {
            self.audio.trailing_silence_ms = default_trailing_silence_ms();
        }
        if self.inject.paste_delay_ms == 0 {
            self.inject.paste_delay_ms = default_paste_delay_ms();
        }
        if self.inject.post_paste_delay_ms == 0 {
            self.inject.post_paste_delay_ms = default_post_paste_delay_ms();
        }
        self.normalize_platform_hotkey();
    }

    #[cfg(target_os = "macos")]
    fn normalize_platform_hotkey(&mut self) {
        if self.app.hotkey.eq_ignore_ascii_case("fn") {
            self.app.activation_mode = ActivationMode::HoldKey;
        }
    }

    #[cfg(target_os = "windows")]
    fn normalize_platform_hotkey(&mut self) {
        if self.app.hotkey.eq_ignore_ascii_case("fn") {
            self.app.hotkey = default_hotkey();
            self.app.activation_mode = ActivationMode::HoldKey;
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn normalize_platform_hotkey(&mut self) {
        if self.app.hotkey.trim().is_empty() {
            self.app.hotkey = default_hotkey();
        }
    }

    pub fn provider_key(&self, provider_id: &str) -> Option<&str> {
        self.app.provider_keys.get(provider_id)
            .map(String::as_str)
            .filter(|key| !key.is_empty())
    }

    pub fn validate(&self) -> Vec<ValidationWarning> {
        let mut warnings = Vec::new();
        for profile in self.profiles.values() {
            if let ProfileBackendConfig::Local { model_path, .. } = &profile.backend {
                let path_str = model_path.to_string_lossy().to_lowercase();
                let is_en_model = path_str.contains(".en.bin") || path_str.contains("-en.");
                if is_en_model {
                    match &profile.language_hint {
                        None => warnings.push(ValidationWarning::EnModelForAutoDetect {
                            profile_id: profile.id.clone(),
                            model_path: model_path.clone(),
                        }),
                        Some(hint) if hint != "en" => {
                            warnings.push(ValidationWarning::EnModelForNonEnglish {
                                profile_id: profile.id.clone(),
                                model_path: model_path.clone(),
                            })
                        }
                        _ => {}
                    }
                }
            }
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn default_config_roundtrips_toml() {
        let config = AppConfig::default();
        let toml_str = toml::to_string_pretty(&config).expect("serialize");
        let back: AppConfig = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back.active_profile_id, config.active_profile_id);
        assert!(back.profiles.contains_key("auto"));
    }

    #[test]
    fn cloud_profile_serde() {
        let profile = LanguageProfile {
            id: "zh".into(),
            display_name: "中文".into(),
            language_hint: Some("zh".into()),
            backend: ProfileBackendConfig::OpenAi {
                model: "gpt-4o-mini-transcribe".into(),
                api_key: None,
                transport: AsrTransportKind::RestBatch,
            },
            prompt: Some("输出简体中文".into()),
        };
        let toml_str = toml::to_string_pretty(&profile).unwrap();
        assert!(toml_str.contains("type = \"OpenAi\""));
        assert!(toml_str.contains("gpt-4o-mini-transcribe"));
        let back: LanguageProfile = toml::from_str(&toml_str).unwrap();
        assert_eq!(back.id, "zh");
        assert!(matches!(back.backend, ProfileBackendConfig::OpenAi { .. }));
    }

    #[test]
    fn legacy_cloud_profile_deserializes_as_openai() {
        let toml_str = r#"
id = "legacy"
display_name = "Legacy Cloud"
language_hint = "en"
prompt = "context"

[backend]
type = "Cloud"
model = "gpt-4o-transcribe"
"#;
        let back: LanguageProfile = toml::from_str(toml_str).unwrap();
        assert!(matches!(back.backend, ProfileBackendConfig::OpenAi { .. }));
    }

    #[test]
    fn local_profile_serde() {
        let profile = LanguageProfile {
            id: "en".into(),
            display_name: "English".into(),
            language_hint: Some("en".into()),
            backend: ProfileBackendConfig::Local {
                model_path: PathBuf::from("/models/ggml-base.en.bin"),
                transport: AsrTransportKind::LocalBatch,
            },
            prompt: None,
        };
        let toml_str = toml::to_string_pretty(&profile).unwrap();
        assert!(toml_str.contains("type = \"Local\""));
        let back: LanguageProfile = toml::from_str(&toml_str).unwrap();
        assert!(matches!(back.backend, ProfileBackendConfig::Local { .. }));
    }

    #[test]
    fn validate_warns_en_model_for_auto() {
        let mut config = AppConfig::default();
        config.profiles.insert("en".into(), LanguageProfile {
            id: "en".into(),
            display_name: "English".into(),
            language_hint: None, // auto — should warn
            backend: ProfileBackendConfig::Local {
                model_path: PathBuf::from("/models/ggml-base.en.bin"),
                transport: AsrTransportKind::LocalBatch,
            },
            prompt: None,
        });
        let warnings = config.validate();
        assert!(!warnings.is_empty());
        assert!(matches!(warnings[0], ValidationWarning::EnModelForAutoDetect { .. }));
    }
}

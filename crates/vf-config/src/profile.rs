use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrTransportKind {
    RestBatch,
    WebSocketStreaming,
    LocalBatch,
    LocalStreaming,
}

impl Default for AsrTransportKind {
    fn default() -> Self { Self::RestBatch }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProfileBackendConfig {
    #[serde(alias = "Cloud")]
    OpenAi {
        model: String,
        api_key: Option<String>,
        #[serde(default)]
        transport: AsrTransportKind,
    },
    VoxNexus {
        #[serde(default = "default_voxnexus_model_id")]
        model_id: String,
        api_key: Option<String>,
        #[serde(default)]
        transport: AsrTransportKind,
        #[serde(default)]
        enable_timestamps: bool,
        #[serde(default)]
        enable_speaker_diarization: bool,
        #[serde(default)]
        enable_llm_transform: bool,
        #[serde(default)]
        llm_model_id: Option<String>,
        #[serde(default)]
        llm_max_tokens: Option<u32>,
    },
    Local {
        model_path: PathBuf,
        #[serde(default = "default_local_transport")]
        transport: AsrTransportKind,
    },
}

fn default_local_transport() -> AsrTransportKind { AsrTransportKind::LocalBatch }
fn default_voxnexus_model_id() -> String { "vn-stt-ultra".to_string() }

impl ProfileBackendConfig {
    pub fn provider_id(&self) -> &'static str {
        match self {
            Self::OpenAi { .. } => "openai",
            Self::VoxNexus { .. } => "voxnexus",
            Self::Local { .. } => "local_whisper",
        }
    }

    pub fn transport(&self) -> AsrTransportKind {
        match self {
            Self::OpenAi { transport, .. } => *transport,
            Self::VoxNexus { transport, .. } => *transport,
            Self::Local { transport, .. } => *transport,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageProfile {
    pub id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_hint: Option<String>,
    pub backend: ProfileBackendConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ValidationWarning {
    EnModelForNonEnglish { profile_id: String, model_path: PathBuf },
    EnModelForAutoDetect { profile_id: String, model_path: PathBuf },
}

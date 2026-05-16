use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::error::AsrError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrTransportKind {
    RestBatch,
    WebSocketStreaming,
    LocalBatch,
    LocalStreaming,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrCapabilities {
    pub batch: bool,
    pub streaming: bool,
    pub language_hint: bool,
    pub auto_language_detection: bool,
    pub prompt: bool,
    pub timestamps: bool,
    pub speaker_diarization: bool,
    pub model_selection: bool,
    pub llm_transform: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrProviderDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub transports: Vec<AsrTransportKind>,
    pub capabilities: AsrCapabilities,
}

#[derive(Debug, Clone)]
pub struct TranscribeRequest {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub language_hint: Option<String>,
    pub prompt: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StreamTranscribeRequest {
    pub sample_rate: u32,
    pub language_hint: Option<String>,
    pub prompt: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscribeResult {
    pub text: String,
    pub language_detected: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    Ready { request_id: Option<String> },
    Partial { text: String },
    Final { result: TranscribeResult },
    FlushDone { result: TranscribeResult },
    Error { message: String },
}

#[async_trait]
pub trait StreamingTranscriber: Send {
    async fn send_audio(&mut self, chunk: AudioChunk) -> Result<(), AsrError>;
    async fn next_event(&mut self) -> Result<StreamEvent, AsrError>;
    async fn finish(&mut self) -> Result<TranscribeResult, AsrError>;
    async fn cancel(&mut self) -> Result<(), AsrError>;
}

#[async_trait]
pub trait AsrBackend: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    async fn is_ready(&self) -> bool;
    async fn prepare(&mut self) -> Result<(), AsrError>;
    async fn transcribe(&self, req: TranscribeRequest) -> Result<TranscribeResult, AsrError>;
    async fn start_stream(
        &mut self,
        _req: StreamTranscribeRequest,
    ) -> Result<Box<dyn StreamingTranscriber>, AsrError> {
        Err(AsrError::UnsupportedTransport(format!(
            "{} does not implement streaming yet",
            self.name()
        )))
    }
}

pub fn provider_descriptors() -> Vec<AsrProviderDescriptor> {
    vec![
        AsrProviderDescriptor {
            id: "openai",
            display_name: "OpenAI",
            transports: vec![AsrTransportKind::RestBatch],
            capabilities: AsrCapabilities {
                batch: true,
                streaming: false,
                language_hint: true,
                auto_language_detection: true,
                prompt: true,
                timestamps: false,
                speaker_diarization: false,
                model_selection: true,
                llm_transform: false,
            },
        },
        AsrProviderDescriptor {
            id: "voxnexus",
            display_name: "VoxNexus",
            transports: vec![AsrTransportKind::RestBatch, AsrTransportKind::WebSocketStreaming],
            capabilities: AsrCapabilities {
                batch: true,
                streaming: true,
                language_hint: true,
                auto_language_detection: true,
                prompt: true,
                timestamps: true,
                speaker_diarization: true,
                model_selection: true,
                llm_transform: true,
            },
        },
        AsrProviderDescriptor {
            id: "local_whisper",
            display_name: "Local Whisper.cpp",
            transports: vec![AsrTransportKind::LocalBatch],
            capabilities: AsrCapabilities {
                batch: true,
                streaming: false,
                language_hint: true,
                auto_language_detection: true,
                prompt: true,
                timestamps: false,
                speaker_diarization: false,
                model_selection: true,
                llm_transform: false,
            },
        },
    ]
}

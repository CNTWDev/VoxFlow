use std::time::Instant;
use async_trait::async_trait;
use crate::backend::{AsrBackend, TranscribeRequest, TranscribeResult};
use crate::error::AsrError;
use crate::wav::encode_wav_f32_mono;

pub struct VoxNexusBackend {
    api_key: String,
    model_id: String,
    client: reqwest::Client,
    ready: bool,
    enable_timestamps: bool,
    enable_speaker_diarization: bool,
}

impl VoxNexusBackend {
    pub fn new(
        api_key: impl Into<String>,
        model_id: impl Into<String>,
        enable_timestamps: bool,
        enable_speaker_diarization: bool,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            model_id: model_id.into(),
            client: reqwest::Client::new(),
            ready: false,
            enable_timestamps,
            enable_speaker_diarization,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct VoxNexusResponse {
    language: Option<String>,
    text: String,
    duration_ms: Option<u64>,
}

#[async_trait]
impl AsrBackend for VoxNexusBackend {
    fn name(&self) -> &'static str { "VoxNexus STT REST API" }

    async fn is_ready(&self) -> bool { self.ready && !self.api_key.is_empty() }

    async fn prepare(&mut self) -> Result<(), AsrError> {
        if self.api_key.is_empty() {
            return Err(AsrError::Api("VoxNexus API key not configured".into()));
        }
        self.ready = true;
        tracing::info!("VoxNexusBackend ready");
        Ok(())
    }

    async fn transcribe(&self, req: TranscribeRequest) -> Result<TranscribeResult, AsrError> {
        let t0 = Instant::now();
        let wav_bytes = encode_wav_f32_mono(&req.samples, req.sample_rate)?;

        let mut query = vec![
            ("model_id", self.model_id.clone()),
            ("sample_rate", req.sample_rate.to_string()),
            ("enable_timestamps", self.enable_timestamps.to_string()),
            ("enable_speaker_diarization", self.enable_speaker_diarization.to_string()),
        ];
        if let Some(language) = req.language_hint.as_deref().filter(|v| !v.is_empty()) {
            query.push(("language", language.to_string()));
        }

        tracing::info!(
            "VoxNexus STT request: model_id={} sample_rate={} wav_bytes={} timestamps={} diarization={} language={}",
            self.model_id,
            req.sample_rate,
            wav_bytes.len(),
            self.enable_timestamps,
            self.enable_speaker_diarization,
            req.language_hint.as_deref().unwrap_or("auto")
        );

        let resp = self.client
            .post("https://api.voxnexus.ai/v1/stt")
            .header("X-Api-Key", &self.api_key)
            .header(reqwest::header::CONTENT_TYPE, "audio/wav")
            .query(&query)
            .body(wav_bytes)
            .send()
            .await
            .map_err(|e| AsrError::Api(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AsrError::Api(format!("VoxNexus HTTP {status}: {body}")));
        }

        let api_resp: VoxNexusResponse = resp.json().await
            .map_err(|e| AsrError::Api(e.to_string()))?;

        Ok(TranscribeResult {
            text: api_resp.text,
            language_detected: api_resp.language,
            duration_ms: api_resp.duration_ms.unwrap_or_else(|| t0.elapsed().as_millis() as u64),
        })
    }
}

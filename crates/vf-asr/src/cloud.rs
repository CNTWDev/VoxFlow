use std::time::Instant;
use async_trait::async_trait;
use crate::backend::{AsrBackend, TranscribeRequest, TranscribeResult};
use crate::error::AsrError;
use crate::wav::encode_wav_f32_mono;

pub struct OpenAiBackend {
    api_key: String,
    model: String,
    client: reqwest::Client,
    ready: bool,
}

pub type CloudBackend = OpenAiBackend;

impl OpenAiBackend {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            client: reqwest::Client::new(),
            ready: false,
        }
    }
}

#[async_trait]
impl AsrBackend for OpenAiBackend {
    fn name(&self) -> &'static str { "OpenAI Transcription API" }

    async fn is_ready(&self) -> bool { self.ready && !self.api_key.is_empty() }

    async fn prepare(&mut self) -> Result<(), AsrError> {
        if self.api_key.is_empty() {
            return Err(AsrError::Api("API key not configured".into()));
        }
        self.ready = true;
        tracing::info!("OpenAiBackend ready (model: {})", self.model);
        Ok(())
    }

    async fn transcribe(&self, req: TranscribeRequest) -> Result<TranscribeResult, AsrError> {
        let t0 = Instant::now();
        let wav_bytes = encode_wav_f32_mono(&req.samples, req.sample_rate)?;

        let file_part = reqwest::multipart::Part::bytes(wav_bytes)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| AsrError::Api(e.to_string()))?;

        let mut form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone());

        if let Some(lang) = &req.language_hint {
            form = form.text("language", lang.clone());
        }
        if let Some(prompt) = &req.prompt {
            form = form.text("prompt", prompt.clone());
        }

        let resp = self.client
            .post("https://api.openai.com/v1/audio/transcriptions")
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| AsrError::Api(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AsrError::Api(format!("HTTP {status}: {body}")));
        }

        #[derive(serde::Deserialize)]
        struct ApiResponse { text: String }
        let api_resp: ApiResponse = resp.json().await
            .map_err(|e| AsrError::Api(e.to_string()))?;

        Ok(TranscribeResult {
            text: api_resp.text,
            language_detected: req.language_hint,
            duration_ms: t0.elapsed().as_millis() as u64,
        })
    }
}

use std::time::Instant;
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use futures_util::{stream::{SplitSink, SplitStream}, SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::Message,
    MaybeTlsStream,
    WebSocketStream,
};
use crate::backend::{
    AsrBackend, AsrTransportKind, AudioChunk, StreamEvent, StreamTranscribeRequest,
    StreamingTranscriber, TranscribeRequest, TranscribeResult,
};
use crate::error::AsrError;
use crate::wav::encode_wav_f32_mono;

pub struct VoxNexusBackend {
    api_key: String,
    model_id: String,
    client: reqwest::Client,
    ready: bool,
    transport: AsrTransportKind,
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
            transport: AsrTransportKind::RestBatch,
            enable_timestamps,
            enable_speaker_diarization,
        }
    }

    pub fn with_transport(mut self, transport: AsrTransportKind) -> Self {
        self.transport = transport;
        self
    }

    async fn transcribe_rest(&self, req: TranscribeRequest) -> Result<TranscribeResult, AsrError> {
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
            "VoxNexus STT REST request: model_id={} sample_rate={} wav_bytes={} timestamps={} diarization={} language={}",
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

    async fn transcribe_websocket(&self, req: TranscribeRequest) -> Result<TranscribeResult, AsrError> {
        let t0 = Instant::now();
        let mut stream = self.start_voxnexus_stream(StreamTranscribeRequest {
            sample_rate: req.sample_rate,
            language_hint: req.language_hint,
            prompt: req.prompt,
        }).await?;

        for chunk in req.samples.chunks(4096) {
            stream.send_audio(AudioChunk {
                samples: chunk.to_vec(),
                sample_rate: req.sample_rate,
            }).await?;
        }

        let mut result = stream.finish().await?;
        if result.duration_ms == 0 {
            result.duration_ms = t0.elapsed().as_millis() as u64;
        }
        Ok(result)
    }

    async fn start_voxnexus_stream(
        &self,
        req: StreamTranscribeRequest,
    ) -> Result<Box<dyn StreamingTranscriber>, AsrError> {
        if self.api_key.is_empty() {
            return Err(AsrError::Api("VoxNexus API key not configured".into()));
        }

        let url = format!(
            "wss://api.voxnexus.ai/v1/stt/realtime?token={}",
            self.api_key
        );
        let (ws, _) = connect_async(url)
            .await
            .map_err(|e| AsrError::Api(format!("VoxNexus WebSocket connect failed: {e}")))?;
        let (mut write, read) = ws.split();

        let init = json!({
            "type": "init",
            "language": req.language_hint.clone().filter(|value| !value.is_empty()),
            "format": "pcm",
            "sample_rate": req.sample_rate,
            "model_id": self.model_id,
            "enable_timestamps": self.enable_timestamps,
            "enable_speaker_diarization": self.enable_speaker_diarization,
            "enable_language_detection": req.language_hint.as_deref().map(str::is_empty).unwrap_or(true),
        });
        write.send(Message::Text(init.to_string().into()))
            .await
            .map_err(|e| AsrError::Api(format!("VoxNexus WebSocket init failed: {e}")))?;

        tracing::info!(
            "VoxNexus STT WebSocket stream started: model_id={} sample_rate={} timestamps={} diarization={} language={}",
            self.model_id,
            req.sample_rate,
            self.enable_timestamps,
            self.enable_speaker_diarization,
            req.language_hint.as_deref().unwrap_or("auto")
        );

        Ok(Box::new(VoxNexusStreamingTranscriber {
            write,
            read,
            sample_rate: req.sample_rate,
            final_result: None,
        }))
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
    fn name(&self) -> &'static str { "VoxNexus STT API" }

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
        match self.transport {
            AsrTransportKind::RestBatch => self.transcribe_rest(req).await,
            AsrTransportKind::WebSocketStreaming => self.transcribe_websocket(req).await,
            other => Err(AsrError::UnsupportedTransport(format!(
                "VoxNexus does not support {other:?}"
            ))),
        }
    }

    async fn start_stream(
        &mut self,
        req: StreamTranscribeRequest,
    ) -> Result<Box<dyn StreamingTranscriber>, AsrError> {
        self.start_voxnexus_stream(req).await
    }
}

type VoxNexusWs = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct VoxNexusStreamingTranscriber {
    write: SplitSink<VoxNexusWs, Message>,
    read: SplitStream<VoxNexusWs>,
    sample_rate: u32,
    final_result: Option<TranscribeResult>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type")]
enum VoxNexusWsMessage {
    #[serde(rename = "ready")]
    Ready { request_id: Option<String> },
    #[serde(rename = "partial")]
    Partial { text: String },
    #[serde(rename = "final")]
    Final {
        text: String,
        language: Option<String>,
        end_time_ms: Option<u64>,
    },
    #[serde(rename = "error")]
    Error {
        error: Option<String>,
        message: Option<String>,
    },
}

#[async_trait]
impl StreamingTranscriber for VoxNexusStreamingTranscriber {
    async fn send_audio(&mut self, chunk: AudioChunk) -> Result<(), AsrError> {
        if chunk.sample_rate != self.sample_rate {
            return Err(AsrError::Encoding(format!(
                "stream sample rate changed from {} to {}",
                self.sample_rate, chunk.sample_rate
            )));
        }
        let pcm = pcm16le_from_f32(&chunk.samples);
        let message = json!({
            "type": "audio",
            "data": general_purpose::STANDARD.encode(pcm),
        });
        self.write.send(Message::Text(message.to_string().into()))
            .await
            .map_err(|e| AsrError::Api(format!("VoxNexus WebSocket audio send failed: {e}")))
    }

    async fn next_event(&mut self) -> Result<StreamEvent, AsrError> {
        loop {
            let message = self.read.next().await
                .ok_or_else(|| AsrError::Api("VoxNexus WebSocket closed".into()))?
                .map_err(|e| AsrError::Api(format!("VoxNexus WebSocket read failed: {e}")))?;

            let Message::Text(text) = message else {
                continue;
            };
            match serde_json::from_str::<VoxNexusWsMessage>(&text)
                .map_err(|e| AsrError::Api(format!("VoxNexus WebSocket message parse failed: {e}; body={text}")))? {
                VoxNexusWsMessage::Ready { request_id } => return Ok(StreamEvent::Ready { request_id }),
                VoxNexusWsMessage::Partial { text } => return Ok(StreamEvent::Partial { text }),
                VoxNexusWsMessage::Final { text, language, end_time_ms } => {
                    let result = TranscribeResult {
                        text,
                        language_detected: language,
                        duration_ms: end_time_ms.unwrap_or_default(),
                    };
                    self.final_result = Some(result.clone());
                    return Ok(StreamEvent::Final { result });
                }
                VoxNexusWsMessage::Error { error, message } => {
                    let message = error.or(message).unwrap_or_else(|| "unknown VoxNexus WebSocket error".into());
                    return Ok(StreamEvent::Error { message });
                }
            }
        }
    }

    async fn finish(&mut self) -> Result<TranscribeResult, AsrError> {
        let end = json!({ "type": "end" });
        self.write.send(Message::Text(end.to_string().into()))
            .await
            .map_err(|e| AsrError::Api(format!("VoxNexus WebSocket finish failed: {e}")))?;

        let deadline = tokio::time::Duration::from_secs(30);
        let result = tokio::time::timeout(deadline, async {
            loop {
                match self.next_event().await? {
                    StreamEvent::Final { result } => return Ok(result),
                    StreamEvent::Error { message } => return Err(AsrError::Api(message)),
                    StreamEvent::Ready { .. } | StreamEvent::Partial { .. } => {}
                }
            }
        }).await
            .map_err(|_| AsrError::Timeout)??;

        Ok(result)
    }

    async fn cancel(&mut self) -> Result<(), AsrError> {
        self.write.close().await
            .map_err(|e| AsrError::Api(format!("VoxNexus WebSocket close failed: {e}")))
    }
}

fn pcm16le_from_f32(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

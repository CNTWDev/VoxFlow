use std::{collections::HashMap, time::Instant};
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
    llm_config: VoxNexusLlmConfig,
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
            llm_config: VoxNexusLlmConfig::default(),
        }
    }

    pub fn with_transport(mut self, transport: AsrTransportKind) -> Self {
        self.transport = transport;
        self
    }

    pub fn with_llm_transform(
        mut self,
        enabled: bool,
        model_id: Option<String>,
        max_tokens: Option<u32>,
    ) -> Self {
        self.llm_config = VoxNexusLlmConfig {
            enabled,
            model_id,
            max_tokens,
        };
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
        self.push_llm_query_params(&mut query, req.prompt.as_deref())?;

        tracing::info!(
            "VoxNexus STT REST request: model_id={} sample_rate={} wav_bytes={} timestamps={} diarization={} llm_transform={} language={}",
            self.model_id,
            req.sample_rate,
            wav_bytes.len(),
            self.enable_timestamps,
            self.enable_speaker_diarization,
            self.llm_config.enabled,
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

        let mut init = json!({
            "type": "init",
            "language": req.language_hint.clone().filter(|value| !value.is_empty()),
            "format": "pcm",
            "sample_rate": req.sample_rate,
            "model_id": self.model_id,
            "enable_timestamps": self.enable_timestamps,
            "enable_llm_transform": self.llm_config.enabled,
        });
        self.apply_llm_init_params(&mut init, req.prompt.as_deref())?;
        write.send(Message::Text(init.to_string().into()))
            .await
            .map_err(|e| AsrError::Api(format!("VoxNexus WebSocket init failed: {e}")))?;

        tracing::info!(
            "VoxNexus STT WebSocket stream started: model_id={} sample_rate={} timestamps={} llm_transform={} language={}",
            self.model_id,
            req.sample_rate,
            self.enable_timestamps,
            self.llm_config.enabled,
            req.language_hint.as_deref().unwrap_or("auto")
        );

        Ok(Box::new(VoxNexusStreamingTranscriber {
            write,
            read,
            sample_rate: req.sample_rate,
            final_result: None,
            language_detected: None,
            duration_ms: 0,
            transcript_parts: Vec::new(),
            llm_full_text: String::new(),
            llm_segment_order: Vec::new(),
            llm_segments: HashMap::new(),
        }))
    }

    fn push_llm_query_params(
        &self,
        query: &mut Vec<(&'static str, String)>,
        prompt: Option<&str>,
    ) -> Result<(), AsrError> {
        query.push(("enable_llm_transform", self.llm_config.enabled.to_string()));
        if !self.llm_config.enabled {
            return Ok(());
        }

        let prompt = normalized_llm_prompt(prompt)?;
        query.push(("llm_prompt", prompt.to_string()));
        if let Some(model_id) = self.llm_config.model_id.as_deref().filter(|value| !value.is_empty()) {
            query.push(("llm_model_id", model_id.to_string()));
        }
        if let Some(max_tokens) = self.llm_config.max_tokens {
            query.push(("llm_max_tokens", max_tokens.to_string()));
        }
        Ok(())
    }

    fn apply_llm_init_params(
        &self,
        init: &mut serde_json::Value,
        prompt: Option<&str>,
    ) -> Result<(), AsrError> {
        if !self.llm_config.enabled {
            return Ok(());
        }

        let prompt = normalized_llm_prompt(prompt)?;
        init["llm_prompt"] = json!(prompt);
        init["llm_mode"] = json!("post_flush");
        if let Some(model_id) = self.llm_config.model_id.as_deref().filter(|value| !value.is_empty()) {
            init["llm_model_id"] = json!(model_id);
        }
        if let Some(max_tokens) = self.llm_config.max_tokens {
            init["llm_max_tokens"] = json!(max_tokens);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
struct VoxNexusLlmConfig {
    enabled: bool,
    model_id: Option<String>,
    max_tokens: Option<u32>,
}

fn normalized_llm_prompt(prompt: Option<&str>) -> Result<&str, AsrError> {
    prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AsrError::Api(
            "VoxNexus LLM polish is enabled but no prompt is configured".into()
        ))
}

#[derive(Debug, serde::Deserialize)]
struct VoxNexusResponse {
    language: Option<String>,
    #[allow(dead_code)]
    transcript: Option<String>,
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
    language_detected: Option<String>,
    duration_ms: u64,
    transcript_parts: Vec<String>,
    llm_full_text: String,
    llm_segment_order: Vec<String>,
    llm_segments: HashMap<String, String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type")]
enum VoxNexusWsMessage {
    #[serde(rename = "ready")]
    Ready { request_id: Option<String> },
    #[serde(rename = "transcript")]
    Transcript {
        is_final: bool,
        text: String,
        segment_id: Option<String>,
        language: Option<String>,
        duration: Option<u64>,
        offset: Option<u64>,
    },
    #[serde(rename = "llm")]
    Llm {
        segment_id: Option<String>,
        text: Option<String>,
        delta: Option<String>,
        is_final: bool,
    },
    #[serde(rename = "flush_done")]
    FlushDone {
        request_id: Option<String>,
    },
    #[serde(rename = "error")]
    Error {
        error: Option<String>,
        code: Option<String>,
        message: Option<String>,
    },
}

impl VoxNexusStreamingTranscriber {
    fn final_result(&self) -> TranscribeResult {
        TranscribeResult {
            text: final_text(
                &self.transcript_parts,
                &self.llm_full_text,
                &self.llm_segment_order,
                &self.llm_segments,
                self.final_result.as_ref(),
            ),
            language_detected: self.language_detected.clone(),
            duration_ms: self.duration_ms,
        }
    }
}

fn final_text(
    transcript_parts: &[String],
    llm_full_text: &str,
    llm_segment_order: &[String],
    llm_segments: &HashMap<String, String>,
    final_result: Option<&TranscribeResult>,
) -> String {
    if !llm_full_text.is_empty() {
        return llm_full_text.to_string();
    }

    let llm_text = llm_segment_order
        .iter()
        .filter_map(|segment_id| llm_segments.get(segment_id))
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if !llm_text.is_empty() {
        return llm_text;
    }

    if !transcript_parts.is_empty() {
        return transcript_parts.join(" ");
    }

    final_result
        .map(|result| result.text.clone())
        .unwrap_or_default()
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
                VoxNexusWsMessage::Transcript {
                    text,
                    is_final,
                    segment_id,
                    language,
                    duration,
                    offset,
                } => {
                    if !is_final {
                        return Ok(StreamEvent::Partial { text });
                    }
                    if let Some(segment_id) = segment_id {
                        if !self.llm_segments.contains_key(&segment_id) {
                            self.llm_segment_order.push(segment_id.clone());
                        }
                        self.llm_segments.entry(segment_id).or_default();
                    }
                    self.transcript_parts.push(text.clone());
                    if language.is_some() {
                        self.language_detected = language.clone();
                    }
                    self.duration_ms = self.duration_ms.max(
                        offset.unwrap_or_default() + duration.unwrap_or_default(),
                    );
                    let result = TranscribeResult {
                        text,
                        language_detected: self.language_detected.clone(),
                        duration_ms: self.duration_ms,
                    };
                    self.final_result = Some(result.clone());
                    return Ok(StreamEvent::Final { result });
                }
                VoxNexusWsMessage::Llm {
                    segment_id,
                    text,
                    delta,
                    is_final,
                } => {
                    let text_payload = text.filter(|value| !value.is_empty());
                    let is_text_payload = text_payload.is_some();
                    let llm_text = text_payload
                        .or(delta.filter(|value| !value.is_empty()))
                        .unwrap_or_default();
                    if let Some(segment_id) = segment_id {
                        if !self.llm_segments.contains_key(&segment_id) {
                            self.llm_segment_order.push(segment_id.clone());
                        }
                        let entry = self.llm_segments.entry(segment_id).or_default();
                        if is_text_payload {
                            *entry = llm_text;
                        } else {
                            entry.push_str(&llm_text);
                        }
                        if is_final {
                            let text = entry.clone();
                            let result = TranscribeResult {
                                text,
                                language_detected: self.language_detected.clone(),
                                duration_ms: self.duration_ms,
                            };
                            self.final_result = Some(result.clone());
                            return Ok(StreamEvent::Final { result });
                        }
                    } else {
                        if is_text_payload {
                            self.llm_full_text = llm_text;
                        } else {
                            self.llm_full_text.push_str(&llm_text);
                        }
                        if is_final {
                            let result = TranscribeResult {
                                text: self.llm_full_text.clone(),
                                language_detected: self.language_detected.clone(),
                                duration_ms: self.duration_ms,
                            };
                            self.final_result = Some(result.clone());
                            return Ok(StreamEvent::Final { result });
                        }
                    }
                }
                VoxNexusWsMessage::FlushDone { request_id } => {
                    drop(request_id);
                    let result = self.final_result();
                    return Ok(StreamEvent::FlushDone { result });
                }
                VoxNexusWsMessage::Error { error, code, message } => {
                    let message = error
                        .or(message)
                        .or(code.map(|code| format!("VoxNexus WebSocket error: {code}")))
                        .unwrap_or_else(|| "unknown VoxNexus WebSocket error".into());
                    return Ok(StreamEvent::Error { message });
                }
            }
        }
    }

    async fn finish(&mut self) -> Result<TranscribeResult, AsrError> {
        let flush = json!({ "type": "command", "command": "flush" });
        self.write.send(Message::Text(flush.to_string().into()))
            .await
            .map_err(|e| AsrError::Api(format!("VoxNexus WebSocket finish failed: {e}")))?;

        let deadline = tokio::time::Duration::from_secs(30);
        let result = tokio::time::timeout(deadline, async {
            loop {
                match self.next_event().await? {
                    StreamEvent::FlushDone { result } => return Ok(result),
                    StreamEvent::Error { message } => return Err(AsrError::Api(message)),
                    StreamEvent::Ready { .. } | StreamEvent::Partial { .. } | StreamEvent::Final { .. } => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_llm_text_field() {
        let message: VoxNexusWsMessage = serde_json::from_str(
            r#"{"type":"llm","request_id":"req","text":"polished","is_final":true}"#,
        ).unwrap();

        match message {
            VoxNexusWsMessage::Llm { text, delta, is_final, .. } => {
                assert_eq!(text.as_deref(), Some("polished"));
                assert!(delta.is_none());
                assert!(is_final);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn final_result_prefers_full_llm_text() {
        assert_eq!(
            final_text(
                &["raw one".into(), "raw two".into()],
                "rewritten full",
                &[],
                &HashMap::new(),
                None,
            ),
            "rewritten full"
        );
    }

    #[test]
    fn final_result_joins_llm_segments_before_transcript() {
        let order = vec!["a".into(), "b".into()];
        let segments = HashMap::from([
            ("a".into(), "polished one".into()),
            ("b".into(), "polished two".into()),
        ]);

        assert_eq!(
            final_text(
                &["raw one".into(), "raw two".into()],
                "",
                &order,
                &segments,
                None,
            ),
            "polished one polished two"
        );
    }
}

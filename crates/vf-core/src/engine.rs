use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::LocalSet;
use tokio::time::{timeout, Duration, Instant};
use serde::{Deserialize, Serialize};
use vf_config::{AppConfig, AsrTransportKind, ProfileBackendConfig};
use vf_audio::{AudioCapture, AudioResampler, TARGET_SAMPLE_RATE};
use vf_asr::{AsrBackend, OpenAiBackend, TranscribeRequest, VoxNexusBackend};
use vf_inject::TextInjector;
use crate::state::RecorderState;
use crate::error::VoxError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EngineEvent {
    StateChanged {
        state: RecorderState,
    },
    AudioLevel {
        rms: f32,
        peak: f32,
    },
    Transcription {
        text: String,
        duration_ms: u64,
        profile_id: String,
    },
    Error {
        code: String,
        message: String,
    },
}

const POST_RECORDING_TIMEOUT: Duration = Duration::from_secs(5);

fn set_state(
    state_tx: &watch::Sender<RecorderState>,
    event_tx: &broadcast::Sender<EngineEvent>,
    state: RecorderState,
) {
    tracing::info!("state changed: {:?}", state);
    state_tx.send_replace(state.clone());
    let _ = event_tx.send(EngineEvent::StateChanged { state });
}

#[derive(Debug)]
enum EngineCommand {
    StartRecording,
    StopRecording,
    Cancel,
    GetConfig(oneshot::Sender<AppConfig>),
    SetConfig(AppConfig, oneshot::Sender<Result<(), VoxError>>),
    SetActiveProfile(String, oneshot::Sender<Result<(), VoxError>>),
    Shutdown,
}

/// Handle to the VoxEngine background task — cheap to clone.
#[derive(Clone)]
pub struct VoxEngine {
    cmd_tx: mpsc::Sender<EngineCommand>,
    state_tx: Arc<watch::Sender<RecorderState>>,
    event_tx: broadcast::Sender<EngineEvent>,
}

impl VoxEngine {
    pub fn new(config: AppConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (state_tx, _) = watch::channel(RecorderState::Idle);
        let state_tx = Arc::new(state_tx);
        let (event_tx, _) = broadcast::channel(128);

        let state_clone = Arc::clone(&state_tx);
        let event_clone = event_tx.clone();

        // Dedicated OS thread: current_thread runtime + LocalSet so AudioCapture (!Send) works.
        std::thread::Builder::new()
            .name("vox-engine".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("engine runtime");
                let local = LocalSet::new();
                local.block_on(&rt, engine_loop(config, cmd_rx, state_clone, event_clone, None));
            })
            .expect("engine thread spawn");

        Self { cmd_tx, state_tx, event_tx }
    }

    pub fn state(&self) -> RecorderState {
        self.state_tx.borrow().clone()
    }

    pub fn subscribe_state(&self) -> watch::Receiver<RecorderState> {
        self.state_tx.subscribe()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<EngineEvent> {
        self.event_tx.subscribe()
    }

    pub async fn start_recording(&self) -> Result<(), VoxError> {
        self.cmd_tx.send(EngineCommand::StartRecording).await
            .map_err(|_| VoxError::Other(anyhow::anyhow!("engine stopped")))?;
        Ok(())
    }

    pub async fn stop_recording(&self) -> Result<(), VoxError> {
        self.cmd_tx.send(EngineCommand::StopRecording).await
            .map_err(|_| VoxError::Other(anyhow::anyhow!("engine stopped")))?;
        Ok(())
    }

    pub async fn cancel(&self) -> Result<(), VoxError> {
        self.cmd_tx.send(EngineCommand::Cancel).await
            .map_err(|_| VoxError::Other(anyhow::anyhow!("engine stopped")))?;
        Ok(())
    }

    pub async fn get_config(&self) -> AppConfig {
        let (tx, rx) = oneshot::channel();
        let _ = self.cmd_tx.send(EngineCommand::GetConfig(tx)).await;
        rx.await.unwrap_or_default()
    }

    pub async fn set_config(&self, config: AppConfig) -> Result<(), VoxError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(EngineCommand::SetConfig(config, tx)).await
            .map_err(|_| VoxError::Other(anyhow::anyhow!("engine stopped")))?;
        rx.await.unwrap_or(Err(VoxError::Other(anyhow::anyhow!("engine stopped"))))
    }

    pub async fn set_active_profile(&self, profile_id: String) -> Result<(), VoxError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(EngineCommand::SetActiveProfile(profile_id, tx)).await
            .map_err(|_| VoxError::Other(anyhow::anyhow!("engine stopped")))?;
        rx.await.unwrap_or(Err(VoxError::Other(anyhow::anyhow!("engine stopped"))))
    }

    #[cfg(test)]
    pub fn new_for_test(config: AppConfig, init_backend: Option<(String, Box<dyn AsrBackend>)>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (state_tx, _) = watch::channel(RecorderState::Idle);
        let state_tx = Arc::new(state_tx);
        let (event_tx, _) = broadcast::channel(128);

        let state_clone = Arc::clone(&state_tx);
        let event_clone = event_tx.clone();

        std::thread::Builder::new()
            .name("vox-engine-test".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("engine runtime");
                let local = LocalSet::new();
                local.block_on(&rt, engine_loop(config, cmd_rx, state_clone, event_clone, init_backend));
            })
            .expect("engine thread spawn");

        Self { cmd_tx, state_tx, event_tx }
    }
}

impl Drop for VoxEngine {
    fn drop(&mut self) {
        // Best-effort: tell the engine loop to abort any in-progress recording and exit.
        // If the channel is already closed (all senders gone), this silently fails.
        let _ = self.cmd_tx.try_send(EngineCommand::Shutdown);
    }
}

async fn engine_loop(
    mut config: AppConfig,
    mut cmd_rx: mpsc::Receiver<EngineCommand>,
    state_tx: Arc<watch::Sender<RecorderState>>,
    event_tx: broadcast::Sender<EngineEvent>,
    init_backend: Option<(String, Box<dyn AsrBackend>)>,
) {
    let mut stop_tx: Option<oneshot::Sender<()>> = None;
    let mut record_handle: Option<tokio::task::JoinHandle<Vec<f32>>> = None;
    let mut cached_backend: Option<(String, Box<dyn AsrBackend>)> = init_backend;

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            EngineCommand::StartRecording => {
                if !matches!(*state_tx.borrow(), RecorderState::Idle) {
                    tracing::warn!("StartRecording ignored — not Idle");
                    continue;
                }

                let capture = match AudioCapture::start() {
                    Ok(c) => c,
                    Err(e) => {
                        let msg = e.to_string();
                        tracing::error!("audio capture failed: {msg}");
                        let _ = event_tx.send(EngineEvent::Error { code: "audio".into(), message: msg });
                        continue; // state remains Idle — recording can be retried
                    }
                };

                let (tx, rx) = oneshot::channel::<()>();
                stop_tx = Some(tx);
                let max_secs = config.audio.max_recording_secs;
                let handle = tokio::task::spawn_local(accumulate(
                    capture,
                    rx,
                    max_secs,
                    event_tx.clone(),
                ));
                record_handle = Some(handle);

                set_state(&state_tx, &event_tx, RecorderState::Recording);
                tracing::info!("recording started");
            }

            EngineCommand::StopRecording => {
                tracing::info!("stop recording requested");
                if !matches!(*state_tx.borrow(), RecorderState::Recording) {
                    tracing::warn!("StopRecording ignored — not Recording");
                    continue;
                }

                if let Some(tx) = stop_tx.take() {
                    let _ = tx.send(());
                }

                set_state(&state_tx, &event_tx, RecorderState::Processing);

                let samples = match record_handle.take() {
                    Some(h) => match timeout(POST_RECORDING_TIMEOUT, h).await {
                        Ok(joined) => joined.unwrap_or_default(),
                        Err(_) => {
                            tracing::error!("stopping audio capture timed out");
                            let _ = event_tx.send(EngineEvent::Error {
                                code: "recording_timeout".into(),
                                message: "停止录音超时，请检查麦克风设备后重试".into(),
                            });
                            set_state(&state_tx, &event_tx, RecorderState::Idle);
                            continue;
                        }
                    },
                    None => Vec::new(),
                };

                tracing::info!("recording stopped: {} samples ({:.1}s)",
                    samples.len(),
                    samples.len() as f32 / TARGET_SAMPLE_RATE as f32);

                if samples.is_empty() {
                    tracing::warn!("no audio captured — skipping ASR");
                    let _ = event_tx.send(EngineEvent::Error {
                        code: "no_audio".into(),
                        message: "没有录到麦克风声音，请检查麦克风权限或输入设备".into(),
                    });
                    set_state(&state_tx, &event_tx, RecorderState::Idle);
                    continue;
                }

                let audio_level = audio_level(&samples);
                tracing::info!(
                    "recording level: rms={:.6}, peak={:.6}",
                    audio_level.rms,
                    audio_level.peak
                );
                if audio_level.peak == 0.0 {
                    tracing::warn!("audio samples are all zero — skipping ASR");
                    let _ = event_tx.send(EngineEvent::Error {
                        code: "no_audio_signal".into(),
                        message: "没有录到麦克风声音，请检查麦克风权限或输入设备".into(),
                    });
                    set_state(&state_tx, &event_tx, RecorderState::Idle);
                    continue;
                }

                // Debug WAV — only in debug builds, written off the critical path
                #[cfg(debug_assertions)]
                {
                    let wav_samples = samples.clone();
                    let wav_path = std::env::temp_dir().join("vox_flow_debug.wav");
                    tokio::task::spawn_blocking(move || {
                        if let Err(e) = vf_audio::save_wav(&wav_samples, TARGET_SAMPLE_RATE, &wav_path) {
                            tracing::warn!("debug wav failed: {e}");
                        }
                    });
                }

                // Run ASR
                let profile_id = config.active_profile_id.clone();
                let profile = config.active_profile().cloned();

                match timeout(
                    POST_RECORDING_TIMEOUT,
                    run_asr(samples, &profile_id, profile.as_ref(), &config, &mut cached_backend),
                )
                .await
                {
                    Ok(Ok(result)) => {
                        tracing::info!("transcription: {:?} ({}ms)", result.text, result.duration_ms);

                        if result.text.trim().is_empty() {
                            tracing::warn!("transcription was empty — skipping injection");
                            let _ = event_tx.send(EngineEvent::Error {
                                code: "empty_transcription".into(),
                                message: "识别结果为空，请再说一遍或检查识别语言设置".into(),
                            });
                            set_state(&state_tx, &event_tx, RecorderState::Idle);
                            continue;
                        }

                        set_state(&state_tx, &event_tx, RecorderState::Injecting);

                        let inject_text = result.text.clone();
                        let paste_delay = config.inject.paste_delay_ms;
                        let post_paste_delay = config.inject.post_paste_delay_ms;
                        let restore = config.inject.restore_text_clipboard;

                        let inject_task = tokio::task::spawn_blocking(move || {
                            TextInjector::new(paste_delay, post_paste_delay, restore)
                                .inject_sync(inject_text)
                        });

                        let inject_result = timeout(POST_RECORDING_TIMEOUT, inject_task).await;

                        let injected = match inject_result {
                            Ok(Ok(Ok(()))) => {
                                tracing::info!("text injected");
                                true
                            }
                            Ok(Ok(Err(e))) => {
                                tracing::warn!("injection failed: {e}");
                                let _ = event_tx.send(EngineEvent::Error {
                                    code: "inject".into(),
                                    message: e.to_string(),
                                });
                                false
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("injection task panicked: {e}");
                                let _ = event_tx.send(EngineEvent::Error {
                                    code: "inject".into(),
                                    message: "injection panicked".into(),
                                });
                                false
                            }
                            Err(_) => {
                                tracing::error!("injection timed out");
                                let _ = event_tx.send(EngineEvent::Error {
                                    code: "inject_timeout".into(),
                                    message: "输入文字超时，请检查当前应用是否允许粘贴".into(),
                                });
                                false
                            }
                        };

                        if injected {
                            let _ = event_tx.send(EngineEvent::Transcription {
                                text: result.text,
                                duration_ms: result.duration_ms,
                                profile_id: profile_id.clone(),
                            });
                        }
                    }
                    Ok(Err(e)) => {
                        let msg = e.to_string();
                        tracing::error!("ASR failed: {msg}");
                        let _ = event_tx.send(EngineEvent::Error {
                            code: "asr".into(),
                            message: msg,
                        });
                    }
                    Err(_) => {
                        tracing::error!("ASR timed out");
                        let _ = event_tx.send(EngineEvent::Error {
                            code: "asr_timeout".into(),
                            message: "语音识别超时，请检查网络或服务配置后重试".into(),
                        });
                    }
                }

                set_state(&state_tx, &event_tx, RecorderState::Idle);
            }

            EngineCommand::Cancel => {
                tracing::info!("cancel recording requested");
                if let Some(tx) = stop_tx.take() {
                    let _ = tx.send(());
                }
                if let Some(h) = record_handle.take() {
                    h.abort();
                }
                set_state(&state_tx, &event_tx, RecorderState::Idle);
                tracing::info!("recording cancelled");
            }

            EngineCommand::GetConfig(reply) => {
                let _ = reply.send(config.clone());
            }

            EngineCommand::SetConfig(mut new_config, reply) => {
                new_config.normalize();
                cached_backend = None;
                let result = new_config.save().map_err(VoxError::Other);
                if result.is_ok() {
                    config = new_config;
                }
                let _ = reply.send(result);
            }

            EngineCommand::SetActiveProfile(profile_id, reply) => {
                if !matches!(*state_tx.borrow(), RecorderState::Idle) {
                    let _ = reply.send(Err(VoxError::InvalidTransition(
                        "profile can only be changed while idle".into(),
                    )));
                    continue;
                }
                if !config.profiles.contains_key(&profile_id) {
                    let _ = reply.send(Err(VoxError::ProfileNotFound(profile_id)));
                    continue;
                }
                if config.active_profile_id != profile_id {
                    config.active_profile_id = profile_id;
                    cached_backend = None;
                }
                let result = config.save().map_err(VoxError::Other);
                let _ = reply.send(result);
            }

            EngineCommand::Shutdown => {
                tracing::info!("engine shutdown requested");
                if let Some(tx) = stop_tx.take() {
                    let _ = tx.send(());
                }
                if let Some(h) = record_handle.take() {
                    h.abort();
                }
                return;
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AudioLevel {
    rms: f32,
    peak: f32,
}

fn audio_level(samples: &[f32]) -> AudioLevel {
    if samples.is_empty() {
        return AudioLevel { rms: 0.0, peak: 0.0 };
    }

    let mut sum_squares = 0.0f64;
    let mut peak = 0.0f32;
    for sample in samples {
        let amplitude = sample.abs();
        peak = peak.max(amplitude);
        sum_squares += (*sample as f64) * (*sample as f64);
    }

    AudioLevel {
        rms: (sum_squares / samples.len() as f64).sqrt() as f32,
        peak,
    }
}

async fn run_asr(
    samples: Vec<f32>,
    profile_id: &str,
    profile: Option<&vf_config::LanguageProfile>,
    config: &AppConfig,
    cached: &mut Option<(String, Box<dyn AsrBackend>)>,
) -> Result<vf_asr::TranscribeResult, VoxError> {
    // Get or create backend for the active profile
    let need_new = cached.as_ref().map(|(id, _)| id != profile_id).unwrap_or(true);

    if need_new {
        let backend = build_backend(profile_id, profile, config)?;
        *cached = Some((profile_id.to_string(), backend));
    }

    let (language_hint, prompt) = profile
        .map(|p| (p.language_hint.clone(), p.prompt.clone()))
        .unwrap_or((None, None));

    let req = TranscribeRequest {
        samples,
        sample_rate: TARGET_SAMPLE_RATE,
        language_hint,
        prompt,
    };

    let (_, backend) = cached.as_mut().unwrap();
    backend.prepare().await.map_err(|e| VoxError::Other(anyhow::anyhow!("prepare: {e}")))?;
    backend.transcribe(req).await.map_err(|e| VoxError::Other(anyhow::anyhow!("{e}")))
}

fn build_backend(
    profile_id: &str,
    profile: Option<&vf_config::LanguageProfile>,
    config: &AppConfig,
) -> Result<Box<dyn AsrBackend>, VoxError> {
    match profile.map(|p| &p.backend) {
        Some(ProfileBackendConfig::OpenAi { model, api_key, transport }) => {
            ensure_rest_batch(*transport)?;
            tracing::info!("building OpenAI ASR backend for profile '{profile_id}' model '{model}'");
            let (raw_key, key_source) = api_key.as_deref()
                .filter(|k| !k.is_empty())
                .map(|k| (k, "profile override"))
                .or_else(|| config.provider_key("openai").map(|k| (k, "provider_keys.openai")))
                .or_else(|| {
                    (!config.app.openai_api_key.is_empty())
                        .then_some((config.app.openai_api_key.as_str(), "legacy openai_api_key"))
                })
                .unwrap_or((config.app.global_api_key.as_str(), "legacy global_api_key"));
            let key = normalize_api_key(raw_key);
            if key.is_empty() {
                return Err(VoxError::Other(anyhow::anyhow!(
                    "OpenAI API key not configured. Set it in Settings."
                )));
            }
            tracing::info!("using OpenAI API key from {key_source}: {}", key_fingerprint(&key));
            Ok(Box::new(OpenAiBackend::new(key, model.as_str())))
        }
        Some(ProfileBackendConfig::VoxNexus {
            model_id,
            api_key,
            transport,
            enable_timestamps,
            enable_speaker_diarization,
        }) => {
            tracing::info!("building VoxNexus ASR backend for profile '{profile_id}' model '{model_id}'");
            let (raw_key, key_source) = api_key.as_deref()
                .filter(|k| !k.is_empty())
                .map(|k| (k, "profile override"))
                .or_else(|| config.provider_key("voxnexus").map(|k| (k, "provider_keys.voxnexus")))
                .unwrap_or((config.app.voxnexus_api_key.as_str(), "legacy voxnexus_api_key"));
            let key = normalize_api_key(raw_key);
            if key.is_empty() {
                return Err(VoxError::Other(anyhow::anyhow!(
                    "VoxNexus API key not configured. Set it in Settings."
                )));
            }
            tracing::info!("using VoxNexus API key from {key_source}: {}", key_fingerprint(&key));
            if !matches!(transport, AsrTransportKind::RestBatch | AsrTransportKind::WebSocketStreaming) {
                return Err(VoxError::Other(anyhow::anyhow!(
                    "VoxNexus supports rest_batch and web_socket_streaming transports"
                )));
            }
            Ok(Box::new(VoxNexusBackend::new(
                key,
                model_id.as_str(),
                *enable_timestamps,
                *enable_speaker_diarization,
            ).with_transport(to_asr_transport(*transport))))
        }
        Some(ProfileBackendConfig::Local { transport, .. }) => {
            if !matches!(transport, AsrTransportKind::LocalBatch) {
                return Err(VoxError::Other(anyhow::anyhow!(
                    "local streaming ASR is not implemented yet"
                )));
            }
            Err(VoxError::Other(anyhow::anyhow!("local ASR not yet implemented (Phase 7)")))
        }
        None => {
            // Profile not found — fall back to cloud with global key and default model
            tracing::warn!("profile '{profile_id}' not found, using default cloud config");
            let key = &config.app.global_api_key;
            if key.is_empty() {
                return Err(VoxError::Other(anyhow::anyhow!(
                    "OpenAI API key not configured. Set it in Settings."
                )));
            }
            Ok(Box::new(OpenAiBackend::new(key, "gpt-4o-transcribe")))
        }
    }
}

fn ensure_rest_batch(transport: AsrTransportKind) -> Result<(), VoxError> {
    if matches!(transport, AsrTransportKind::RestBatch) {
        Ok(())
    } else {
        Err(VoxError::Other(anyhow::anyhow!(
            "streaming transport is reserved for a later phase; switch this profile to rest_batch"
        )))
    }
}

fn to_asr_transport(transport: AsrTransportKind) -> vf_asr::AsrTransportKind {
    match transport {
        AsrTransportKind::RestBatch => vf_asr::AsrTransportKind::RestBatch,
        AsrTransportKind::WebSocketStreaming => vf_asr::AsrTransportKind::WebSocketStreaming,
        AsrTransportKind::LocalBatch => vf_asr::AsrTransportKind::LocalBatch,
        AsrTransportKind::LocalStreaming => vf_asr::AsrTransportKind::LocalStreaming,
    }
}

fn normalize_api_key(key: &str) -> String {
    let trimmed = key.trim().trim_matches('"').trim_matches('\'').trim();
    trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

fn key_fingerprint(key: &str) -> String {
    let len = key.chars().count();
    let suffix_rev: String = key.chars().rev().take(4).collect();
    let suffix: String = suffix_rev.chars().rev().collect();
    format!("len={len}, suffix=...{suffix}")
}

async fn accumulate(
    mut capture: AudioCapture,
    stop: oneshot::Receiver<()>,
    max_secs: u32,
    event_tx: broadcast::Sender<EngineEvent>,
) -> Vec<f32> {
    let native_rate = capture.native_rate;
    let mut buf: Vec<f32> = Vec::with_capacity(native_rate as usize * max_secs as usize);
    let mut last_level_at = Instant::now() - Duration::from_millis(120);

    tokio::select! {
        _ = stop => {}
        _ = tokio::time::sleep(std::time::Duration::from_secs(max_secs as u64)) => {
            tracing::info!("max recording duration reached ({max_secs}s), auto-stopping");
        }
        _ = async {
            while let Some(chunk) = capture.rx.recv().await {
                if last_level_at.elapsed() >= Duration::from_millis(50) {
                    let level = audio_level(&chunk);
                    let _ = event_tx.send(EngineEvent::AudioLevel {
                        rms: level.rms,
                        peak: level.peak,
                    });
                    last_level_at = Instant::now();
                }
                buf.extend_from_slice(&chunk);
            }
        } => {}
    }

    // Resample from native device rate to 16 kHz in one pass.
    // Doing it here (not in the callback) avoids blocking the audio thread and
    // eliminates per-chunk zero-padding artefacts — only the negligible tail is padded.
    if native_rate == TARGET_SAMPLE_RATE {
        return buf;
    }
    match AudioResampler::new(native_rate, TARGET_SAMPLE_RATE, 1) {
        Ok(mut r) => r.process_to_mono_16k(&buf).unwrap_or_else(|e| {
            tracing::warn!("resample failed: {e}");
            buf
        }),
        Err(e) => {
            tracing::warn!("resampler init failed: {e}");
            buf
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use vf_asr::{AsrBackend, TranscribeRequest, TranscribeResult, AsrError};
    use vf_config::AppConfig;

    struct MockAsrBackend {
        response: Result<TranscribeResult, String>,
    }

    #[async_trait::async_trait]
    impl AsrBackend for MockAsrBackend {
        fn name(&self) -> &'static str { "mock" }
        async fn is_ready(&self) -> bool { true }
        async fn prepare(&mut self) -> Result<(), AsrError> { Ok(()) }
        async fn transcribe(&self, _req: TranscribeRequest) -> Result<TranscribeResult, AsrError> {
            match &self.response {
                Ok(r) => Ok(r.clone()),
                Err(msg) => Err(AsrError::Api(msg.clone())),
            }
        }
    }

    fn engine_with_mock(result: Result<&str, &str>) -> VoxEngine {
        let config = AppConfig::default();
        let profile_id = config.active_profile_id.clone();
        let response = result.map(
            |t| TranscribeResult { text: t.into(), language_detected: None, duration_ms: 10 }
        ).map_err(|e| e.to_string());
        let backend: Box<dyn AsrBackend> = Box::new(MockAsrBackend { response });
        VoxEngine::new_for_test(config, Some((profile_id, backend)))
    }

    #[tokio::test]
    async fn test_initial_state() {
        let engine = VoxEngine::new(AppConfig::default());
        assert_eq!(engine.state(), RecorderState::Idle);
    }

    #[tokio::test]
    async fn test_cancel_when_idle_is_noop() {
        let engine = VoxEngine::new(AppConfig::default());
        engine.cancel().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(engine.state(), RecorderState::Idle);
    }

    #[tokio::test]
    async fn test_stop_when_idle_is_noop() {
        let engine = VoxEngine::new(AppConfig::default());
        engine.stop_recording().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(engine.state(), RecorderState::Idle);
    }

    #[tokio::test]
    async fn test_get_config_returns_stored_key() {
        let mut config = AppConfig::default();
        config.app.global_api_key = "sk-test-1234".into();
        let engine = VoxEngine::new(config);
        let got = engine.get_config().await;
        assert_eq!(got.app.global_api_key, "sk-test-1234");
    }

    // These tests require a physical audio device — run manually with: cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn test_full_flow_asr_success() {
        let engine = engine_with_mock(Ok("hello world"));
        let mut events = engine.subscribe_events();

        engine.start_recording().await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(engine.state(), RecorderState::Recording);

        engine.stop_recording().await.unwrap();

        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await.expect("timeout waiting for event").expect("channel closed");

        match event {
            EngineEvent::Transcription { text, .. } => assert_eq!(text, "hello world"),
            EngineEvent::Error { message, .. } => panic!("unexpected error: {message}"),
            EngineEvent::StateChanged { .. } | EngineEvent::AudioLevel { .. } => {}
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(engine.state(), RecorderState::Idle);
    }

    #[tokio::test]
    #[ignore]
    async fn test_full_flow_asr_error_recovers_to_idle() {
        let engine = engine_with_mock(Err("asr failed"));
        let mut events = engine.subscribe_events();

        engine.start_recording().await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        engine.stop_recording().await.unwrap();

        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await.expect("timeout").expect("channel closed");

        assert!(matches!(event, EngineEvent::Error { .. }));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(engine.state(), RecorderState::Idle);

        // Verify recording can start again after an error
        engine.start_recording().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(engine.state(), RecorderState::Recording);
        engine.cancel().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(engine.state(), RecorderState::Idle);
    }

    #[tokio::test]
    #[ignore]
    async fn test_cancel_during_recording() {
        let engine = engine_with_mock(Ok("anything"));
        engine.start_recording().await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(engine.state(), RecorderState::Recording);
        engine.cancel().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(engine.state(), RecorderState::Idle);
    }
}

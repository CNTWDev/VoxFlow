use thiserror::Error;

#[derive(Debug, Error)]
pub enum VoxError {
    #[error("audio error: {0}")]
    Audio(#[from] vf_audio::AudioError),
    #[error("asr error: {0}")]
    Asr(#[from] vf_asr::AsrError),
    #[error("inject error: {0}")]
    Inject(#[from] vf_inject::InjectError),
    #[error("profile not found: {0}")]
    ProfileNotFound(String),
    #[error("invalid state transition: {0}")]
    InvalidTransition(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

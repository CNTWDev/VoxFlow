use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("no input device available")]
    NoInputDevice,
    #[error("unsupported stream config: {0}")]
    UnsupportedConfig(String),
    #[error("stream error: {0}")]
    Stream(String),
    #[error("resampler error: {0}")]
    Resampler(String),
}

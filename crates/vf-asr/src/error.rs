use thiserror::Error;

#[derive(Debug, Error)]
pub enum AsrError {
    #[error("API error: {0}")]
    Api(String),
    #[error("audio encoding failed: {0}")]
    Encoding(String),
    #[error("model not loaded: {0}")]
    ModelNotLoaded(String),
    #[error("timeout")]
    Timeout,
    #[error("unsupported transport: {0}")]
    UnsupportedTransport(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

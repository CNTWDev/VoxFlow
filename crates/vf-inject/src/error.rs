use thiserror::Error;

#[derive(Debug, Error)]
pub enum InjectError {
    #[error("no text to input")]
    EmptyText,
    #[error("clipboard error: {0}")]
    Clipboard(String),
    #[error("key simulation error: {0}")]
    Keys(String),
}

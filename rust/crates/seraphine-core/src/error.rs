use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("LCU is not connected")]
    NotConnected,
    #[error("invalid parameters: {0}")]
    InvalidParams(String),
    #[error("platform error: {0}")]
    Platform(String),
    #[error("LCU error: {0}")]
    Lcu(String),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("resource deferred: {0}")]
    Deferred(String),
    #[error("operation conflict: {0}")]
    OperationConflict(String),
    #[error("outcome unknown: {0}")]
    OutcomeUnknown(String),
    #[error("invalid state: {0}")]
    InvalidState(String),
}

impl CoreError {
    pub fn io(error: impl std::fmt::Display) -> Self {
        Self::Io(error.to_string())
    }

    pub fn json(error: impl std::fmt::Display) -> Self {
        Self::Config(error.to_string())
    }
}

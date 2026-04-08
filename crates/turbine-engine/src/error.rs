use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("PHP embed SAPI initialization failed")]
    InitFailed,

    #[error("PHP request startup failed")]
    RequestStartupFailed,

    #[error("PHP eval failed for: {code}")]
    EvalFailed { code: String },

    #[error("PHP engine already initialized")]
    AlreadyInitialized,

    #[error("PHP engine not initialized")]
    NotInitialized,

    #[error("PHP code contains null byte")]
    NullByteInCode,

    #[error("PHP request lifecycle error")]
    RequestLifecycle,
}

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphRelayError {
    #[error("symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("file not indexed: {0}")]
    FileNotIndexed(String),

    #[error("unsupported language for: {0}")]
    UnsupportedLanguage(String),

    #[error("index not ready")]
    IndexNotReady,

    #[error("embedding model not loaded: {0}")]
    ModelNotLoaded(String),

    #[error("store error: {0}")]
    StoreError(String),

    #[error("parse error for {path}: {message}")]
    ParseError { path: String, message: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

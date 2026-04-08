use thiserror::Error;

#[derive(Debug, Error)]
pub enum CorpusError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Bincode(#[from] bincode::Error),
}

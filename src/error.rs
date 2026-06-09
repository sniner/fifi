#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("hash algorithm not yet implemented: {0}")]
    UnsupportedAlgo(&'static str),
}

pub type Result<T> = std::result::Result<T, ScanError>;

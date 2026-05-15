use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("hash algorithm not yet implemented: {0}")]
    UnsupportedAlgo(&'static str),

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, ScanError>;

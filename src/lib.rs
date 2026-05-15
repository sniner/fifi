pub mod error;
pub mod hash;
pub mod model;
pub mod pipeline;
pub mod progress;
pub mod scanner;
pub mod util;

pub use error::{Result, ScanError};
pub use hash::{
    DigestHasher, DigestKey, FullHashStrategy, PARTIAL_THRESHOLD, PARTIAL_WINDOW, Sha256Hasher,
    Xxh3Hasher, hex,
};
pub use model::{DuplicateGroup, FileEntry, ScanResult};
pub use progress::{ProgressSink, TracingProgress};
pub use scanner::{ScanOptions, dedup_hardlinks, walk_paths};

use std::path::PathBuf;

pub fn scan(paths: &[PathBuf], opts: &ScanOptions) -> Result<ScanResult> {
    tracing::info!("Scanning {} path(s)...", paths.len());
    let mut entries = walk_paths(paths, opts);
    if !opts.per_path {
        entries = dedup_hardlinks(entries);
    }
    pipeline::run_pipeline(entries, opts)
}

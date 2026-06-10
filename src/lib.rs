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
pub use model::{DuplicateGroup, FileEntry, OrderBy, ScanResult};
pub use progress::{ProgressSink, TracingProgress};
pub use scanner::{ScanOptions, WalkResult, dedup_hardlinks, walk_paths};

use std::path::PathBuf;

/// Walk `paths`, collapse hardlink families (unless `opts.per_path`), and
/// detect duplicate files.
///
/// # Errors
///
/// Returns [`ScanError::UnsupportedAlgo`] when the configured hasher
/// reports itself unavailable. Unreadable files and skipped directories
/// are reported inside the [`ScanResult`], not as errors.
pub fn scan(paths: &[PathBuf], opts: &ScanOptions) -> Result<ScanResult> {
    tracing::info!("Scanning {} path(s)...", paths.len());
    let walk = walk_paths(paths, opts);
    let entries = if opts.per_path {
        walk.files
    } else {
        dedup_hardlinks(walk.files)
    };
    let mut result = pipeline::run_pipeline(entries, opts)?;
    result.skipped_dirs = walk.skipped_dirs;
    Ok(result)
}

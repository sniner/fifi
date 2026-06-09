use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub path: PathBuf,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<PathBuf>,

    pub size: u64,

    /// Inode birth time as Unix seconds with sub-second precision. On
    /// filesystems without a birth time (POSIX doesn't mandate one), the
    /// fallback is `min(mtime, ctime)`. See `scanner::age_seconds` for the
    /// full rationale.
    pub age: f64,

    pub hash: Option<String>,

    #[serde(skip)]
    pub dev: u64,
    #[serde(skip)]
    pub ino: u64,
}

#[derive(Debug, Clone)]
pub struct DuplicateGroup {
    pub entries: Vec<FileEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub unique: Vec<FileEntry>,
    pub duplicates: Vec<DuplicateGroup>,
    pub unreadable: Vec<FileEntry>,

    /// Number of directories the walk could not enter (permission errors
    /// and the like). Non-zero means the scan was incomplete.
    pub skipped_dirs: usize,
}

impl ScanResult {
    #[must_use]
    pub fn has_duplicates(&self) -> bool {
        !self.duplicates.is_empty()
    }
}

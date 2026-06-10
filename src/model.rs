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

    /// Index of the scan root (command-line path argument, in argument order)
    /// this file was found under. Drives `OrderBy::Source`. For a merged
    /// hardlink family it is the canonical entry's root; the aliases' roots
    /// are not tracked separately.
    #[serde(skip)]
    pub root: usize,
}

#[derive(Debug, Clone)]
pub struct DuplicateGroup {
    pub entries: Vec<FileEntry>,
}

/// How to order the members within a duplicate group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrderBy {
    /// Oldest first — the original-detection heuristic. The first member is
    /// the presumed "original", the one `--dupes-only` keeps.
    #[default]
    Age,
    /// By the scan root (command-line path argument) each file was found
    /// under, in argument order, then by path (shallowest first). The first
    /// member is the one from the earliest-listed path — for `fifi orig
    /// backup`, that keeps the `orig` copy and prunes the `backup` one. Within
    /// a single root this degenerates to the path heuristic.
    Source,
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

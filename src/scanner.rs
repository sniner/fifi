use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use walkdir::WalkDir;

use crate::hash::FullHashStrategy;
use crate::model::FileEntry;
use crate::platform;
use crate::progress::ProgressSink;
use crate::util::dup_sort;

pub struct ScanOptions {
    pub follow: bool,
    pub include_hidden: bool,
    pub one_file_system: bool,
    pub per_path: bool,
    /// Maximum number of subdirectory levels to descend below each root.
    /// `Some(0)` scans only the files directly in the named directories;
    /// `Some(N)` allows N levels of subdirectory descent; `None` is
    /// unbounded.
    pub depth: Option<usize>,
    pub algo: FullHashStrategy,
    pub progress: Option<Arc<dyn ProgressSink>>,
}

impl ScanOptions {
    pub fn new(algo: FullHashStrategy) -> Self {
        Self {
            follow: false,
            include_hidden: false,
            one_file_system: false,
            per_path: false,
            depth: None,
            algo,
            progress: None,
        }
    }
}

fn entry_from_metadata(path: PathBuf, meta: &fs::Metadata) -> io::Result<FileEntry> {
    let (dev, ino) = platform::file_id(&path, meta)?;
    Ok(FileEntry {
        path,
        aliases: Vec::new(),
        size: meta.len(),
        age: platform::age_seconds(meta),
        hash: None,
        dev,
        ino,
    })
}

fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

fn walk_path(
    root: &Path,
    files: &mut Vec<FileEntry>,
    progress: &Option<Arc<dyn ProgressSink>>,
    opts: &ScanOptions,
) {
    // Follow symlinks at the root level (matches Python: tests is_dir/is_file
    // on the root, both of which follow links). The --follow flag only
    // changes behavior *inside* the walk.
    let meta = match fs::metadata(root) {
        Ok(m) => m,
        Err(e) => {
            tracing::info!("'{}' not found: {}", root.display(), e);
            return;
        }
    };

    if meta.is_file() {
        match entry_from_metadata(root.to_path_buf(), &meta) {
            Ok(e) => {
                files.push(e);
                if let Some(p) = &progress {
                    p.tick(&format!("Scanned {} file(s) so far...", files.len()));
                }
            }
            Err(e) => tracing::error!("Cannot stat '{}': {}", root.display(), e),
        }
        return;
    }

    if !meta.is_dir() {
        tracing::debug!(
            "Ignoring '{}' (not a regular file or directory)",
            root.display()
        );
        return;
    }

    let mut walker = WalkDir::new(root)
        .follow_links(opts.follow)
        .same_file_system(opts.one_file_system);
    if let Some(d) = opts.depth {
        // walkdir treats the root as depth 0 and its direct children as
        // depth 1, so a user-facing "0 subdirectory descents" maps to
        // max_depth(1).
        walker = walker.max_depth(d.saturating_add(1));
    }
    let walker = walker.into_iter().filter_entry(|de| {
        if de.depth() == 0 {
            return true;
        }
        let name = de.file_name().to_string_lossy();
        if !opts.include_hidden && is_hidden_name(&name) {
            tracing::debug!("Ignoring hidden '{}'", de.path().display());
            return false;
        }
        if !opts.follow && de.file_type().is_symlink() {
            tracing::debug!("Ignoring symlink '{}'", de.path().display());
            return false;
        }
        true
    });

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                // walkdir surfaces loop detection (under follow_links) and
                // permission errors here. Demote to debug so they don't
                // interrupt scans.
                tracing::debug!("walk error: {}", e);
                continue;
            }
        };
        let ft = entry.file_type();
        if !ft.is_file() {
            continue;
        }
        // metadata() follows symlinks; for follow_links(false) the symlink
        // entries were already filtered above.
        let m = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("Cannot stat '{}': {}", entry.path().display(), e);
                continue;
            }
        };
        match entry_from_metadata(entry.path().to_path_buf(), &m) {
            Ok(e) => {
                files.push(e);
                if let Some(p) = &progress {
                    p.tick(&format!("Scanned {} file(s) so far...", files.len()));
                }
            }
            Err(e) => tracing::error!("Cannot stat '{}': {}", entry.path().display(), e),
        }
    }
}

pub fn walk_paths(roots: &[PathBuf], opts: &ScanOptions) -> Vec<FileEntry> {
    let mut files: Vec<FileEntry> = Vec::new();
    let progress = opts.progress.clone();

    for root in roots {
        walk_path(root, &mut files, &progress, opts)
    }

    files
}

/// Collapse files that share `(dev, ino)` into one canonical `FileEntry`,
/// stashing the other paths in `aliases`. Empty `aliases` after this means
/// the inode has only one path on disk.
///
/// We group by `(dev, ino, size)` rather than `(dev, ino)` alone as a
/// safety net against filesystems that synthesize inode numbers (most
/// commonly SMB/CIFS and WebDAV), where the same fake inode can appear
/// across distinct files. Genuine hardlinks share their size by
/// construction, so they're still merged correctly; impostors with
/// mismatched sizes stay separate and produce a warning.
pub fn dedup_hardlinks(entries: Vec<FileEntry>) -> Vec<FileEntry> {
    let mut groups: HashMap<(u64, u64, u64), Vec<FileEntry>> = HashMap::new();
    for e in entries {
        groups.entry((e.dev, e.ino, e.size)).or_default().push(e);
    }

    // Surface synthetic-inode collisions: the same (dev, ino) appearing
    // under multiple sizes means the filesystem isn't being honest about
    // file identity. We've already kept those entries separate above; the
    // warning tells the user what's going on and points at the workaround.
    let mut sizes_per_devino: HashMap<(u64, u64), usize> = HashMap::new();
    for &(dev, ino, _size) in groups.keys() {
        *sizes_per_devino.entry((dev, ino)).or_default() += 1;
    }
    for ((dev, ino), n) in &sizes_per_devino {
        if *n > 1 {
            tracing::warn!(
                "synthetic inode detected: (dev={dev}, ino={ino}) reported with {n} different sizes \
                 — these files will not be merged as hardlinks. This is typical of SMB/CIFS or WebDAV \
                 mounts; pass --per-path to disable hardlink dedup entirely if your share's inodes are \
                 not trustworthy."
            );
        }
    }

    let mut out = Vec::with_capacity(groups.len());
    for (_, members) in groups {
        if members.len() == 1 {
            out.push(members.into_iter().next().unwrap());
            continue;
        }
        let sorted = dup_sort(&members);
        let mut iter = sorted.into_iter();
        let mut canonical = iter.next().unwrap();
        canonical.aliases = iter.map(|e| e.path).collect();
        out.push(canonical);
    }
    out
}

pub(crate) fn _ignore_path<P: AsRef<Path>>(_p: P) {
    // Reserved for future use: path-based exclude filters in step 2.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, dev: u64, ino: u64, age: f64) -> FileEntry {
        entry_with_size(path, dev, ino, age, 10)
    }

    fn entry_with_size(path: &str, dev: u64, ino: u64, age: f64, size: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            aliases: Vec::new(),
            size,
            age,
            hash: None,
            dev,
            ino,
        }
    }

    #[test]
    fn dedup_hardlinks_keeps_singletons() {
        let v = vec![entry("a", 1, 1, 0.0), entry("b", 1, 2, 0.0)];
        let out = dedup_hardlinks(v);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|e| e.aliases.is_empty()));
    }

    #[test]
    fn dedup_hardlinks_collapses_same_inode() {
        let v = vec![
            entry("a/long-name", 1, 1, 100.0),
            entry("a/x", 1, 1, 100.0),
            entry("a/medium", 1, 1, 100.0),
        ];
        let out = dedup_hardlinks(v);
        assert_eq!(out.len(), 1);
        // dup_sort heuristic: shortest path wins
        assert_eq!(out[0].path, PathBuf::from("a/x"));
        assert_eq!(out[0].aliases.len(), 2);
    }

    #[test]
    fn dedup_hardlinks_separates_distinct_devices() {
        let v = vec![entry("a", 1, 1, 0.0), entry("b", 2, 1, 0.0)];
        let out = dedup_hardlinks(v);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedup_hardlinks_refuses_to_merge_size_mismatched_entries() {
        // Same (dev, ino) tuple split across two sizes — i.e. the
        // filesystem is lying about inode identity (typical of SMB/CIFS).
        // The two real-content-size-100 entries must still merge into one
        // entry; the impostor with size 200 must stay standalone.
        let v = vec![
            entry_with_size("a", 1, 1, 0.0, 100),
            entry_with_size("b", 1, 1, 0.0, 100),
            entry_with_size("c", 1, 1, 0.0, 200),
        ];
        let out = dedup_hardlinks(v);
        assert_eq!(out.len(), 2);

        let merged = out
            .iter()
            .find(|e| !e.aliases.is_empty())
            .expect("the size-100 pair should have merged");
        assert_eq!(merged.size, 100);
        assert_eq!(merged.aliases.len(), 1);

        let standalone = out
            .iter()
            .find(|e| e.aliases.is_empty())
            .expect("the size-200 impostor should be standalone");
        assert_eq!(standalone.size, 200);
        assert_eq!(standalone.path, PathBuf::from("c"));
    }
}

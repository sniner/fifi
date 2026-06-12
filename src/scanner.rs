use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use walkdir::WalkDir;

use crate::hash::FullHashStrategy;
use crate::model::{FileEntry, OrderBy};
use crate::progress::ProgressSink;
use crate::util::dup_sort;

// One bool per independent CLI flag — folding them into bitflags or
// sub-structs would only obscure the 1:1 mapping to the command line.
#[allow(clippy::struct_excessive_bools)]
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
    /// How to order members within each duplicate group.
    pub order_by: OrderBy,
    pub progress: Option<Arc<dyn ProgressSink>>,
}

impl ScanOptions {
    #[must_use]
    pub fn new(algo: FullHashStrategy) -> Self {
        Self {
            follow: false,
            include_hidden: false,
            one_file_system: false,
            per_path: false,
            depth: None,
            algo,
            order_by: OrderBy::default(),
            progress: None,
        }
    }
}

fn entry_from_metadata(path: PathBuf, meta: &fs::Metadata, root: usize) -> FileEntry {
    FileEntry {
        path,
        aliases: Vec::new(),
        size: meta.size(),
        age: age_seconds(meta),
        hash: None,
        dev: meta.dev(),
        ino: meta.ino(),
        root,
    }
}

/// File age as Unix seconds, used to pick the "original" within a
/// duplicate group.
///
/// Prefers the real inode birth time via `Metadata::created()`, which on
/// Linux 4.11+ reads `statx()` and works on ext4/btrfs/xfs/f2fs, and on
/// macOS reads `st_birthtime`. POSIX doesn't mandate a creation time, so
/// filesystems like ext3 or older mounts fall back to
/// `min(mtime, ctime)`: ctime advances on any inode-touch (chmod, chown,
/// link-count change, ...) even when the content hasn't changed, and
/// mtime can be set arbitrarily via `touch -d`. The smaller of the two
/// picks the older signal in both cases.
//
// Unix timestamps fit comfortably into f64's 52-bit mantissa for any
// plausible date, and the nanosecond fields are < 1e9 — the precision
// loss clippy warns about cannot occur here.
#[allow(clippy::cast_precision_loss)]
fn age_seconds(meta: &fs::Metadata) -> f64 {
    if let Ok(created) = meta.created() {
        if let Ok(d) = created.duration_since(UNIX_EPOCH) {
            return d.as_secs_f64();
        }
    }
    let mtime = meta.mtime() as f64 + meta.mtime_nsec() as f64 / 1_000_000_000.0;
    let ctime = meta.ctime() as f64 + meta.ctime_nsec() as f64 / 1_000_000_000.0;
    if mtime < ctime { mtime } else { ctime }
}

fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

/// Outcome of the directory walk: the files collected plus the number of
/// directories that could not be entered (permission errors and the like).
/// A non-zero `skipped_dirs` means the scan results are incomplete.
pub struct WalkResult {
    pub files: Vec<FileEntry>,
    pub skipped_dirs: usize,
    /// Roots that could not be accessed at all (missing or
    /// permission-denied). The walk is tolerant — recording instead of
    /// failing — so callers decide whether this is an error.
    pub missing_roots: Vec<PathBuf>,
}

fn walk_path(
    root: &Path,
    root_idx: usize,
    out: &mut WalkResult,
    progress: Option<&dyn ProgressSink>,
    opts: &ScanOptions,
) {
    // Follow symlinks at the root level (matches Python: tests is_dir/is_file
    // on the root, both of which follow links). The --follow flag only
    // changes behavior *inside* the walk.
    let meta = match fs::metadata(root) {
        Ok(m) => m,
        Err(e) => {
            tracing::info!("'{}' not found: {}", root.display(), e);
            out.missing_roots.push(root.to_path_buf());
            return;
        }
    };

    if meta.is_file() {
        out.files
            .push(entry_from_metadata(root.to_path_buf(), &meta, root_idx));
        if let Some(p) = progress {
            p.tick(&format!("Scanned {} file(s) so far...", out.files.len()));
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
                // Loop detection under follow_links is expected noise.
                // Everything else is typically EACCES on a directory: the
                // subtree silently vanishes from the results otherwise, so
                // surface it at warn level and count it — callers can tell
                // an incomplete scan from a clean one.
                if e.loop_ancestor().is_some() {
                    tracing::debug!("walk error: {}", e);
                } else {
                    out.skipped_dirs += 1;
                    tracing::warn!("{}", e);
                }
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
        out.files.push(entry_from_metadata(
            entry.path().to_path_buf(),
            &m,
            root_idx,
        ));
        if let Some(p) = progress {
            p.tick(&format!("Scanned {} file(s) so far...", out.files.len()));
        }
    }
}

pub fn walk_paths(roots: &[PathBuf], opts: &ScanOptions) -> WalkResult {
    let mut out = WalkResult {
        files: Vec::new(),
        skipped_dirs: 0,
        missing_roots: Vec::new(),
    };
    let progress = opts.progress.as_deref();

    // Scan each distinct root once: `fifi x x` would otherwise report every
    // file as its own duplicate under --per-path. Nested roots (one inside
    // another) stay legitimate — with --depth the outer walk may not reach
    // the inner root at all — so they only get a warning.
    // `.enumerate()` over the original argument list: the index is the file's
    // scan-root identity for `OrderBy::Source`. Skipped (duplicate) roots
    // leave gaps, but the surviving roots keep their command-line position, so
    // relative order is preserved.
    let mut seen: Vec<PathBuf> = Vec::new();
    for (root_idx, root) in roots.iter().enumerate() {
        let canon = fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        if seen.contains(&canon) {
            tracing::warn!("Skipping duplicate root '{}'", root.display());
            continue;
        }
        if let Some(other) = seen
            .iter()
            .find(|s| canon.starts_with(s) || s.starts_with(&canon))
        {
            tracing::warn!(
                "Root '{}' overlaps with '{}' — files reachable from both will be \
                 reported twice (as false duplicates under --per-path)",
                root.display(),
                other.display()
            );
        }
        seen.push(canon);
        walk_path(root, root_idx, &mut out, progress, opts);
    }

    out
}

/// Collapse files that share `(dev, ino)` into one canonical `FileEntry`,
/// stashing the other paths in `aliases`. Empty `aliases` after this means
/// the inode has only one path on disk.
///
/// We group by `(dev, ino, size)` rather than `(dev, ino)` alone as a
/// safety net against filesystems that synthesize inode numbers (most
/// commonly SMB/CIFS and `WebDAV`), where the same fake inode can appear
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
        // Singletons skip the dup_sort (and its clone); for hardlink
        // families the heuristically-first entry becomes canonical and
        // the rest turn into aliases.
        let sorted = if members.len() == 1 {
            members
        } else {
            dup_sort(&members)
        };
        let mut iter = sorted.into_iter();
        let Some(mut canonical) = iter.next() else {
            continue;
        };
        canonical.aliases = iter.map(|e| e.path).collect();
        out.push(canonical);
    }
    out
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
            root: 0,
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

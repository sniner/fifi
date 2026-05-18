use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::UNIX_EPOCH;

pub fn file_id(_path: &Path, meta: &fs::Metadata) -> io::Result<(u64, u64)> {
    Ok((meta.dev(), meta.ino()))
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
pub fn age_seconds(meta: &fs::Metadata) -> f64 {
    if let Ok(created) = meta.created() {
        if let Ok(d) = created.duration_since(UNIX_EPOCH) {
            return d.as_secs_f64();
        }
    }
    let mtime = meta.mtime() as f64 + meta.mtime_nsec() as f64 / 1_000_000_000.0;
    let ctime = meta.ctime() as f64 + meta.ctime_nsec() as f64 / 1_000_000_000.0;
    if mtime < ctime { mtime } else { ctime }
}

use std::fs;
use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::time::UNIX_EPOCH;

// Windows file IDs come from `GetFileInformationByHandle`, which needs an
// open handle — not just stat-equivalent metadata. We re-open every file
// here. That's a per-file cost on Windows that Unix doesn't pay; the
// payoff is hardlink dedup that works the same way as on Unix.
pub fn file_id(path: &Path, _meta: &fs::Metadata) -> io::Result<(u64, u64)> {
    let handle = winapi_util::Handle::from_path_any(path)?;
    let info = winapi_util::file::information(&handle)?;
    Ok((info.volume_serial_number() as u64, info.file_index()))
}

// NTFS stores file times as 100-ns intervals since 1601-01-01 UTC.
// Offset to the Unix epoch (1970-01-01 UTC) is 11_644_473_600 seconds,
// i.e. 116_444_736_000_000_000 NTFS ticks.
const NTFS_TO_UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;

fn ntfs_ticks_to_unix_seconds(ticks: u64) -> Option<f64> {
    if ticks < NTFS_TO_UNIX_EPOCH_TICKS {
        return None;
    }
    let unix_ticks = ticks - NTFS_TO_UNIX_EPOCH_TICKS;
    Some(unix_ticks as f64 / 10_000_000.0)
}

/// File age as Unix seconds. NTFS reliably tracks creation time, so
/// `Metadata::created()` is the primary signal. Fallback to
/// `last_write_time()` keeps things sensible if creation time happens
/// to be unavailable (e.g. on FAT-formatted media).
pub fn age_seconds(meta: &fs::Metadata) -> f64 {
    if let Ok(created) = meta.created() {
        if let Ok(d) = created.duration_since(UNIX_EPOCH) {
            return d.as_secs_f64();
        }
    }
    ntfs_ticks_to_unix_seconds(meta.last_write_time()).unwrap_or(0.0)
}

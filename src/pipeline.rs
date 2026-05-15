use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use rayon::prelude::*;

use crate::error::{Result, ScanError};
use crate::hash::{
    DigestHasher, DigestKey, FullHashStrategy, PARTIAL_THRESHOLD, hex, partial_xxh3,
};

const BYTEWISE_BLOCK: usize = 1 << 20;
use crate::model::{DuplicateGroup, FileEntry, ScanResult};
use crate::progress::ProgressSink;
use crate::scanner::ScanOptions;
use crate::util::{dup_sort, human_size, natural_cmp};

struct BucketedGroup {
    multi: Vec<Vec<FileEntry>>,
    singletons: Vec<FileEntry>,
    unreadable: Vec<FileEntry>,
}

impl BucketedGroup {
    fn new() -> Self {
        Self {
            multi: Vec::new(),
            singletons: Vec::new(),
            unreadable: Vec::new(),
        }
    }
}

fn split_by_size(files: Vec<FileEntry>) -> (Vec<FileEntry>, Vec<Vec<FileEntry>>) {
    let mut by_size: HashMap<u64, Vec<FileEntry>> = HashMap::new();
    for f in files {
        by_size.entry(f.size).or_default().push(f);
    }
    let mut unique = Vec::new();
    let mut candidates = Vec::new();
    for (size, group) in by_size {
        if size == 0 {
            if group.len() > 1 {
                tracing::info!("{} empty files are not considered identical", group.len());
            }
            unique.extend(group);
        } else if group.len() == 1 {
            unique.extend(group);
        } else {
            candidates.push(group);
        }
    }
    (unique, candidates)
}

fn bucket_one_group_partial(
    group: Vec<FileEntry>,
    progress: Option<&dyn ProgressSink>,
) -> BucketedGroup {
    let mut out = BucketedGroup::new();
    if group.is_empty() {
        return out;
    }
    if group[0].size < PARTIAL_THRESHOLD {
        out.multi.push(group);
        return out;
    }
    let mut buckets: HashMap<DigestKey, Vec<FileEntry>> = HashMap::new();
    for f in group {
        if let Some(p) = progress {
            p.tick(&format!(
                "Partial-hashing {} ({})",
                f.path.display(),
                human_size(f.size)
            ));
        }
        match partial_xxh3(&f.path, f.size) {
            Ok(k) => buckets.entry(k).or_default().push(f),
            Err(e) => {
                tracing::error!("Unable to read '{}': {}", f.path.display(), e);
                out.unreadable.push(f);
            }
        }
    }
    for items in buckets.into_values() {
        if items.len() == 1 {
            out.singletons.extend(items);
        } else {
            out.multi.push(items);
        }
    }
    out
}

fn bucket_one_group_full(
    group: Vec<FileEntry>,
    hasher: &dyn DigestHasher,
    progress: Option<&dyn ProgressSink>,
) -> BucketedGroup {
    let mut out = BucketedGroup::new();
    let mut buckets: HashMap<DigestKey, Vec<FileEntry>> = HashMap::new();
    for mut f in group {
        if let Some(p) = progress {
            p.tick(&format!(
                "Full-hashing {} ({})",
                f.path.display(),
                human_size(f.size)
            ));
        }
        match hasher.digest(&f.path) {
            Ok(k) => {
                f.hash = Some(hex(&k));
                buckets.entry(k).or_default().push(f);
            }
            Err(e) => {
                tracing::error!("Unable to read '{}': {}", f.path.display(), e);
                out.unreadable.push(f);
            }
        }
    }
    for items in buckets.into_values() {
        if items.len() == 1 {
            out.singletons.extend(items);
        } else {
            out.multi.push(items);
        }
    }
    out
}

/// Compare two files byte-by-byte. Returns `Ok(true)` if every byte
/// matches (and both files end together), `Ok(false)` on first
/// mismatch or differing lengths. Lengths typically match — the caller
/// has already size-grouped — but we defend against the TOCTOU window
/// where a file was truncated between stat and open.
fn files_equal(a: &Path, b: &Path) -> io::Result<bool> {
    let mut fa = File::open(a)?;
    let mut fb = File::open(b)?;
    let mut buf_a = vec![0u8; BYTEWISE_BLOCK];
    let mut buf_b = vec![0u8; BYTEWISE_BLOCK];
    loop {
        let na = fa.read(&mut buf_a)?;
        let nb = fb.read(&mut buf_b)?;
        if na != nb {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
        if buf_a[..na] != buf_b[..nb] {
            return Ok(false);
        }
    }
}

fn bucket_one_group_bytewise(
    group: Vec<FileEntry>,
    progress: Option<&dyn ProgressSink>,
) -> BucketedGroup {
    let mut out = BucketedGroup::new();
    if group.is_empty() {
        return out;
    }
    // For each candidate entry, try to place it into an existing subgroup
    // whose first member's content matches byte-for-byte. If no match,
    // start a new subgroup. The typical case after partial-xxh3 is "all
    // identical", so most entries land in the first subgroup with one
    // comparison each (O(N) reads, O(N) comparisons).
    let mut subgroups: Vec<Vec<FileEntry>> = Vec::new();
    for entry in group {
        if let Some(p) = progress {
            p.tick(&format!(
                "Bytewise-comparing {} ({})",
                entry.path.display(),
                human_size(entry.size)
            ));
        }
        let mut handled = false;
        for sub in subgroups.iter_mut() {
            match files_equal(&entry.path, &sub[0].path) {
                Ok(true) => {
                    sub.push(entry.clone());
                    handled = true;
                    break;
                }
                Ok(false) => continue,
                Err(e) => {
                    tracing::error!(
                        "Unable to compare '{}' vs '{}': {}",
                        entry.path.display(),
                        sub[0].path.display(),
                        e
                    );
                    out.unreadable.push(entry.clone());
                    handled = true;
                    break;
                }
            }
        }
        if !handled {
            subgroups.push(vec![entry]);
        }
    }
    for sub in subgroups {
        if sub.len() == 1 {
            out.singletons.extend(sub);
        } else {
            out.multi.push(sub);
        }
    }
    out
}

fn merge(buckets: Vec<BucketedGroup>) -> BucketedGroup {
    let mut acc = BucketedGroup::new();
    for b in buckets {
        acc.multi.extend(b.multi);
        acc.singletons.extend(b.singletons);
        acc.unreadable.extend(b.unreadable);
    }
    acc
}

pub fn run_pipeline(files: Vec<FileEntry>, opts: &ScanOptions) -> Result<ScanResult> {
    let progress = opts.progress.as_deref();

    let total_files = files.len();
    let (mut unique, candidates) = split_by_size(files);
    tracing::info!(
        "Discovered {} file(s) in {} size group(s)",
        total_files,
        candidates.len() + unique.len()
    );

    // Phase 2: partial hash (parallel over groups).
    let partial_count: usize = candidates
        .iter()
        .filter(|g| {
            g.first()
                .map(|f| f.size >= PARTIAL_THRESHOLD)
                .unwrap_or(false)
        })
        .map(|g| g.len())
        .sum();
    if partial_count > 0 {
        tracing::info!("Partial-hashing {} file(s)...", partial_count);
    }
    let partial: Vec<BucketedGroup> = candidates
        .into_par_iter()
        .map(|g| bucket_one_group_partial(g, progress))
        .collect();
    let partial = merge(partial);
    unique.extend(partial.singletons);
    let mut unreadable = partial.unreadable;

    // Phase 3: full hash, dispatched on the strategy.
    let full_count: usize = partial.multi.iter().map(|g| g.len()).sum();
    if full_count > 0 {
        tracing::info!("Full-hashing {} file(s)...", full_count);
    }

    let full_buckets: Vec<BucketedGroup> = match &opts.algo {
        FullHashStrategy::Digest(d) if !d.available() => {
            return Err(ScanError::UnsupportedAlgo(d.name()));
        }
        FullHashStrategy::Digest(d) => {
            let hasher = d.as_ref();
            partial
                .multi
                .into_par_iter()
                .map(|g| bucket_one_group_full(g, hasher, progress))
                .collect()
        }
        FullHashStrategy::Bytewise => partial
            .multi
            .into_par_iter()
            .map(|g| bucket_one_group_bytewise(g, progress))
            .collect(),
    };
    let full = merge(full_buckets);
    unique.extend(full.singletons);
    unreadable.extend(full.unreadable);

    // Establish deterministic order in the result so callers (lib users
    // and renderers alike) see the same output across runs:
    //   - within each duplicate group, entries are pre-sorted by the
    //     original-detection heuristic (canonical first);
    //   - groups themselves are ordered by their canonical entry's path;
    //   - unique and unreadable are ordered by natural path order.
    let mut duplicates: Vec<DuplicateGroup> = full
        .multi
        .into_iter()
        .map(|entries| DuplicateGroup {
            entries: dup_sort(&entries),
        })
        .collect();
    duplicates.sort_by(|a, b| natural_cmp(&a.entries[0].path, &b.entries[0].path));

    unique.sort_by(|a, b| natural_cmp(&a.path, &b.path));
    unreadable.sort_by(|a, b| natural_cmp(&a.path, &b.path));

    Ok(ScanResult {
        unique,
        duplicates,
        unreadable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(path: &str, size: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            aliases: Vec::new(),
            size,
            age: 0.0,
            hash: None,
            dev: 0,
            ino: 0,
        }
    }

    #[test]
    fn split_by_size_isolates_empty_and_singletons() {
        let v = vec![
            entry("e1", 0),
            entry("e2", 0),
            entry("only", 7),
            entry("dup1", 100),
            entry("dup2", 100),
        ];
        let (unique, candidates) = split_by_size(v);
        // 2 empty + 1 singleton = 3 unique
        assert_eq!(unique.len(), 3);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].len(), 2);
        assert_eq!(candidates[0][0].size, 100);
    }

    #[test]
    fn small_groups_passthrough_partial_phase() {
        // size < PARTIAL_THRESHOLD: the whole group should land in `multi`
        // unchanged, no partial hashing attempted.
        let g = vec![entry("a", 100), entry("b", 100)];
        let out = bucket_one_group_partial(g, None);
        assert_eq!(out.multi.len(), 1);
        assert_eq!(out.multi[0].len(), 2);
        assert!(out.singletons.is_empty());
        assert!(out.unreadable.is_empty());
    }

    fn make_real_entry(path: PathBuf, size: u64) -> FileEntry {
        FileEntry {
            path,
            aliases: Vec::new(),
            size,
            age: 0.0,
            hash: None,
            dev: 0,
            ino: 0,
        }
    }

    #[test]
    fn bytewise_groups_identical_files() {
        let td = tempfile::TempDir::new().unwrap();
        let p1 = td.path().join("a");
        let p2 = td.path().join("b");
        let p3 = td.path().join("c");
        std::fs::write(&p1, b"identical content").unwrap();
        std::fs::write(&p2, b"identical content").unwrap();
        std::fs::write(&p3, b"identical content").unwrap();
        let g = vec![
            make_real_entry(p1, 17),
            make_real_entry(p2, 17),
            make_real_entry(p3, 17),
        ];
        let out = bucket_one_group_bytewise(g, None);
        assert_eq!(out.multi.len(), 1);
        assert_eq!(out.multi[0].len(), 3);
        assert!(out.singletons.is_empty());
        assert!(out.unreadable.is_empty());
    }

    #[test]
    fn bytewise_separates_distinct_content() {
        // Same size, different content — should end up as two singletons.
        let td = tempfile::TempDir::new().unwrap();
        let p1 = td.path().join("a");
        let p2 = td.path().join("b");
        std::fs::write(&p1, b"AAAAAA").unwrap();
        std::fs::write(&p2, b"BBBBBB").unwrap();
        let g = vec![make_real_entry(p1, 6), make_real_entry(p2, 6)];
        let out = bucket_one_group_bytewise(g, None);
        assert!(out.multi.is_empty());
        assert_eq!(out.singletons.len(), 2);
    }

    #[test]
    fn bytewise_separates_partial_collision() {
        // Same size, same first byte, same last byte (the partial-hash
        // window), but a different middle byte — must be detected as
        // distinct in the full pass.
        let td = tempfile::TempDir::new().unwrap();
        let p1 = td.path().join("a");
        let p2 = td.path().join("b");
        let mut buf = vec![0u8; 100];
        buf[50] = 1;
        std::fs::write(&p1, &buf).unwrap();
        buf[50] = 2;
        std::fs::write(&p2, &buf).unwrap();
        let g = vec![make_real_entry(p1, 100), make_real_entry(p2, 100)];
        let out = bucket_one_group_bytewise(g, None);
        assert_eq!(out.singletons.len(), 2);
        assert!(out.multi.is_empty());
    }
}

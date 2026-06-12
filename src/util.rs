use std::cmp::Ordering;
use std::path::Path;

use crate::model::{DuplicateGroup, FileEntry, OrderBy, SortGroups};

// Display-only rounding to one decimal — the f64 precision loss clippy
// warns about is far below what the formatting shows anyway.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn human_size(n: u64) -> String {
    let mut x = n as f64;
    for unit in ["B", "KiB", "MiB", "GiB", "TiB"] {
        if x.abs() < 1024.0 {
            return format!("{x:.1} {unit}");
        }
        x /= 1024.0;
    }
    format!("{x:.1} PiB")
}

#[derive(Debug, PartialEq, Eq)]
enum Chunk<'a> {
    Num(u64),
    Text(&'a str),
}

fn natural_chunks(s: &str) -> Vec<Chunk<'_>> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            // Saturate on overflow so absurdly long digit runs still order sensibly.
            let n: u64 = s[start..i].parse().unwrap_or(u64::MAX);
            out.push(Chunk::Num(n));
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Chunk::Text(&s[start..i]));
        }
    }
    out
}

#[must_use]
pub fn natural_cmp(a: &Path, b: &Path) -> Ordering {
    let a_str = a.to_string_lossy();
    let b_str = b.to_string_lossy();
    let a_chunks = natural_chunks(&a_str);
    let b_chunks = natural_chunks(&b_str);
    for (x, y) in a_chunks.iter().zip(b_chunks.iter()) {
        let ord = match (x, y) {
            (Chunk::Num(p), Chunk::Num(q)) => p.cmp(q),
            (Chunk::Text(p), Chunk::Text(q)) => p.to_lowercase().cmp(&q.to_lowercase()),
            // Numeric chunks sort before text chunks at the same position;
            // Python's _natural_key has the same implicit ordering via the
            // mixed-type key tuple.
            (Chunk::Num(_), Chunk::Text(_)) => Ordering::Less,
            (Chunk::Text(_), Chunk::Num(_)) => Ordering::Greater,
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a_chunks.len().cmp(&b_chunks.len())
}

/// Reclaimable bytes for a group: deleting every copy but one frees
/// `(members − 1) × size`. A duplicate group always has at least two members
/// of equal size, so the first member's size represents the whole group.
#[must_use]
pub fn group_reclaimable(group: &DuplicateGroup) -> u64 {
    let size = group.entries.first().map_or(0, |e| e.size);
    (group.entries.len().saturating_sub(1) as u64) * size
}

/// Order the duplicate groups relative to one another. `Path` (default) sorts
/// by the first member's natural path order; `Size` sorts by reclaimable
/// space, largest first, with path order as the deterministic tiebreaker.
pub fn sort_duplicate_groups(groups: &mut [DuplicateGroup], how: SortGroups) {
    match how {
        SortGroups::Path => {
            groups.sort_by(|a, b| natural_cmp(&a.entries[0].path, &b.entries[0].path));
        }
        SortGroups::Size => groups.sort_by(|a, b| {
            group_reclaimable(b)
                .cmp(&group_reclaimable(a))
                .then_with(|| natural_cmp(&a.entries[0].path, &b.entries[0].path))
        }),
    }
}

/// Order a group's members according to `order_by`. The first element of the
/// returned vec is the "kept" entry (the original under `Age`, the
/// earliest-root entry under `Source`); the rest are the prunable copies.
#[must_use]
pub fn order_group(entries: &[FileEntry], order_by: OrderBy) -> Vec<FileEntry> {
    match order_by {
        OrderBy::Age => dup_sort(entries),
        OrderBy::Source => source_sort(entries),
    }
}

/// Secondary, age/root-independent path heuristic, applied once the primary
/// key (age or scan root) ties: fewer path components (shallower) first, then
/// shorter path string, then lexicographic path order as a deterministic
/// tiebreaker. Shared by `dup_sort` and `source_sort` so the "kept" entry is
/// chosen the same way in both — the shallowest, shortest path, which is the
/// intuitive "original" sitting above its tucked-away copies.
fn path_heuristic_cmp(a: &FileEntry, b: &FileEntry) -> Ordering {
    a.path
        .components()
        .count()
        .cmp(&b.path.components().count())
        .then_with(|| a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
        .then_with(|| a.path.cmp(&b.path))
}

/// Original-detection heuristic: oldest first, then the shared path heuristic
/// (shallowest/shortest) as the tiebreaker.
#[must_use]
pub fn dup_sort(entries: &[FileEntry]) -> Vec<FileEntry> {
    let mut v = entries.to_vec();
    v.sort_by(|a, b| {
        a.age
            .partial_cmp(&b.age)
            .unwrap_or(Ordering::Equal)
            .then_with(|| path_heuristic_cmp(a, b))
    });
    v
}

/// Scan-root ordering: by the command-line path argument each file was found
/// under (argument order), then the shared path heuristic. Age is deliberately
/// ignored — what matters is *which tree* a file came from, not when it was
/// created. Within a single root this degenerates to the path heuristic, so
/// the shallowest path is kept as the original (not, say, a deeper one that
/// happens to sort first by digits).
#[must_use]
pub fn source_sort(entries: &[FileEntry]) -> Vec<FileEntry> {
    let mut v = entries.to_vec();
    v.sort_by(|a, b| a.root.cmp(&b.root).then_with(|| path_heuristic_cmp(a, b)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn human_size_unit_jumps() {
        assert_eq!(human_size(0), "0.0 B");
        assert_eq!(human_size(1023), "1023.0 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1024 * 1024), "1.0 MiB");
        assert_eq!(human_size(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }

    #[test]
    fn natural_cmp_orders_numerically() {
        let a = Path::new("file2");
        let b = Path::new("file10");
        assert_eq!(natural_cmp(a, b), Ordering::Less);
    }

    #[test]
    fn natural_cmp_case_insensitive_text() {
        let a = Path::new("Apple");
        let b = Path::new("banana");
        assert_eq!(natural_cmp(a, b), Ordering::Less);
    }

    fn entry(path: &str, age: f64) -> FileEntry {
        entry_rooted(path, age, 0)
    }

    fn entry_rooted(path: &str, age: f64, root: usize) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            aliases: Vec::new(),
            size: 0,
            age,
            hash: None,
            dev: 0,
            ino: 0,
            root,
        }
    }

    #[test]
    fn dup_sort_orders_by_age_first() {
        let v = vec![entry("z", 100.0), entry("a", 50.0)];
        let sorted = dup_sort(&v);
        assert_eq!(sorted[0].path, PathBuf::from("a"));
    }

    #[test]
    fn dup_sort_breaks_ties_by_path_depth() {
        let v = vec![
            entry("a/b/c/x", 100.0),
            entry("a/x", 100.0),
            entry("a/b/x", 100.0),
        ];
        let sorted = dup_sort(&v);
        assert_eq!(sorted[0].path, PathBuf::from("a/x"));
        assert_eq!(sorted[1].path, PathBuf::from("a/b/x"));
        assert_eq!(sorted[2].path, PathBuf::from("a/b/c/x"));
    }

    #[test]
    fn dup_sort_breaks_depth_ties_by_string_length() {
        let v = vec![entry("a/xxxx", 100.0), entry("a/x", 100.0)];
        let sorted = dup_sort(&v);
        assert_eq!(sorted[0].path, PathBuf::from("a/x"));
    }

    #[test]
    fn source_sort_orders_by_root_then_path_ignoring_age() {
        // The root-1 file is older, but `source` ignores age: the root-0 file
        // sorts first because its scan root was listed earlier.
        let v = vec![
            entry_rooted("b/copy", 10.0, 1),
            entry_rooted("a/orig", 99.0, 0),
        ];
        let sorted = source_sort(&v);
        assert_eq!(sorted[0].path, PathBuf::from("a/orig"));
        assert_eq!(sorted[1].path, PathBuf::from("b/copy"));
    }

    #[test]
    fn source_sort_within_one_root_keeps_shallowest_path() {
        // Single root (everything root 0): the shallow file is kept as the
        // original, the one tucked into a subdirectory is the copy — even
        // though the deeper path would sort first under a naive digit-aware
        // comparison (the reported `Downloads/11.12.25 PROD/...` surprise).
        let v = vec![
            entry_rooted("top/2024 backup/report.bos", 0.0, 0),
            entry_rooted("top/report.bos", 0.0, 0),
        ];
        let sorted = source_sort(&v);
        assert_eq!(sorted[0].path, PathBuf::from("top/report.bos"));
        assert_eq!(sorted[1].path, PathBuf::from("top/2024 backup/report.bos"));
    }

    fn sized_group(path: &str, size: u64, copies: usize) -> DuplicateGroup {
        let entries = (0..copies)
            .map(|i| {
                let mut e = entry(&format!("{path}/{i}"), 0.0);
                e.size = size;
                e
            })
            .collect();
        DuplicateGroup { entries }
    }

    #[test]
    fn group_reclaimable_is_copies_times_size() {
        // 3 members of 100 bytes → 2 redundant copies → 200 reclaimable.
        assert_eq!(group_reclaimable(&sized_group("g", 100, 3)), 200);
    }

    #[test]
    fn sort_groups_by_size_orders_by_reclaimable_descending() {
        // small: 1 copy × 1000 = 1000; big: 3 copies × 100 = 300... wait,
        // construct so the larger total wins regardless of per-file size.
        let small_file_many = sized_group("a", 100, 5); // 4 × 100 = 400
        let big_file_few = sized_group("b", 1000, 2); // 1 × 1000 = 1000
        let mut groups = vec![small_file_many, big_file_few];
        sort_duplicate_groups(&mut groups, SortGroups::Size);
        // The 1000-reclaimable group comes first even though it has fewer copies.
        assert_eq!(groups[0].entries[0].size, 1000);
        assert_eq!(groups[1].entries[0].size, 100);
    }

    #[test]
    fn sort_groups_by_size_breaks_ties_by_path() {
        // Equal reclaimable (both 100) → deterministic path order.
        let mut groups = vec![sized_group("zzz", 100, 2), sized_group("aaa", 100, 2)];
        sort_duplicate_groups(&mut groups, SortGroups::Size);
        assert_eq!(groups[0].entries[0].path, PathBuf::from("aaa/0"));
    }

    #[test]
    fn sort_groups_by_path_ignores_size() {
        let mut groups = vec![sized_group("zzz", 9999, 9), sized_group("aaa", 1, 2)];
        sort_duplicate_groups(&mut groups, SortGroups::Path);
        assert_eq!(groups[0].entries[0].path, PathBuf::from("aaa/0"));
    }
}

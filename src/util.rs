use std::cmp::Ordering;
use std::path::Path;

use crate::model::FileEntry;

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

/// Original-detection heuristic: oldest first, then fewer path components,
/// then shorter path string, then lexicographic path order as a deterministic
/// tiebreaker.
pub fn dup_sort(entries: &[FileEntry]) -> Vec<FileEntry> {
    let mut v = entries.to_vec();
    v.sort_by(|a, b| {
        a.age
            .partial_cmp(&b.age)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                a.path
                    .components()
                    .count()
                    .cmp(&b.path.components().count())
            })
            .then_with(|| a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
            .then_with(|| a.path.cmp(&b.path))
    });
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
        FileEntry {
            path: PathBuf::from(path),
            aliases: Vec::new(),
            size: 0,
            age,
            hash: None,
            dev: 0,
            ino: 0,
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
}

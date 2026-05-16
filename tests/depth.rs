mod common;

use std::collections::HashSet;
use std::path::PathBuf;

use tempfile::TempDir;

use common::*;

/// All file paths seen by the scan, regardless of unique/duplicate
/// classification.
fn all_paths(r: &fifi::ScanResult) -> HashSet<PathBuf> {
    let mut out: HashSet<PathBuf> = r.unique.iter().map(|e| e.path.clone()).collect();
    for g in &r.duplicates {
        for e in &g.entries {
            out.insert(e.path.clone());
        }
    }
    for e in &r.unreadable {
        out.insert(e.path.clone());
    }
    out
}

fn tree() -> (TempDir, PathBuf, PathBuf, PathBuf) {
    // root/
    //   a.txt           (depth 0 — directly in root)
    //   sub1/
    //     b.txt         (depth 1)
    //     sub2/
    //       c.txt       (depth 2)
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    let a = mkfile(&root, "a.txt", b"alpha");
    let sub1 = mkdir(&root, "sub1");
    let b = mkfile(&sub1, "b.txt", b"bravo");
    let sub2 = mkdir(&sub1, "sub2");
    let c = mkfile(&sub2, "c.txt", b"charlie");
    (td, a, b, c)
}

#[test]
fn depth_zero_scans_only_direct_files() {
    let (td, a, _b, _c) = tree();
    let r = scan_with(&[td.path()], |o| o.depth = Some(0));
    let paths = all_paths(&r);
    assert_eq!(paths, HashSet::from([a]));
}

#[test]
fn depth_one_includes_one_subdirectory_level() {
    let (td, a, b, _c) = tree();
    let r = scan_with(&[td.path()], |o| o.depth = Some(1));
    let paths = all_paths(&r);
    assert_eq!(paths, HashSet::from([a, b]));
}

#[test]
fn depth_two_includes_two_subdirectory_levels() {
    let (td, a, b, c) = tree();
    let r = scan_with(&[td.path()], |o| o.depth = Some(2));
    let paths = all_paths(&r);
    assert_eq!(paths, HashSet::from([a, b, c]));
}

#[test]
fn depth_unbounded_default_sees_everything() {
    let (td, a, b, c) = tree();
    let r = scan_one(td.path());
    let paths = all_paths(&r);
    assert_eq!(paths, HashSet::from([a, b, c]));
}

#[test]
fn depth_does_not_affect_file_roots() {
    // A file passed directly as a root is always scanned, regardless of
    // depth — depth only governs how deep we descend into directories.
    let td = TempDir::new().unwrap();
    let f = mkfile(td.path(), "lone.txt", b"x");
    let r = scan_with(&[&f], |o| o.depth = Some(0));
    let paths = all_paths(&r);
    assert_eq!(paths, HashSet::from([f]));
}

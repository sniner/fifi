mod common;

use std::path::PathBuf;

use tempfile::TempDir;

use common::*;

#[test]
fn finds_duplicates_across_directories() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    mkfile(&a, "file.txt", b"identical content");
    mkfile(&b, "file.txt", b"identical content");
    mkfile(&a, "other.txt", b"unique");

    let r = scan_one(root);
    assert_eq!(r.duplicates.len(), 1);
    assert_eq!(r.duplicates[0].entries.len(), 2);
    assert_eq!(r.unique.len(), 1);
    assert_eq!(r.unique[0].path.file_name().unwrap(), "other.txt");
}

#[test]
fn empty_files_are_never_duplicates() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "e1", b"");
    mkfile(root, "e2", b"");
    mkfile(root, "e3", b"");

    let r = scan_one(root);
    assert!(r.duplicates.is_empty());
    assert_eq!(r.unique.len(), 3);
}

#[test]
fn hidden_files_ignored_by_default() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "visible.txt", b"hello");
    mkfile(root, ".hidden.txt", b"hello");

    let r = scan_one(root);
    assert!(r.duplicates.is_empty());
    assert_eq!(r.unique.len(), 1);
    assert_eq!(r.unique[0].path.file_name().unwrap(), "visible.txt");
}

#[test]
fn hidden_files_included_when_requested() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "visible.txt", b"hello");
    mkfile(root, ".hidden.txt", b"hello");

    let r = scan_with(&[root], |o| o.include_hidden = true);
    assert_eq!(r.duplicates.len(), 1);
    assert_eq!(r.duplicates[0].entries.len(), 2);
}

#[test]
fn hidden_directories_pruned_with_subtree() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let secret = mkdir(root, ".secret");
    mkfile(root, "open.txt", b"hello");
    mkfile(&secret, "buried.txt", b"hello");

    let r = scan_one(root);
    assert!(r.duplicates.is_empty());
    assert_eq!(r.unique.len(), 1);
}

#[test]
fn symlinks_ignored_by_default() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let target = mkfile(root, "target.txt", b"hello");
    symlink(&target, &root.join("alias.txt"));

    let r = scan_one(root);
    assert!(r.duplicates.is_empty());
    assert_eq!(r.unique.len(), 1);
}

#[test]
fn symlinks_followed_when_requested() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let real_dir = mkdir(root, "real");
    mkfile(&real_dir, "file.txt", b"shared");
    let other_dir = mkdir(root, "other");
    // A symlink that points to real_dir; with --follow the walker descends
    // into it and finds another path to "shared", which is a duplicate.
    symlink(&real_dir, &other_dir.join("real_link"));
    mkfile(&other_dir, "real-copy.txt", b"shared");

    let r = scan_with(&[root], |o| {
        o.follow = true;
        o.per_path = true;
    });
    // Expect at least one duplicate group with shared content.
    let group = r
        .duplicates
        .iter()
        .find(|g| g.entries.iter().any(|e| e.size == b"shared".len() as u64))
        .expect("should find shared content as duplicate");
    assert!(group.entries.len() >= 2);
}

#[test]
fn missing_path_silently_skipped() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "real.txt", b"hello");
    let missing = root.join("does-not-exist");

    let r = fifi::scan(&[root.to_path_buf(), missing], &default_opts()).unwrap();
    assert_eq!(r.unique.len(), 1);
}

#[test]
fn hardlinks_count_as_one_by_default() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkfile(root, "a.txt", b"shared content here");
    hardlink(&a, &root.join("a-link.txt"));
    mkfile(root, "copy.txt", b"shared content here");

    let r = scan_one(root);
    assert_eq!(
        r.duplicates.len(),
        1,
        "expected one duplicate group, got {:?}",
        dup_paths(&r)
    );
    let group = &r.duplicates[0];
    assert_eq!(group.entries.len(), 2);
    let canonical = group
        .entries
        .iter()
        .find(|e| !e.aliases.is_empty())
        .expect("hardlink family should have an alias");
    assert_eq!(canonical.aliases.len(), 1);
}

#[test]
fn per_path_lists_each_hardlink_separately() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkfile(root, "a.txt", b"shared content here");
    hardlink(&a, &root.join("a-link.txt"));
    mkfile(root, "copy.txt", b"shared content here");

    let r = scan_with(&[root], |o| o.per_path = true);
    assert_eq!(r.duplicates.len(), 1);
    assert_eq!(r.duplicates[0].entries.len(), 3);
    assert!(r.duplicates[0].entries.iter().all(|e| e.aliases.is_empty()));
}

#[test]
fn empty_hardlinks_still_unique() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkfile(root, "empty1", b"");
    hardlink(&a, &root.join("empty2"));
    hardlink(&a, &root.join("empty3"));

    let r = scan_one(root);
    assert!(r.duplicates.is_empty());
    assert_eq!(r.unique.len(), 1);
    assert_eq!(r.unique[0].aliases.len(), 2);
}

#[test]
fn single_file_argument_works() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let f = mkfile(root, "lonely.txt", b"hi");
    let r = fifi::scan(std::slice::from_ref(&f), &default_opts()).unwrap();
    assert_eq!(r.unique.len(), 1);
    assert_eq!(r.unique[0].path, PathBuf::from(&f));
}

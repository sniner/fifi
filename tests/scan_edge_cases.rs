mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use tempfile::TempDir;

use common::*;

fn is_root() -> bool {
    // root bypasses chmod 000 read-protection, so unreadable tests need
    // skipping.
    nix::unistd::geteuid().is_root()
}

#[test]
fn deep_directory_tree_does_not_recurse() {
    // Single-letter directory names keep the total path under PATH_MAX
    // even on macOS (1024 bytes), which used to fail with ENAMETOOLONG.
    // The point of this test is the *depth* (proving we don't blow the
    // stack), not the per-segment name length.
    let td = TempDir::new().unwrap();
    let root = td.path();
    let mut cur = root.to_path_buf();
    for _ in 0..200 {
        cur = cur.join("d");
    }
    fs::create_dir_all(&cur).unwrap();
    mkfile(&cur, "deep.txt", b"hello");

    let r = scan_one(root);
    assert_eq!(r.unique.len(), 1);
}

#[test]
fn symlink_cycle_with_follow_terminates() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    mkfile(&a, "x.txt", b"hello");
    // Symlink inside a/ pointing back to a/ — walkdir's follow_links cycle
    // detection should skip the recursive descent without crashing.
    symlink(&a, &a.join("loop"));

    let _r = scan_with(&[root], |o| o.follow = true);
    // Reaching this line means we did not loop infinitely.
}

#[test]
fn unreadable_file_appears_in_unreadable_list() {
    if is_root() {
        eprintln!("Skipping: running as root, chmod 000 has no effect");
        return;
    }
    let td = TempDir::new().unwrap();
    let root = td.path();
    // Two large identical files so the partial-hash phase actually opens
    // them. Below PARTIAL_THRESHOLD they'd go straight to full hashing,
    // which is the same outcome — but exercising both phases is nicer.
    let buf = vec![0xab_u8; 70_000];
    let a = mkfile(root, "a.bin", &buf);
    mkfile(root, "b.bin", &buf);
    fs::set_permissions(&a, fs::Permissions::from_mode(0o000)).unwrap();

    let r = scan_one(root);
    assert!(
        !r.unreadable.is_empty(),
        "expected at least one unreadable entry; got {r:?}"
    );

    // Restore so tempfile can clean up.
    fs::set_permissions(&a, fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn duplicates_still_found_when_one_file_unreadable() {
    if is_root() {
        eprintln!("Skipping: running as root, chmod 000 has no effect");
        return;
    }
    let td = TempDir::new().unwrap();
    let root = td.path();
    let content = b"three identical small files";
    let a = mkfile(root, "a", content);
    mkfile(root, "b", content);
    mkfile(root, "c", content);
    fs::set_permissions(&a, fs::Permissions::from_mode(0o000)).unwrap();

    let r = scan_one(root);
    assert_eq!(r.unreadable.len(), 1);
    assert_eq!(r.duplicates.len(), 1);
    assert_eq!(r.duplicates[0].entries.len(), 2);

    fs::set_permissions(&a, fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn unreadable_directory_is_counted_as_skipped() {
    if is_root() {
        eprintln!("Skipping: running as root, chmod 000 has no effect");
        return;
    }
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "visible", b"x");
    let locked = mkdir(root, "locked");
    mkfile(&locked, "hidden-from-scan", b"y");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

    let r = scan_one(root);
    assert_eq!(r.skipped_dirs, 1, "the locked directory must be counted");
    assert_eq!(r.unique.len(), 1, "only the visible file is scanned");

    // Restore so tempfile can clean up.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn readable_scan_reports_zero_skipped_dirs() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "a", b"x");

    let r = scan_one(root);
    assert_eq!(r.skipped_dirs, 0);
}

#[test]
fn duplicate_roots_are_scanned_once() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "only", b"lonely content");

    // Naming the same root twice must not turn the file into its own
    // duplicate — neither per path nor via a self-alias.
    let r = scan_with(&[root, root], |o| o.per_path = true);
    assert!(r.duplicates.is_empty(), "got {:?}", r.duplicates);
    assert_eq!(r.unique.len(), 1);
    assert!(r.unique[0].aliases.is_empty());
}

#[test]
fn broken_symlink_does_not_crash_when_following() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    symlink(
        std::path::Path::new("/nonexistent/never/here"),
        &root.join("dangling"),
    );
    mkfile(root, "real.txt", b"x");
    let _r = scan_with(&[root], |o| o.follow = true);
}

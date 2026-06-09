mod common;

use tempfile::TempDir;

use common::*;

const LARGE: usize = 128 * 1024; // > PARTIAL_THRESHOLD (64 KiB)

fn make_buf(seed: u8) -> Vec<u8> {
    let mut v = vec![seed; LARGE];
    // A non-uniform tail so the partial hash isn't trivially zero.
    for (i, b) in v.iter_mut().enumerate() {
        *b = u8::try_from(i & 0xff).unwrap() ^ seed;
    }
    v
}

#[test]
fn partial_separates_head_difference() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let mut a = make_buf(0);
    let mut b = a.clone();
    a[0] ^= 0xff;
    mkfile(root, "a.bin", &a);
    mkfile(root, "b.bin", &b);
    let _ = &mut b;

    let r = scan_one(root);
    // Different head → different partial hash → 2 unique, no group.
    assert!(r.duplicates.is_empty());
    assert_eq!(r.unique.len(), 2);
}

#[test]
fn partial_separates_tail_difference() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let mut a = make_buf(0);
    let mut b = a.clone();
    let n = a.len();
    a[n - 1] ^= 0xff;
    mkfile(root, "a.bin", &a);
    mkfile(root, "b.bin", &b);
    let _ = &mut b;

    let r = scan_one(root);
    assert!(r.duplicates.is_empty());
    assert_eq!(r.unique.len(), 2);
}

#[test]
fn full_hash_resolves_partial_collision() {
    // Same head and tail (4 KiB each) but a different middle byte.
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = make_buf(0);
    let mut b = a.clone();
    b[LARGE / 2] ^= 0xff;
    mkfile(root, "a.bin", &a);
    mkfile(root, "b.bin", &b);

    let r = scan_one(root);
    // Partial hash sees them as identical (head + tail match) so the full
    // hash is what splits them. Result: 2 unique, no duplicates.
    assert!(r.duplicates.is_empty());
    assert_eq!(r.unique.len(), 2);
}

#[test]
fn small_files_skip_partial_use_full() {
    // Files < PARTIAL_THRESHOLD bypass phase 2 and go straight to full
    // hashing. Identical small files must end up as a duplicate group.
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "small1", b"hello world");
    mkfile(root, "small2", b"hello world");

    let r = scan_one(root);
    assert_eq!(r.duplicates.len(), 1);
    assert_eq!(r.duplicates[0].entries.len(), 2);
}

#[test]
fn large_identical_files_pass_partial_and_full() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let buf = make_buf(7);
    mkfile(root, "a.bin", &buf);
    mkfile(root, "b.bin", &buf);

    let r = scan_one(root);
    assert_eq!(r.duplicates.len(), 1);
    assert_eq!(r.duplicates[0].entries.len(), 2);
    // Hash should be set after phase 3
    assert!(r.duplicates[0].entries[0].hash.is_some());
}

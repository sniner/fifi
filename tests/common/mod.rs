// Each integration test file compiles `mod common;` independently and only
// exercises a subset of these helpers, so dead-code warnings are noise here.
#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use fifi::{FullHashStrategy, ScanOptions, ScanResult, scan};

pub fn mkfile(parent: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = parent.join(name);
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(contents).unwrap();
    f.sync_all().unwrap();
    path
}

pub fn mkdir(parent: &Path, name: &str) -> PathBuf {
    let p = parent.join(name);
    fs::create_dir_all(&p).unwrap();
    p
}

#[allow(dead_code)]
pub fn symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[allow(dead_code)]
pub fn hardlink(target: &Path, link: &Path) {
    fs::hard_link(target, link).unwrap();
}

pub fn default_opts() -> ScanOptions {
    ScanOptions::new(FullHashStrategy::xxh3())
}

pub fn scan_one(root: &Path) -> ScanResult {
    scan(&[root.to_path_buf()], &default_opts()).unwrap()
}

pub fn scan_with(roots: &[&Path], modify: impl FnOnce(&mut ScanOptions)) -> ScanResult {
    let mut opts = default_opts();
    modify(&mut opts);
    let pb: Vec<PathBuf> = roots.iter().map(|p| p.to_path_buf()).collect();
    scan(&pb, &opts).unwrap()
}

#[allow(dead_code)]
pub fn dup_paths(result: &ScanResult) -> Vec<Vec<PathBuf>> {
    result
        .duplicates
        .iter()
        .map(|g| g.entries.iter().map(|e| e.path.clone()).collect())
        .collect()
}

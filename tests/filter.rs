mod common;

use std::collections::HashSet;
use std::path::PathBuf;

use tempfile::TempDir;

use fifi::{FilterChain, FilterRule, RuleKind};

use common::*;

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

fn chain(rules: &[(RuleKind, &str)]) -> FilterChain {
    let mut c = FilterChain::new();
    for (k, p) in rules {
        c.push(FilterRule::parse(*k, p).unwrap());
    }
    c
}

/// Layout used by most tests below:
///
/// ```text
/// root/
///   movie.mkv
///   photo.jpg
///   notes.txt
///   sub/
///     extra.mkv
///     readme.txt
///   tmp/
///     cache.bin
/// ```
fn fixture() -> (
    TempDir,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
) {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let mkv = mkfile(root, "movie.mkv", b"m");
    let jpg = mkfile(root, "photo.jpg", b"p");
    let txt = mkfile(root, "notes.txt", b"n");
    let sub = mkdir(root, "sub");
    let sub_mkv = mkfile(&sub, "extra.mkv", b"x");
    let sub_txt = mkfile(&sub, "readme.txt", b"r");
    let tmp = mkdir(root, "tmp");
    let cache = mkfile(&tmp, "cache.bin", b"c");
    (td, mkv, jpg, txt, sub_mkv, sub_txt, cache)
}

#[test]
fn empty_filter_keeps_default_behavior() {
    let (td, mkv, jpg, txt, sub_mkv, sub_txt, cache) = fixture();
    let r = scan_with(&[td.path()], |o| o.filter = FilterChain::new());
    assert_eq!(
        all_paths(&r),
        HashSet::from([mkv, jpg, txt, sub_mkv, sub_txt, cache])
    );
}

#[test]
fn basename_mode_keeps_only_mkv_anywhere() {
    // The canonical example from the design discussion. With basename
    // auto-mode, the short form does what users expect.
    let (td, mkv, _jpg, _txt, sub_mkv, _sub_txt, _cache) = fixture();
    let r = scan_with(&[td.path()], |o| {
        o.filter = chain(&[(RuleKind::Exclude, "*"), (RuleKind::Include, "*.mkv")])
    });
    assert_eq!(all_paths(&r), HashSet::from([mkv, sub_mkv]));
}

#[test]
fn directory_only_pattern_prunes_descent_at_any_depth() {
    // `tmp/` is a basename match for any directory named `tmp`,
    // regardless of depth. Pruning means files inside are never
    // visited.
    let (td, mkv, jpg, txt, sub_mkv, sub_txt, _cache) = fixture();
    let r = scan_with(&[td.path()], |o| {
        o.filter = chain(&[(RuleKind::Exclude, "tmp/")])
    });
    assert_eq!(
        all_paths(&r),
        HashSet::from([mkv, jpg, txt, sub_mkv, sub_txt])
    );
}

#[test]
fn anchored_pattern_with_slash_is_root_relative() {
    // Once a pattern contains `/`, the basename auto-mode is off
    // and the pattern is anchored to the scan root. `sub/*.mkv`
    // only matches the mkv inside `sub/`, not the top-level one.
    let (td, mkv, jpg, txt, _sub_mkv, sub_txt, cache) = fixture();
    let r = scan_with(&[td.path()], |o| {
        o.filter = chain(&[(RuleKind::Exclude, "sub/*.mkv")])
    });
    assert_eq!(
        all_paths(&r),
        HashSet::from([mkv, jpg, txt, sub_txt, cache])
    );
}

#[test]
fn directory_include_rescues_subtree() {
    // `--exclude '*' --include 'sub/'` keeps only files inside any
    // directory named `sub`. The rescue propagates through every
    // file underneath via the ancestor walk in FilterChain::decide.
    let (td, _mkv, _jpg, _txt, sub_mkv, sub_txt, _cache) = fixture();
    let r = scan_with(&[td.path()], |o| {
        o.filter = chain(&[(RuleKind::Exclude, "*"), (RuleKind::Include, "sub/")])
    });
    assert_eq!(all_paths(&r), HashSet::from([sub_mkv, sub_txt]));
}

#[test]
fn rescued_subtree_can_still_be_filtered_by_later_rule() {
    // The rescue isn't absolute — a more specific rule after the
    // directory include can still exclude individual files. Here,
    // `sub/` rescues everything in sub, but the later `*.txt` exclude
    // removes the txt file from that rescue.
    let (td, _mkv, _jpg, _txt, sub_mkv, _sub_txt, _cache) = fixture();
    let r = scan_with(&[td.path()], |o| {
        o.filter = chain(&[
            (RuleKind::Exclude, "*"),
            (RuleKind::Include, "sub/"),
            (RuleKind::Exclude, "*.txt"),
        ])
    });
    assert_eq!(all_paths(&r), HashSet::from([sub_mkv]));
}

#[test]
fn last_match_wins_on_overlapping_rules() {
    // `*.txt` would otherwise keep both txt files; the later
    // `readme.txt` exclude removes the readme one (basename match
    // at any depth).
    let (td, mkv, jpg, txt, sub_mkv, _sub_txt, cache) = fixture();
    let r = scan_with(&[td.path()], |o| {
        o.filter = chain(&[
            (RuleKind::Include, "*.txt"),
            (RuleKind::Exclude, "readme.txt"),
        ])
    });
    assert_eq!(
        all_paths(&r),
        HashSet::from([mkv, jpg, txt, sub_mkv, cache])
    );
}

#[test]
fn deeply_nested_excluded_dir_is_not_traversed() {
    // Build a directory tree where the excluded directory contains
    // a deep file. If pruning works, that file never appears in the
    // result; if it doesn't, the file would show up.
    let td = TempDir::new().unwrap();
    let root = td.path();
    let kept = mkfile(root, "keep.txt", b"k");
    let deep = mkdir(root, "deep");
    let mut p = deep.clone();
    for _ in 0..10 {
        p = mkdir(&p, "x");
    }
    mkfile(&p, "buried.txt", b"b");

    let r = scan_with(&[root], |o| {
        o.filter = chain(&[(RuleKind::Exclude, "deep/")])
    });
    assert_eq!(all_paths(&r), HashSet::from([kept]));
}

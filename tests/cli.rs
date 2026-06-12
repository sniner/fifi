mod common;

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

use common::*;

fn fifi() -> Command {
    Command::cargo_bin("fifi").unwrap()
}

#[test]
fn default_text_output_uses_tree_markers() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    mkfile(&a, "x.txt", b"shared");
    mkfile(&b, "x.txt", b"shared");
    mkfile(&a, "lonely.txt", b"unique");

    let out = fifi().arg(root).assert().code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    // Original line starts the group with the root marker, single duplicate
    // (no aliases) closes the group.
    let first_line = stdout.lines().next().unwrap();
    assert!(
        first_line.starts_with("─┬─── "),
        "expected `─┬─── ` on first line:\n{stdout}"
    );
    assert!(
        stdout.contains(" └─── "),
        "expected ` └─── ` for the last inode:\n{stdout}"
    );
}

#[test]
fn dupes_only_emits_nul_terminated_paths() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    let c = mkdir(root, "c");
    mkfile(&a, "x.txt", b"shared");
    let copy_b = mkfile(&b, "x.txt", b"shared");
    let copy_c = mkfile(&c, "x.txt", b"shared");

    let out = fifi()
        .args(["--dupes-only", "--print0"])
        .arg(root)
        .assert()
        .code(1);
    let stdout = out.get_output().stdout.clone();
    // Two copies, each terminated by a NUL — the stream must be directly
    // consumable by `xargs -0`, so no other separators may appear.
    let expected = format!("{}\0{}\0", copy_b.display(), copy_c.display());
    assert_eq!(stdout, expected.as_bytes(), "stdout was {stdout:?}");
}

#[test]
fn json_output_has_expected_shape() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    mkfile(&a, "x.txt", b"shared");
    mkfile(&b, "x.txt", b"shared");

    let out = fifi().arg("--json").arg(root).assert().code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    let dups = json.get("duplicates").unwrap().as_array().unwrap();
    assert_eq!(dups.len(), 1);
    let group = &dups[0];
    let hash = group.get("hash").unwrap().as_str().unwrap();
    assert!(
        hash.starts_with("xxh3:"),
        "expected xxh3: prefix, got {hash}"
    );
    let files = group.get("files").unwrap().as_array().unwrap();
    assert_eq!(files.len(), 2);

    let stats = json.get("statistics").unwrap();
    assert_eq!(stats.get("duplicate_groups").unwrap().as_u64().unwrap(), 1);
    assert_eq!(stats.get("duplicate_copies").unwrap().as_u64().unwrap(), 1);
    assert_eq!(stats.get("skipped_dirs").unwrap().as_u64().unwrap(), 0);
}

#[test]
fn json_unique_flag_switches_subject() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    mkfile(&a, "dup", b"shared");
    mkfile(&b, "dup", b"shared");
    mkfile(root, "solo", b"only-once");

    // Without --unique: the duplicates array is present, no unique array.
    let out = fifi().arg("--json").arg(root).assert().code(1);
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert!(json.get("duplicates").is_some(), "duplicates expected");
    assert!(
        json.get("unique").is_none(),
        "unique must be absent without the flag"
    );

    // With --unique: the subject flips — unique present, duplicates omitted.
    // The statistics block still describes the whole scan.
    let out = fifi()
        .args(["--json", "--unique"])
        .arg(root)
        .assert()
        .code(1);
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert!(
        json.get("duplicates").is_none(),
        "duplicates must be omitted under --unique"
    );
    assert_eq!(json.get("unique").unwrap().as_array().unwrap().len(), 1);
    assert_eq!(
        json["statistics"]["duplicate_groups"].as_u64().unwrap(),
        1,
        "stats still report the duplicate group"
    );
}

#[test]
fn unique_text_lists_only_uniques_not_the_tree() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    let dup_a = mkfile(&a, "dup", b"shared");
    mkfile(&b, "dup", b"shared");
    let solo = mkfile(root, "solo", b"only-once");

    let out = fifi().arg("--unique").arg(root).assert().code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains(solo.display().to_string().as_str()),
        "the unique file must be listed:\n{stdout}"
    );
    assert!(
        !stdout.contains(dup_a.display().to_string().as_str()),
        "duplicate-group files must not appear:\n{stdout}"
    );
    assert!(
        !stdout.contains('┬'),
        "the unique listing has no tree markers:\n{stdout}"
    );
}

#[test]
fn unique_conflicts_with_dupes_only() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "a", b"x");

    fifi()
        .args(["--unique", "--dupes-only"])
        .arg(root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn algo_sha256_finds_duplicates() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "a.txt", b"shared");
    mkfile(root, "b.txt", b"shared");

    let out = fifi()
        .args(["--algo", "sha256", "--json"])
        .arg(root)
        .assert()
        .code(1);
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let dups = json.get("duplicates").unwrap().as_array().unwrap();
    assert_eq!(dups.len(), 1);
    let hash = dups[0].get("hash").unwrap().as_str().unwrap();
    assert!(
        hash.starts_with("sha256:"),
        "expected sha256: prefix, got {hash}"
    );
}

#[test]
fn algo_bytewise_finds_duplicates() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "a.txt", b"shared");
    mkfile(root, "b.txt", b"shared");
    mkfile(root, "different.txt", b"unique");

    let out = fifi()
        .args(["--algo", "bytewise", "--json"])
        .arg(root)
        .assert()
        .code(1);
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let dups = json.get("duplicates").unwrap().as_array().unwrap();
    assert_eq!(dups.len(), 1);
    let files = dups[0].get("files").unwrap().as_array().unwrap();
    assert_eq!(files.len(), 2);
    // bytewise does not compute a digest, so the hash field is null.
    assert!(dups[0].get("hash").unwrap().is_null());
}

#[test]
fn exit_code_one_when_duplicates_found() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "a", b"shared");
    mkfile(root, "b", b"shared");

    fifi().arg(root).assert().failure().code(1);
}

#[test]
fn exit_code_zero_when_no_duplicates() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "a", b"unique-a");
    mkfile(root, "b", b"unique-b");

    fifi().arg(root).assert().success().code(0);
}

#[test]
fn summary_with_json_emits_only_statistics_block() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    mkfile(&a, "x.txt", b"shared");
    mkfile(&b, "x.txt", b"shared");

    let out = fifi()
        .args(["--summary", "--json"])
        .arg(root)
        .assert()
        .code(1);
    let json: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("valid JSON");
    // Only the statistics key, nothing else from the full result.
    assert!(json.get("statistics").is_some());
    assert!(json.get("duplicates").is_none());
    assert!(json.get("scanned_paths").is_none());
    assert!(json.get("unique").is_none());
    let stats = json.get("statistics").unwrap();
    assert_eq!(stats.get("duplicate_groups").unwrap().as_u64().unwrap(), 1);
    assert_eq!(stats.get("duplicate_copies").unwrap().as_u64().unwrap(), 1);
}

#[test]
fn summary_conflicts_with_dupes_only() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "a", b"x");

    fifi()
        .args(["--summary", "--dupes-only"])
        .arg(root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn summary_alone_goes_to_stdout() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "a", b"shared");
    mkfile(root, "b", b"shared");

    let out = fifi().arg("--summary").arg(root).assert().code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stdout.contains("SUMMARY:"),
        "expected SUMMARY on stdout, got stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(!stderr.contains("SUMMARY:"));
}

#[test]
fn summary_reports_reclaimable_bytes() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    // One 6-byte copy is redundant → 6.0 B reclaimable.
    mkfile(root, "a", b"shared");
    mkfile(root, "b", b"shared");

    let out = fifi().arg("--summary").arg(root).assert().code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("(6.0 B reclaimable)"),
        "summary must name the reclaimable bytes:\n{stdout}"
    );
}

#[test]
fn summary_omits_reclaimable_when_no_duplicates() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "a", b"unique-a");
    mkfile(root, "b", b"unique-b");

    let out = fifi().arg("--summary").arg(root).assert().code(0);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        !stdout.contains("reclaimable"),
        "nothing to reclaim, nothing to report:\n{stdout}"
    );
}

#[test]
fn json_stats_count_storage_units_not_paths_in_default_mode() {
    // a.txt + a-link.txt (hardlink, same inode) + copy.txt (separate inode,
    // identical content). Default mode collapses the hardlink family into
    // one canonical entry; statistics should reflect storage usage, not
    // path counts.
    let td = TempDir::new().unwrap();
    let root = td.path();
    let content = b"shared with link";
    let a = mkfile(root, "a.txt", content);
    fs::hard_link(&a, root.join("a-link.txt")).unwrap();
    mkfile(root, "copy.txt", content);

    let out = fifi().arg("--json").arg(root).assert().code(1);
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let stats = json.get("statistics").unwrap();

    // Two distinct inodes (the hardlink family + the separate copy).
    assert_eq!(stats.get("total_files").unwrap().as_u64().unwrap(), 2);
    assert_eq!(stats.get("duplicate_groups").unwrap().as_u64().unwrap(), 1);
    // One reclaimable copy: deleting either the family or the standalone
    // copy frees one inode's worth of space (= content.len() bytes).
    assert_eq!(stats.get("duplicate_copies").unwrap().as_u64().unwrap(), 1);
    let dup_bytes = stats.get("duplicate_bytes").unwrap().as_u64().unwrap();
    assert_eq!(dup_bytes, content.len() as u64);
    let total_bytes = stats.get("total_bytes").unwrap().as_u64().unwrap();
    assert_eq!(total_bytes, 2 * content.len() as u64);
}

#[test]
fn json_stats_count_each_path_under_per_path_mode() {
    // Same setup as above, but --per-path counts every directory entry as
    // its own file. Three paths → three "files".
    let td = TempDir::new().unwrap();
    let root = td.path();
    let content = b"shared with link";
    let a = mkfile(root, "a.txt", content);
    fs::hard_link(&a, root.join("a-link.txt")).unwrap();
    mkfile(root, "copy.txt", content);

    let out = fifi()
        .args(["--json", "--per-path"])
        .arg(root)
        .assert()
        .code(1);
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let stats = json.get("statistics").unwrap();

    assert_eq!(stats.get("total_files").unwrap().as_u64().unwrap(), 3);
    assert_eq!(stats.get("duplicate_copies").unwrap().as_u64().unwrap(), 2);
    assert_eq!(
        stats.get("duplicate_bytes").unwrap().as_u64().unwrap(),
        2 * content.len() as u64
    );
}

#[test]
fn hardlinks_show_alias_in_text_output() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkfile(root, "a.txt", b"shared with link");
    fs::hard_link(&a, root.join("a-link.txt")).unwrap();
    mkfile(root, "copy.txt", b"shared with link");

    let out = fifi().arg(root).assert().code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    // The hardlinked pair becomes the original inode (with a ┬-connector
    // marker because it has an alias), and the alias appears below as the
    // only-and-last alias of a non-last parent.
    assert!(
        stdout.contains("─┬─┬─ "),
        "expected `─┬─┬─ ` for the original inode with an alias:\n{stdout}"
    );
    assert!(
        stdout.contains(" │ └─ "),
        "expected ` │ └─ ` for the hardlink alias:\n{stdout}"
    );
}

#[test]
#[allow(clippy::naive_bytecount)] // no point pulling in bytecount for a test
fn dupes_only_with_three_copies_three_nuls() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    let c = mkdir(root, "c");
    let d = mkdir(root, "d");
    mkfile(&a, "f", b"shared");
    mkfile(&b, "f", b"shared");
    mkfile(&c, "f", b"shared");
    mkfile(&d, "f", b"shared");

    let out = fifi()
        .args(["--dupes-only", "--print0"])
        .arg(root)
        .assert()
        .code(1);
    let stdout = out.get_output().stdout.clone();
    // Four files in one group → 3 copies → one terminating NUL each.
    assert_eq!(
        stdout.iter().filter(|&&b| b == 0).count(),
        3,
        "stdout was {stdout:?}"
    );
    assert!(stdout.ends_with(b"\0"), "stream must end on a NUL");
}

#[test]
fn dupes_only_stream_is_pure_nul_across_groups() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    mkfile(&a, "f1", b"group one");
    let copy1 = mkfile(&b, "f1", b"group one");
    mkfile(&a, "f2", b"group two");
    let copy2 = mkfile(&b, "f2", b"group two");

    let out = fifi()
        .args(["--dupes-only", "--print0"])
        .arg(root)
        .assert()
        .code(1);
    let stdout = out.get_output().stdout.clone();
    // Two groups with one copy each. The group boundary must not introduce
    // any separator besides the per-path NUL (a former newline here glued
    // adjacent paths together under `xargs -0`).
    assert!(!stdout.contains(&b'\n'), "no newlines allowed: {stdout:?}");
    let paths: Vec<&[u8]> = stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(
        paths,
        vec![
            copy1.display().to_string().as_bytes(),
            copy2.display().to_string().as_bytes()
        ]
    );
}

#[test]
fn dupes_only_text_drops_original_keeps_tree() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    let c = mkdir(root, "c");
    // `a` is created first (oldest) and sorts first lexicographically, so it
    // is the original the heuristic picks — and the one `--dupes-only` drops.
    let orig = mkfile(&a, "x.txt", b"shared");
    let copy_b = mkfile(&b, "x.txt", b"shared");
    let copy_c = mkfile(&c, "x.txt", b"shared");

    let out = fifi().arg("--dupes-only").arg(root).assert().code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        !stdout.contains(orig.display().to_string().as_str()),
        "original must be dropped:\n{stdout}"
    );
    assert!(stdout.contains(copy_b.display().to_string().as_str()));
    assert!(stdout.contains(copy_c.display().to_string().as_str()));
    // Still the tree representation, just without the original: the first
    // remaining copy carries the group's left-edge cue.
    assert!(
        stdout.starts_with("─┬─── "),
        "expected tree markers:\n{stdout}"
    );
}

#[test]
fn dupes_only_single_copy_uses_flat_marker() {
    // Dropping the original from a two-file group leaves one lone copy. It
    // must not render with the branching `─┬───` marker, whose `┬` would
    // dangle into empty space — it gets the flat `─────` marker instead.
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    mkfile(&a, "f", b"x");
    let copy = mkfile(&b, "f", b"x");

    let out = fifi().arg("--dupes-only").arg(root).assert().code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(
        stdout,
        format!("───── {}\n", copy.display()),
        "lone copy must use the flat marker:\n{stdout}"
    );
}

#[test]
fn dupes_only_lone_copy_with_hardlink_alias_does_not_dangle() {
    // A single remaining copy that carries a hardlink alias: the inode-level
    // junction must stay flat (`───┬─`), only the alias branch keeps its `┬`.
    let td = TempDir::new().unwrap();
    let root = td.path();
    // `aaa` sorts (and is created) before `zzz`, so the heuristic always picks
    // aaa/file as the original and drops it — leaving the hardlinked zzz copy
    // as the lone survivor. This keeps the test independent of birth-time
    // granularity, which would otherwise let the lexicographic tiebreaker flip
    // which entry survives.
    let orig = mkdir(root, "aaa");
    let copy = mkdir(root, "zzz");
    mkfile(&orig, "file", b"samecontent");
    let copy_file = mkfile(&copy, "file", b"samecontent");
    hardlink(&copy_file, &copy.join("file-link"));

    let out = fifi().arg("--dupes-only").arg(root).assert().code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let first_line = stdout.lines().next().unwrap_or_default();
    assert!(
        first_line.starts_with("───┬─ "),
        "lone copy with alias must use the flat inode junction:\n{stdout}"
    );
    assert!(
        stdout.contains("   └─ "),
        "the hardlink alias must hang from the alias branch:\n{stdout}"
    );
}

#[test]
fn print0_alone_includes_originals() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    mkfile(&a, "x.txt", b"shared");
    mkfile(&b, "x.txt", b"shared");

    let out = fifi().arg("--print0").arg(root).assert().code(1);
    let stdout = out.get_output().stdout.clone();
    // Without --dupes-only the flat list carries every duplicate path,
    // original included → both files of the single group.
    assert!(!stdout.contains(&b'\n'), "no newlines allowed: {stdout:?}");
    let paths: Vec<&[u8]> = stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(paths.len(), 2, "stdout was {stdout:?}");
}

#[test]
fn print0_conflicts_with_json() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "a", b"x");

    fifi()
        .args(["--print0", "--json"])
        .arg(root)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn order_by_source_follows_argument_order() {
    let td = TempDir::new().unwrap();
    let left = mkdir(td.path(), "left");
    let right = mkdir(td.path(), "right");
    let lf = mkfile(&left, "f", b"dup");
    let rf = mkfile(&right, "f", b"dup");

    // `left` listed first → its file is kept (first tree line), `right`'s is
    // the prunable copy.
    let out = fifi()
        .args(["--order-by", "source"])
        .arg(&left)
        .arg(&right)
        .assert()
        .code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let first = stdout.lines().next().unwrap_or_default();
    assert!(
        first.ends_with(lf.display().to_string().as_str()),
        "left should be kept first:\n{stdout}"
    );

    // Swap the argument order. Ages and paths are identical to the run above,
    // so only the scan-root order can flip which file is kept.
    let out = fifi()
        .args(["--order-by", "source"])
        .arg(&right)
        .arg(&left)
        .assert()
        .code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let first = stdout.lines().next().unwrap_or_default();
    assert!(
        first.ends_with(rf.display().to_string().as_str()),
        "right should be kept first after the swap:\n{stdout}"
    );
}

#[test]
fn missing_path_is_an_error() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "real", b"hello");

    // A mistyped path must not masquerade as "scan ok, no duplicates".
    fifi()
        .arg(root)
        .arg(root.join("nonexistent"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("nonexistent"));
}

#[test]
fn missing_path_still_renders_reachable_results() {
    // The reachable root is scanned and rendered; only the exit code and
    // stderr carry the failure.
    let td = TempDir::new().unwrap();
    let root = td.path();
    let a = mkdir(root, "a");
    let b = mkdir(root, "b");
    mkfile(&a, "x.txt", b"shared");
    mkfile(&b, "x.txt", b"shared");

    let out = fifi()
        .arg(root)
        .arg(root.join("nonexistent"))
        .assert()
        .code(2);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("x.txt"),
        "duplicates from the reachable root must still be listed:\n{stdout}"
    );
}

#[test]
fn missing_path_appears_in_json_document() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    mkfile(root, "real", b"hello");
    let missing = root.join("nonexistent");

    let out = fifi()
        .arg("--json")
        .arg(root)
        .arg(&missing)
        .assert()
        .code(2);
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let paths = json.get("missing_paths").unwrap().as_array().unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(
        paths[0].as_str().unwrap(),
        missing.display().to_string().as_str()
    );
}

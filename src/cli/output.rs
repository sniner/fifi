use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use fifi::{FileEntry, ScanResult};

#[derive(Serialize)]
struct JsonFile<'a> {
    path: &'a Path,
    age: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    aliases: Vec<&'a Path>,
}

#[derive(Serialize)]
struct JsonFileFull<'a> {
    path: &'a Path,
    size: u64,
    age: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    aliases: Vec<&'a Path>,
}

#[derive(Serialize)]
struct JsonGroup<'a> {
    hash: Option<String>,
    size: u64,
    files: Vec<JsonFile<'a>>,
}

#[derive(Serialize)]
struct JsonStats {
    total_files: usize,
    total_bytes: u64,
    unique_files: usize,
    duplicate_groups: usize,
    duplicate_copies: usize,
    duplicate_bytes: u64,
    unreadable_files: usize,
    elapsed_seconds: f64,
}

#[derive(Serialize)]
struct JsonResult<'a> {
    scanned_paths: Vec<&'a Path>,
    duplicates: Vec<JsonGroup<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unique: Option<Vec<JsonFileFull<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unreadable: Option<Vec<JsonFileFull<'a>>>,
    statistics: JsonStats,
}

fn make_file<'a>(f: &'a FileEntry) -> JsonFile<'a> {
    JsonFile {
        path: &f.path,
        age: f.age,
        aliases: f.aliases.iter().map(|p| p.as_path()).collect(),
    }
}

fn make_file_full<'a>(f: &'a FileEntry) -> JsonFileFull<'a> {
    JsonFileFull {
        path: &f.path,
        size: f.size,
        age: f.age,
        aliases: f.aliases.iter().map(|p| p.as_path()).collect(),
    }
}

// Statistics count storage units (one per FileEntry = one per inode today,
// one per inode-or-extent in step 2). Aliases are presentational; they don't
// occupy extra disk space and so don't contribute to total_bytes,
// duplicate_bytes, or any *_files count. In --per-path mode aliases are
// always empty, so the formulas degenerate to "one entry per directory path"
// without special-casing.
fn count_entries(entries: &[FileEntry]) -> usize {
    entries.len()
}

fn total_bytes(entries: &[FileEntry]) -> u64 {
    entries.iter().map(|e| e.size).sum()
}

pub struct StatsSnapshot {
    pub elapsed_seconds: f64,
}

fn build_stats(result: &ScanResult, snapshot: &StatsSnapshot) -> JsonStats {
    let dup_copies: usize = result
        .duplicates
        .iter()
        .map(|g| count_entries(&g.entries).saturating_sub(1))
        .sum();
    let dup_bytes: u64 = result
        .duplicates
        .iter()
        .map(|g| {
            let size = g.entries.first().map(|e| e.size).unwrap_or(0);
            (count_entries(&g.entries).saturating_sub(1) as u64) * size
        })
        .sum();
    let total_files = count_entries(&result.unique)
        + result
            .duplicates
            .iter()
            .map(|g| count_entries(&g.entries))
            .sum::<usize>()
        + count_entries(&result.unreadable);
    let total_bytes_all = total_bytes(&result.unique)
        + result
            .duplicates
            .iter()
            .map(|g| total_bytes(&g.entries))
            .sum::<u64>()
        + total_bytes(&result.unreadable);

    JsonStats {
        total_files,
        total_bytes: total_bytes_all,
        unique_files: count_entries(&result.unique),
        duplicate_groups: result.duplicates.len(),
        duplicate_copies: dup_copies,
        duplicate_bytes: dup_bytes,
        unreadable_files: count_entries(&result.unreadable),
        elapsed_seconds: (snapshot.elapsed_seconds * 10_000.0).round() / 10_000.0,
    }
}

#[derive(Serialize)]
struct JsonSummaryOnly {
    statistics: JsonStats,
}

pub fn emit_json_summary<W: Write>(
    mut out: W,
    result: &ScanResult,
    snapshot: StatsSnapshot,
) -> io::Result<()> {
    let payload = JsonSummaryOnly {
        statistics: build_stats(result, &snapshot),
    };
    serde_json::to_writer_pretty(&mut out, &payload).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

pub fn emit_json<W: Write>(
    mut out: W,
    scanned_paths: &[PathBuf],
    result: &ScanResult,
    include_unique: bool,
    algo_name: &str,
    snapshot: StatsSnapshot,
) -> io::Result<()> {
    // ScanResult is already deterministically sorted by the pipeline, so
    // the renderer just walks it.
    let dup_groups: Vec<JsonGroup> = result
        .duplicates
        .iter()
        .map(|g| {
            let first = g.entries.first().expect("duplicate group is never empty");
            JsonGroup {
                hash: first.hash.as_ref().map(|h| format!("{algo_name}:{h}")),
                size: first.size,
                files: g.entries.iter().map(make_file).collect(),
            }
        })
        .collect();

    let unique_block = if include_unique && !result.unique.is_empty() {
        Some(result.unique.iter().map(make_file_full).collect())
    } else {
        None
    };

    let unreadable_block = if !result.unreadable.is_empty() {
        Some(result.unreadable.iter().map(make_file_full).collect())
    } else {
        None
    };

    let json = JsonResult {
        scanned_paths: scanned_paths.iter().map(|p| p.as_path()).collect(),
        duplicates: dup_groups,
        unique: unique_block,
        unreadable: unreadable_block,
        statistics: build_stats(result, &snapshot),
    };

    serde_json::to_writer_pretty(&mut out, &json).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

pub fn emit_json_error(message: &str) {
    let payload = serde_json::json!({ "error": message });
    let _ = serde_json::to_writer_pretty(io::stderr().lock(), &payload);
    let _ = writeln!(io::stderr().lock());
}

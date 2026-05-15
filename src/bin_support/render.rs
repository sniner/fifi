use std::io::{self, Write};

use fifi::{FileEntry, ScanResult};

const BRANCH: &str = "├── ";
const LAST: &str = "└── ";
const ALIAS_BRANCH: &str = "│   = ";
const ALIAS_LAST: &str = "    = ";
const ALIAS_TOP: &str = "    = ";

pub struct RenderOptions {
    pub include_unique: bool,
}

pub fn render_text<W: Write>(
    out: &mut W,
    result: &ScanResult,
    opts: &RenderOptions,
) -> io::Result<()> {
    // ScanResult is already sorted by pipeline::run_pipeline: groups by
    // canonical path, entries within a group canonical-first, unique and
    // unreadable by natural path order.
    if opts.include_unique && !result.unique.is_empty() {
        for f in &result.unique {
            writeln!(out, "{}", f.path.display())?;
            for alias in &f.aliases {
                writeln!(out, "{ALIAS_TOP}{}", alias.display())?;
            }
        }
        writeln!(out)?;
    }

    for group in &result.duplicates {
        let (orig, copies) = group.entries.split_first().expect("group non-empty");
        writeln!(out, "{}", orig.path.display())?;
        for alias in &orig.aliases {
            writeln!(out, "{ALIAS_TOP}{}", alias.display())?;
        }
        let last_idx = copies.len().saturating_sub(1);
        for (i, f) in copies.iter().enumerate() {
            let is_last = i == last_idx;
            let (mark, alias_mark) = if is_last {
                (LAST, ALIAS_LAST)
            } else {
                (BRANCH, ALIAS_BRANCH)
            };
            writeln!(out, "{mark}{}", f.path.display())?;
            for alias in &f.aliases {
                writeln!(out, "{alias_mark}{}", alias.display())?;
            }
        }
    }
    Ok(())
}

pub fn render_dupes_only<W: Write>(out: &mut W, result: &ScanResult) -> io::Result<()> {
    for group in &result.duplicates {
        let copies: Vec<&FileEntry> = group.entries.iter().skip(1).collect();
        if copies.is_empty() {
            // Pathological: a group of size 1 shouldn't exist, but in that
            // case we'd emit nothing for it.
            continue;
        }
        // Each copy contributes its canonical path plus any hardlink aliases.
        let mut paths: Vec<&std::path::Path> = Vec::new();
        for c in &copies {
            paths.push(c.path.as_path());
            for a in &c.aliases {
                paths.push(a.as_path());
            }
        }
        for (i, p) in paths.iter().enumerate() {
            if i > 0 {
                out.write_all(b"\0")?;
            }
            out.write_all(p.to_string_lossy().as_bytes())?;
        }
        out.write_all(b"\n")?;
    }
    Ok(())
}

pub struct SummaryStats {
    pub total_files: usize,
    pub copies: usize,
    pub groups: usize,
    pub unreadable: usize,
    pub elapsed_seconds: f64,
}

fn plural<'a>(n: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if n == 1 { singular } else { plural }
}

pub fn summary_line(stats: &SummaryStats) -> String {
    let mut parts = vec![
        format!(
            "{} {} total,",
            stats.total_files,
            plural(stats.total_files, "file", "files")
        ),
        format!(
            "{} {}",
            stats.copies,
            plural(stats.copies, "duplicate", "duplicates")
        ),
        format!(
            "out of {} {}",
            stats.groups,
            plural(stats.groups, "file", "files")
        ),
    ];
    if stats.unreadable > 0 {
        parts.push(format!(
            "({} {})",
            stats.unreadable,
            plural(stats.unreadable, "unreadable", "unreadable")
        ));
    }
    parts.push(format!("in {:.4}s", stats.elapsed_seconds));
    format!("SUMMARY: {}", parts.join(" "))
}

pub fn collect_summary(result: &ScanResult, elapsed: f64) -> SummaryStats {
    // Counts are storage units (one per FileEntry), not paths. Aliases are
    // presentational only and don't add to the totals — see
    // output::build_stats for the same rule applied to the JSON statistics.
    let unique_files = result.unique.len();
    let dup_files: usize = result.duplicates.iter().map(|g| g.entries.len()).sum();
    let unreadable = result.unreadable.len();
    SummaryStats {
        total_files: unique_files + dup_files + unreadable,
        copies: dup_files - result.duplicates.len(),
        groups: result.duplicates.len(),
        unreadable,
        elapsed_seconds: elapsed,
    }
}

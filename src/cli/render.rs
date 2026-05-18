use std::io::{self, Write};

use fifi::{FileEntry, ScanResult};

// Unique block: standalone files keep a flat layout, aliases marked with `=`.
const UNIQUE_ALIAS: &str = "    = ";

enum InodePos {
    First,
    Middle,
    Last,
}

// Six markers for the inode line, picked by (position among inodes, whether
// hardlink aliases follow). The trailing `┬` makes the vertical to the first
// alias visually continuous; `─` keeps the marker width fixed. The original
// (`First`) starts with `─┬` so each group has a visible left-edge cue.
fn inode_marker(pos: InodePos, has_aliases: bool) -> &'static str {
    match (pos, has_aliases) {
        (InodePos::First, false) => "─┬─── ",
        (InodePos::First, true) => "─┬─┬─ ",
        (InodePos::Middle, false) => " ├─── ",
        (InodePos::Middle, true) => " ├─┬─ ",
        (InodePos::Last, false) => " └─── ",
        (InodePos::Last, true) => " └─┬─ ",
    }
}

// Four markers for an alias line. `parent_last` drops the level-1 `│` once
// the parent inode has no more siblings; `alias_last` closes the level-2
// vertical at the last alias of an inode.
fn alias_marker(parent_last: bool, alias_last: bool) -> &'static str {
    match (parent_last, alias_last) {
        (false, false) => " │ ├─ ",
        (false, true) => " │ └─ ",
        (true, false) => "   ├─ ",
        (true, true) => "   └─ ",
    }
}

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
                writeln!(out, "{UNIQUE_ALIAS}{}", alias.display())?;
            }
        }
        writeln!(out)?;
    }

    for (gi, group) in result.duplicates.iter().enumerate() {
        if gi > 0 {
            writeln!(out)?;
        }
        let (orig, copies) = group.entries.split_first().expect("group non-empty");
        render_inode(out, orig, InodePos::First, false)?;

        let last_copy_idx = copies.len().saturating_sub(1);
        for (i, f) in copies.iter().enumerate() {
            let is_last_copy = i == last_copy_idx;
            let pos = if is_last_copy {
                InodePos::Last
            } else {
                InodePos::Middle
            };
            render_inode(out, f, pos, is_last_copy)?;
        }
    }
    Ok(())
}

fn render_inode<W: Write>(
    out: &mut W,
    entry: &FileEntry,
    pos: InodePos,
    parent_last: bool,
) -> io::Result<()> {
    let has_aliases = !entry.aliases.is_empty();
    writeln!(
        out,
        "{}{}",
        inode_marker(pos, has_aliases),
        entry.path.display()
    )?;
    let last_idx = entry.aliases.len().saturating_sub(1);
    for (i, alias) in entry.aliases.iter().enumerate() {
        let alias_last = i == last_idx;
        writeln!(
            out,
            "{}{}",
            alias_marker(parent_last, alias_last),
            alias.display()
        )?;
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

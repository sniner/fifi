use std::io::{self, Write};

use fifi::{FileEntry, ScanResult};

// Unique block: standalone files keep a flat layout, aliases marked with `=`.
const UNIQUE_ALIAS: &str = "    = ";

enum InodePos {
    First,
    Middle,
    Last,
    /// The only inode in its group — no inode siblings above or below. Arises
    /// when `--dupes-only` drops the original and a single copy remains.
    Only,
}

// Markers for the inode line, picked by (position among inodes, whether
// hardlink aliases follow). The inner `┬` makes the vertical to the first
// alias visually continuous; `─` keeps the marker width fixed. `First` starts
// with `─┬` so each group has a visible left-edge cue *and* a vertical down to
// the next inode. `Only` has no inode below it, so its leading junction stays
// flat (`───`) — only the alias branch keeps its `┬`.
fn inode_marker(pos: InodePos, has_aliases: bool) -> &'static str {
    match (pos, has_aliases) {
        (InodePos::First, false) => "─┬─── ",
        (InodePos::First, true) => "─┬─┬─ ",
        (InodePos::Middle, false) => " ├─── ",
        (InodePos::Middle, true) => " ├─┬─ ",
        (InodePos::Last, false) => " └─── ",
        (InodePos::Last, true) => " └─┬─ ",
        (InodePos::Only, false) => "───── ",
        (InodePos::Only, true) => "───┬─ ",
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

/// Render the duplicate groups as a tree. With `dupes_only`, the original
/// (first member) of each group is dropped, leaving only the copies.
pub fn render_text<W: Write>(out: &mut W, result: &ScanResult, dupes_only: bool) -> io::Result<()> {
    // ScanResult is already sorted by pipeline::run_pipeline: groups by their
    // first entry's path, members ordered per `--order-by`.
    for (gi, group) in result.duplicates.iter().enumerate() {
        if gi > 0 {
            writeln!(out)?;
        }
        // `--dupes-only` hides the original (the first member); a duplicate
        // group always has at least two entries, so the copies slice is
        // never empty.
        let entries = if dupes_only {
            &group.entries[1..]
        } else {
            &group.entries[..]
        };
        render_group(out, entries)?;
    }
    Ok(())
}

/// List the unique files (no content match anywhere), one path per line. A
/// unique inode can still carry hardlink aliases — additional names for the
/// same file — which are listed beneath it, marked with `=`.
pub fn render_unique<W: Write>(out: &mut W, result: &ScanResult) -> io::Result<()> {
    for f in &result.unique {
        writeln!(out, "{}", f.path.display())?;
        for alias in &f.aliases {
            writeln!(out, "{UNIQUE_ALIAS}{}", alias.display())?;
        }
    }
    Ok(())
}

// Render one tree group: the first entry carries the `─┬` left-edge cue, the
// rest descend below it. A lone entry (a single copy left after
// `--dupes-only` dropped the original) uses `Only` instead, so its inode-level
// junction stays flat and nothing dangles into empty space.
fn render_group<W: Write>(out: &mut W, entries: &[FileEntry]) -> io::Result<()> {
    let (first, rest) = entries.split_first().expect("group non-empty");
    if rest.is_empty() {
        // `parent_last` true: no inode sibling follows, so aliases hang with
        // the flush-left indent rather than a continuing vertical.
        return render_inode(out, first, InodePos::Only, true);
    }
    render_inode(out, first, InodePos::First, false)?;

    let last_idx = rest.len() - 1;
    for (i, f) in rest.iter().enumerate() {
        let is_last = i == last_idx;
        let pos = if is_last {
            InodePos::Last
        } else {
            InodePos::Middle
        };
        render_inode(out, f, pos, is_last)?;
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

/// Write each entry's canonical path plus its hardlink aliases as a flat
/// NUL-delimited stream — directly consumable by `xargs -0`. Paths are written
/// as raw bytes, so non-UTF-8 names pass through unmangled. Grouping is not
/// represented in this format; `--json` carries it.
fn emit_nul_paths<'a, W: Write>(
    out: &mut W,
    entries: impl Iterator<Item = &'a FileEntry>,
) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    for e in entries {
        out.write_all(e.path.as_os_str().as_bytes())?;
        out.write_all(b"\0")?;
        for a in &e.aliases {
            out.write_all(a.as_os_str().as_bytes())?;
            out.write_all(b"\0")?;
        }
    }
    Ok(())
}

/// NUL-delimited duplicate paths. With `dupes_only`, the original (first
/// member) of each group is skipped so only the redundant copies are emitted;
/// otherwise every duplicate path is emitted, originals included.
pub fn render_print0<W: Write>(
    out: &mut W,
    result: &ScanResult,
    dupes_only: bool,
) -> io::Result<()> {
    let skip = usize::from(dupes_only);
    emit_nul_paths(
        out,
        result
            .duplicates
            .iter()
            .flat_map(|g| g.entries.iter().skip(skip)),
    )
}

/// NUL-delimited unique paths (no content match anywhere), aliases included.
pub fn render_unique_print0<W: Write>(out: &mut W, result: &ScanResult) -> io::Result<()> {
    emit_nul_paths(out, result.unique.iter())
}

pub struct SummaryStats {
    pub total_files: usize,
    pub copies: usize,
    pub groups: usize,
    pub unreadable: usize,
    pub skipped_dirs: usize,
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
            "across {} {}",
            stats.groups,
            plural(stats.groups, "group", "groups")
        ),
    ];
    if stats.unreadable > 0 {
        parts.push(format!("({} unreadable)", stats.unreadable));
    }
    if stats.skipped_dirs > 0 {
        parts.push(format!(
            "({} {} skipped)",
            stats.skipped_dirs,
            plural(stats.skipped_dirs, "dir", "dirs")
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
        skipped_dirs: result.skipped_dirs,
        elapsed_seconds: elapsed,
    }
}

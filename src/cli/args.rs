use std::path::PathBuf;

use clap::{ArgAction, Parser, ValueEnum};

use fifi::{FullHashStrategy, OrderBy, SortGroups};

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum AlgoArg {
    Xxh3,
    Sha256,
    Bytewise,
}

impl AlgoArg {
    pub fn into_strategy(self) -> FullHashStrategy {
        match self {
            AlgoArg::Xxh3 => FullHashStrategy::xxh3(),
            AlgoArg::Sha256 => FullHashStrategy::sha256(),
            AlgoArg::Bytewise => FullHashStrategy::Bytewise,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum OrderByArg {
    Age,
    Source,
}

impl OrderByArg {
    pub fn into_order_by(self) -> OrderBy {
        match self {
            OrderByArg::Age => OrderBy::Age,
            OrderByArg::Source => OrderBy::Source,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum SortGroupsArg {
    Path,
    Size,
}

impl SortGroupsArg {
    pub fn into_sort_groups(self) -> SortGroups {
        match self {
            SortGroupsArg::Path => SortGroups::Path,
            SortGroupsArg::Size => SortGroups::Size,
        }
    }
}

/// Parse a human-friendly size into bytes. A bare number is bytes; the
/// suffixes K/M/G/T/P are powers of 1024 (binary), matching the KiB/MiB/…
/// units fifi prints. An optional `i` and/or trailing `B` is accepted and
/// makes no difference: `4M`, `4MB`, and `4MiB` all mean 4 × 1024². A
/// decimal fraction is allowed (`1.5G`). Comparisons are inclusive on both
/// ends, so `--min-size 1` excludes only empty files.
//
// The float casts are intrinsic to accepting fractional sizes; the value is
// bounds-checked finite and non-negative, and the result saturates at
// u64::MAX, so the precision/sign/truncation lints don't apply.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty size".to_string());
    }
    // The numeric prefix runs up to the first byte that is neither a digit
    // nor a decimal point; the remainder is the unit.
    let split = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let mult: u64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        "t" | "tb" | "tib" => 1 << 40,
        "p" | "pb" | "pib" => 1 << 50,
        other => {
            return Err(format!(
                "unknown size unit '{other}' (use B, K, M, G, T, or P)"
            ));
        }
    };
    // A bare byte count parses as an integer so large exact values survive;
    // a fractional value (only sensible with a unit) goes through f64.
    if mult == 1 && !num.contains('.') {
        return num
            .parse::<u64>()
            .map_err(|_| format!("invalid size '{s}'"));
    }
    let value: f64 = num.parse().map_err(|_| format!("invalid size '{s}'"))?;
    if value < 0.0 || !value.is_finite() {
        return Err(format!("invalid size '{s}'"));
    }
    // value is finite and non-negative; saturate rather than wrap on an
    // absurd request.
    let bytes = (value * mult as f64).round();
    if bytes >= u64::MAX as f64 {
        Ok(u64::MAX)
    } else {
        Ok(bytes as u64)
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "fifi",
    version,
    about = "Find identical files in subdirectories."
)]
// Output is two orthogonal axes. The *format* is the tree-style text
// (default), `--print0` (a flat NUL-delimited path list), `--json`, or
// `--summary`; these are mutually exclusive — except `--json --summary`,
// which means "stats-only JSON". `--dupes-only` is a *selection* modifier
// that drops the original from each group; it composes with the text and
// `--print0` formats. `--print0`/`--dupes-only` make no sense with `--json`
// or `--summary`, so we conflict them there (see the `conflicts_with_all`
// on each).
// One bool per independent CLI flag — that's what a flag struct is.
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
    /// Paths to scan.
    #[arg(required = true)]
    pub path: Vec<PathBuf>,

    /// Follow symlinks (ignored by default).
    #[arg(long)]
    pub follow: bool,

    /// Include hidden files and directories (ignored by default).
    #[arg(long)]
    pub hidden: bool,

    /// Do not enter mounted file systems.
    #[arg(long = "one-file-system")]
    pub one_file_system: bool,

    /// Limit subdirectory descent below each root
    ///
    /// `--depth 0` scans only the files directly in the named directories
    /// (no recursion). `--depth N` allows N levels of subdirectory descent.
    /// Omitted means unbounded.
    #[arg(long, value_name = "N")]
    pub depth: Option<usize>,

    /// Ignore files smaller than SIZE
    ///
    /// SIZE is a byte count with an optional binary unit: `512`, `64K`,
    /// `10M`, `1.5G` (K/M/G/T/P are powers of 1024). The bound is inclusive,
    /// so `--min-size 1` keeps everything except empty files. Filtering
    /// happens during the walk, so excluded files cost no hashing.
    #[arg(long = "min-size", value_name = "SIZE", value_parser = parse_size)]
    pub min_size: Option<u64>,

    /// Ignore files larger than SIZE
    ///
    /// Same SIZE syntax as `--min-size`; the bound is inclusive. Combine the
    /// two to scan a size window, e.g. `--min-size 1M --max-size 100M`.
    #[arg(long = "max-size", value_name = "SIZE", value_parser = parse_size)]
    pub max_size: Option<u64>,

    /// List unique files instead of duplicates
    ///
    /// Switches the subject of the output: instead of the duplicate groups,
    /// fifi lists the files whose content has no match anywhere across the
    /// scanned paths. Works with the text, `-0`/`--print0`, and `--json`
    /// formats. Mutually exclusive with `--dupes-only`, which selects the
    /// opposite (the redundant copies). Empty files are always listed as
    /// unique — they are never considered identical to each other. The exit
    /// code follows the listing: 1 when unique files were found, 0 when
    /// there are none. `--summary` is subject-independent, so combining it
    /// with `--unique` simply ignores the flag (output and exit code alike).
    #[arg(long, conflicts_with = "dupes_only")]
    pub unique: bool,

    /// Count one entry per path; don't merge hardlink families
    ///
    /// Without this flag, hardlinks (files sharing dev+ino) collapse into
    /// one canonical entry with the other paths as aliases — useful for
    /// the "where am I wasting disk space?" question. With `--per-path`,
    /// every path on disk is a standalone entry — useful for the "which
    /// directory entries hold the same content?" question.
    #[arg(long = "per-path")]
    pub per_path: bool,

    /// Full-content hash algorithm
    ///
    /// `xxh3` (default) — non-cryptographic, runs at memory bandwidth.
    /// `sha256` — cryptographic, slower but with audit-trail value.
    /// `bytewise` — no hashing; files are compared pairwise byte-for-byte.
    /// The partial-hash pass (head + tail of files ≥ 64 KiB) is always
    /// xxh3 regardless of this setting; only the full pass is configurable.
    #[arg(long, value_enum, default_value_t = AlgoArg::Xxh3)]
    pub algo: AlgoArg,

    /// Order within each duplicate group
    ///
    /// `age` (default) lists the oldest file first — the presumed "original",
    /// the one `--dupes-only` keeps. `source` ignores age and orders members
    /// by the scan root (the command-line path argument) they were found
    /// under, in argument order, then by path (shallowest first). For
    /// `fifi orig backup` that keeps the `orig` copy and lists the `backup`
    /// one as the prunable duplicate.
    #[arg(long = "order-by", value_enum, default_value_t = OrderByArg::Age)]
    pub order_by: OrderByArg,

    /// Order the duplicate groups relative to one another
    ///
    /// `path` (default) sorts groups by their first member's path. `size`
    /// sorts by reclaimable space — `copies × size` — largest first, so the
    /// biggest wins for a cleanup come first. This orders the groups; use
    /// `--order-by` to order the members within a group.
    #[arg(long = "sort-groups", value_enum, default_value_t = SortGroupsArg::Path)]
    pub sort_groups: SortGroupsArg,

    /// Increase verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = ArgAction::Count)]
    pub verbose: u8,

    /// Suppress all but error output.
    #[arg(short, long, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Emit results as JSON on stdout, including a statistics block.
    #[arg(long)]
    pub json: bool,

    /// Drop the original from each group; print only the redundant copies
    ///
    /// A selection modifier, not a format: it removes the first member (the
    /// "original") of every duplicate group, leaving just the copies in the
    /// usual tree layout. Combine with `-0`/`--print0` for a flat
    /// NUL-delimited list — the canonical `… | xargs -0 rm` form.
    #[arg(long = "dupes-only", conflicts_with_all = ["json", "summary"])]
    pub dupes_only: bool,

    /// Emit a flat, NUL-delimited path list instead of the tree
    ///
    /// A pure format flag (the name mirrors `find -print0`): every path is
    /// terminated by a NUL byte, safe for `xargs -0`. On its own it lists
    /// every duplicate path including the originals; pair it with
    /// `--dupes-only` to list only the copies.
    #[arg(long = "print0", short = '0', conflicts_with_all = ["json", "summary"])]
    pub print0: bool,

    /// Print only the summary line
    ///
    /// Combine with `--json` to emit a stats-only JSON document
    /// (`{"statistics": {...}}`) instead of the one-line text summary.
    #[arg(long)]
    pub summary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Text,
    Print0,
    Json,
    Summary,
    SummaryJson,
}

impl Cli {
    pub fn output_mode(&self) -> OutputMode {
        // `--print0` conflicts with both `--json` and `--summary`, so the
        // `(false, true, _)` arm can only fire with both of those false.
        match (self.json, self.print0, self.summary) {
            (true, _, true) => OutputMode::SummaryJson,
            (true, _, false) => OutputMode::Json,
            (false, true, _) => OutputMode::Print0,
            (false, false, true) => OutputMode::Summary,
            (false, false, false) => OutputMode::Text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn parses_bare_bytes() {
        assert_eq!(parse_size("0"), Ok(0));
        assert_eq!(parse_size("512"), Ok(512));
    }

    #[test]
    fn binary_units_are_powers_of_1024() {
        assert_eq!(parse_size("1K"), Ok(1024));
        assert_eq!(parse_size("1M"), Ok(1024 * 1024));
        assert_eq!(parse_size("2G"), Ok(2 * 1024 * 1024 * 1024));
    }

    #[test]
    fn unit_spelling_variants_are_equivalent() {
        // bare letter, +B, and +iB all mean the binary unit.
        assert_eq!(parse_size("4M"), parse_size("4MB"));
        assert_eq!(parse_size("4M"), parse_size("4MiB"));
        assert_eq!(parse_size("4M"), parse_size("4mib"));
    }

    #[test]
    fn accepts_fractional_with_unit() {
        assert_eq!(parse_size("1.5K"), Ok(1536));
    }

    #[test]
    fn bare_byte_count_keeps_full_precision() {
        // Beyond f64's exact-integer range — must not go through f64.
        let big = (1u64 << 53) + 1;
        assert_eq!(parse_size(&big.to_string()), Ok(big));
    }

    #[test]
    fn rejects_garbage_and_unknown_units() {
        assert!(parse_size("").is_err());
        assert!(parse_size("abc").is_err());
        assert!(parse_size("10X").is_err());
        assert!(parse_size("1.2.3K").is_err());
    }
}

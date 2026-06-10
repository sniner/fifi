use std::path::PathBuf;

use clap::{ArgAction, Parser, ValueEnum};

use fifi::{FullHashStrategy, OrderBy};

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

    /// Also include unique files in the output.
    #[arg(long)]
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

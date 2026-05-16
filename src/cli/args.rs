use std::path::PathBuf;

use clap::{ArgAction, ArgGroup, Parser, ValueEnum};

use fifi::FullHashStrategy;

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

#[derive(Parser, Debug)]
#[command(
    name = "fifi",
    version,
    about = "Find identical files in subdirectories."
)]
// `--json` and `--dupes-only` are the two primary output formats — pick
// one or neither (default is the tree-style text). `--summary` is a scope
// modifier: alone it shrinks the text output to a single line; combined
// with `--json` it shrinks the JSON output to just the statistics block.
// `--summary` with `--dupes-only` makes no sense, so we conflict them.
#[command(group(
    ArgGroup::new("primary_output")
        .args(["json", "dupes_only"])
        .multiple(false)
))]
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

    /// Increase verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = ArgAction::Count)]
    pub verbose: u8,

    /// Suppress all but error output.
    #[arg(short, long, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Emit results as JSON on stdout, including a statistics block.
    #[arg(long)]
    pub json: bool,

    /// Print only duplicate copies (originals excluded), NUL-delimited.
    #[arg(long = "dupes-only")]
    pub dupes_only: bool,

    /// Print only the summary line
    ///
    /// Combine with `--json` to emit a stats-only JSON document
    /// (`{"statistics": {...}}`) instead of the one-line text summary.
    #[arg(long, conflicts_with = "dupes_only")]
    pub summary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Text,
    Json,
    DupesOnly,
    Summary,
    SummaryJson,
}

impl Cli {
    pub fn output_mode(&self) -> OutputMode {
        match (self.json, self.dupes_only, self.summary) {
            (true, _, true) => OutputMode::SummaryJson,
            (true, _, false) => OutputMode::Json,
            (false, true, _) => OutputMode::DupesOnly,
            (false, false, true) => OutputMode::Summary,
            (false, false, false) => OutputMode::Text,
        }
    }
}

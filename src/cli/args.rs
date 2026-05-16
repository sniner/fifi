use std::path::PathBuf;

use clap::{ArgAction, ArgGroup, ArgMatches, Parser, ValueEnum};

use fifi::{FilterChain, FilterParseError, FilterRule, FullHashStrategy, RuleKind};

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

    /// Exclude paths matching the given glob (repeatable)
    ///
    /// Patterns without `/` match against the basename at any depth
    /// (`*.log` excludes every log file in the tree). Patterns with `/`
    /// are anchored to the scan root (`src/*.log` only at the top-level
    /// `src/`). A trailing `/` makes a pattern directory-only and
    /// prunes descent. Combine with `--include`; later flags override
    /// earlier ones, so order on the command line is significant.
    #[arg(long = "exclude", action = ArgAction::Append, value_name = "PATTERN")]
    pub exclude: Vec<String>,

    /// Include paths matching the given glob (repeatable)
    ///
    /// Same pattern syntax as `--exclude`. If `--include` is the first
    /// filter flag on the command line, an implicit `--exclude '*'` is
    /// prepended so that `fifi --include '*.mkv' /path` narrows the
    /// scan to mkv files (rather than being a no-op against the default
    /// of including everything). A directory-only include also rescues
    /// every file inside that directory from a prior broad exclude,
    /// unless a later more-specific rule overrides it.
    #[arg(long = "include", action = ArgAction::Append, value_name = "PATTERN")]
    pub include: Vec<String>,

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

/// Build the filter chain from raw clap matches, preserving the
/// original argv order between `--include` and `--exclude` flags.
///
/// `Vec<String>` on the derive struct gives us the *values* in order
/// per flag, but loses the interleaving between the two flag types.
/// We recover that by asking `ArgMatches` for each value's original
/// argv position, then merging both flag streams by index.
///
/// Convenience: if the first user-supplied rule is an `--include`, an
/// implicit `--exclude '*'` is prepended. Without this, a lone include
/// would be a no-op against fifi's default of including everything,
/// and `fifi --include '*.mkv' /path` would happily return every file
/// instead of just the mkv ones. Starting with an `--exclude` opts out
/// of this convenience (the user has stated their intent explicitly).
pub fn build_filter(matches: &ArgMatches) -> Result<FilterChain, FilterParseError> {
    let mut entries: Vec<(usize, RuleKind, &str)> = Vec::new();
    for (idx, val) in matches
        .indices_of("exclude")
        .into_iter()
        .flatten()
        .zip(matches.get_many::<String>("exclude").into_iter().flatten())
    {
        entries.push((idx, RuleKind::Exclude, val.as_str()));
    }
    for (idx, val) in matches
        .indices_of("include")
        .into_iter()
        .flatten()
        .zip(matches.get_many::<String>("include").into_iter().flatten())
    {
        entries.push((idx, RuleKind::Include, val.as_str()));
    }
    entries.sort_by_key(|(idx, _, _)| *idx);

    let mut chain = FilterChain::new();
    if matches!(entries.first(), Some((_, RuleKind::Include, _))) {
        chain.push(FilterRule::parse(RuleKind::Exclude, "*")?);
    }
    for (_, kind, pat) in entries {
        chain.push(FilterRule::parse(kind, pat)?);
    }
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use fifi::Decision;
    use std::path::Path;

    fn build(args: &[&str]) -> FilterChain {
        let argv = std::iter::once("fifi")
            .chain(args.iter().copied())
            .chain(std::iter::once("/dummy"));
        let matches = Cli::command()
            .try_get_matches_from(argv)
            .expect("argv is well-formed");
        build_filter(&matches).expect("patterns are valid")
    }

    fn included(d: Decision) -> bool {
        matches!(d, Decision::Include)
    }

    #[test]
    fn no_filter_args_produces_empty_chain() {
        let c = build(&[]);
        assert!(c.is_empty());
    }

    #[test]
    fn lone_include_narrows_via_implicit_exclude() {
        let c = build(&["--include", "*.mkv"]);
        // The implicit `--exclude '*'` makes this narrow rather than
        // be a no-op: only mkv files are kept, at any depth.
        assert!(included(c.decide(Path::new("movie.mkv"), false)));
        assert!(included(c.decide(Path::new("sub/movie.mkv"), false)));
        assert!(!included(c.decide(Path::new("notes.txt"), false)));
        assert!(!included(c.decide(Path::new("sub/notes.txt"), false)));
    }

    #[test]
    fn lone_directory_include_rescues_subtree() {
        let c = build(&["--include", "photos/"]);
        assert!(included(c.decide(Path::new("photos/img.jpg"), false)));
        assert!(included(c.decide(Path::new("photos/sub/img.jpg"), false)));
        assert!(!included(c.decide(Path::new("other/img.jpg"), false)));
    }

    #[test]
    fn leading_exclude_opts_out_of_implicit_convention() {
        // First flag is `--exclude`, so the user's intent is "remove
        // specific things, keep the rest". The later `--include` here
        // is effectively a no-op against the default-include semantics,
        // which is the same behavior as before this convenience landed.
        let c = build(&["--exclude", "*.log", "--include", "*.mkv"]);
        assert!(included(c.decide(Path::new("movie.mkv"), false)));
        assert!(included(c.decide(Path::new("photo.jpg"), false)));
        assert!(!included(c.decide(Path::new("app.log"), false)));
    }

    #[test]
    fn include_first_then_exclude_combines() {
        // `--include '*.mkv'` gives implicit `--exclude '*'`, then the
        // explicit `--exclude 'sample.mkv'` removes one specific mkv.
        let c = build(&["--include", "*.mkv", "--exclude", "sample.mkv"]);
        assert!(included(c.decide(Path::new("movie.mkv"), false)));
        assert!(included(c.decide(Path::new("sub/movie.mkv"), false)));
        assert!(!included(c.decide(Path::new("sample.mkv"), false)));
        assert!(!included(c.decide(Path::new("sub/sample.mkv"), false)));
        assert!(!included(c.decide(Path::new("notes.txt"), false)));
    }
}

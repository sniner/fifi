//! Path-based include/exclude filtering for the directory walk.
//!
//! Pattern syntax mixes "natural file-name matching" with "explicit
//! path-relative matching", picked by whether the pattern contains a
//! `/`:
//!
//! - **No `/` in the body** → the pattern matches against basename
//!   anywhere in the tree (gitignore-style). `*.py` matches every
//!   Python file, regardless of depth. `cache` matches anything named
//!   `cache`. This is the common case.
//! - **`/` in the body** → the pattern is anchored to the scan root
//!   and matched against the full relative path. `src/*.log` only
//!   matches `*.log` directly under `src/` at the top level, not
//!   under `nested/src/`. `*` and `?` never cross `/`; use `**` for
//!   recursion.
//! - **Trailing `/`** opts into directory-only. The trailing slash is
//!   stripped before the rest of the rules apply, so `cache/` is a
//!   directory-only basename match (any dir named `cache` anywhere),
//!   and `node_modules/` likewise.
//!
//! When a directory matches a rule (include or exclude), that decision
//! propagates down to descendants: an include on `photos/` rescues
//! everything inside `photos/` from a prior `--exclude '*'`, unless a
//! later rule overrides for specific paths.
//!
//! Order matters: rules are evaluated sequentially, and the **last**
//! matching rule decides the outcome. With no matching rule, the
//! default is to include.
//!
//! There is intentionally no `.gitignore` file handling. The pattern
//! syntax here is inspired by gitignore for the common bits (basename
//! mode, trailing-`/` directory marker) but is otherwise independent
//! and does not pretend to be compatible with it.

use std::fmt;
use std::path::Path;

use globset::{Glob, GlobBuilder, GlobMatcher};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    Include,
    Exclude,
}

#[derive(Debug, Clone, Copy)]
pub enum Decision {
    Include,
    Exclude,
}

#[derive(Debug)]
pub enum ParseError {
    Empty,
    Glob(globset::Error),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => f.write_str("empty pattern"),
            ParseError::Glob(e) => write!(f, "invalid glob pattern: {e}"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ParseError::Glob(e) => Some(e),
            ParseError::Empty => None,
        }
    }
}

impl From<globset::Error> for ParseError {
    fn from(e: globset::Error) -> Self {
        ParseError::Glob(e)
    }
}

#[derive(Debug, Clone)]
pub struct FilterRule {
    kind: RuleKind,
    matcher: GlobMatcher,
    /// True when the original pattern ended with `/`. Such rules only
    /// match directories — never plain files.
    dir_only: bool,
    /// The original pattern, kept for error messages and debug logging.
    raw: String,
}

impl FilterRule {
    pub fn parse(kind: RuleKind, pattern: &str) -> Result<Self, ParseError> {
        if pattern.is_empty() {
            return Err(ParseError::Empty);
        }
        let raw = pattern.to_string();

        // Strip exactly one trailing `/` to opt into directory-only mode.
        let (body, dir_only) = match pattern.strip_suffix('/') {
            Some(b) => (b, true),
            None => (pattern, false),
        };
        if body.is_empty() {
            return Err(ParseError::Empty);
        }

        // Basename auto-mode: a pattern that contains no `/` in its
        // body matches at any depth, against the basename of the
        // candidate (or any ancestor for dir-only rules). We implement
        // that by lifting it under `**/`. Patterns that already contain
        // `/` stay anchored to the scan root.
        let glob_pattern = if body.contains('/') {
            body.to_string()
        } else {
            format!("**/{body}")
        };

        let glob: Glob = GlobBuilder::new(&glob_pattern)
            // `*` and `?` never cross `/`. Recursion is opt-in via `**`.
            .literal_separator(true)
            .build()?;

        Ok(Self {
            kind,
            matcher: glob.compile_matcher(),
            dir_only,
            raw,
        })
    }

    pub fn kind(&self) -> RuleKind {
        self.kind
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Returns true when this rule fires for the given candidate.
    ///
    /// File-style rules (no trailing `/`) apply only to files;
    /// directory-only rules apply only to directories. This split
    /// avoids the trap where a broad `--exclude '*'` would also prune
    /// every directory and prevent us from ever reaching a file that a
    /// later `--include` rule wants to keep.
    fn applies_to(&self, rel_path: &Path, is_dir: bool) -> bool {
        if self.dir_only != is_dir {
            return false;
        }
        self.matcher.is_match(rel_path)
    }
}

#[derive(Debug, Clone, Default)]
pub struct FilterChain {
    rules: Vec<FilterRule>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, rule: FilterRule) {
        self.rules.push(rule);
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Last-match-wins evaluation. Returns `Include` if no rule matches.
    ///
    /// For a file candidate, the decision also takes ancestor directories
    /// into account: a directory-only rule (include or exclude) that
    /// matches some ancestor of the file propagates down. That is what
    /// makes `--include 'subdir/'` rescue every file inside `subdir/`
    /// from a prior broad exclude.
    pub fn decide(&self, rel_path: &Path, is_dir: bool) -> Decision {
        let mut decision = Decision::Include;
        for rule in &self.rules {
            if Self::rule_fires(rule, rel_path, is_dir) {
                decision = match rule.kind {
                    RuleKind::Include => Decision::Include,
                    RuleKind::Exclude => Decision::Exclude,
                };
            }
        }
        decision
    }

    /// A rule fires either when it directly matches the candidate, or
    /// — for a file candidate — when it matches some ancestor directory.
    /// Directory candidates don't trigger ancestor walking: they're only
    /// looked at directly, because their decision is what drives descent
    /// pruning, and we don't want a deeply-buried dir-only rule to
    /// retroactively prune ancestor directories.
    fn rule_fires(rule: &FilterRule, rel_path: &Path, is_dir: bool) -> bool {
        if rule.applies_to(rel_path, is_dir) {
            return true;
        }
        if is_dir {
            return false;
        }
        let mut p = rel_path.parent();
        while let Some(parent) = p {
            if parent.as_os_str().is_empty() {
                break;
            }
            if rule.applies_to(parent, true) {
                return true;
            }
            p = parent.parent();
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn rule(kind: RuleKind, pat: &str) -> FilterRule {
        FilterRule::parse(kind, pat).unwrap()
    }

    fn chain(rules: &[(RuleKind, &str)]) -> FilterChain {
        let mut c = FilterChain::new();
        for (k, p) in rules {
            c.push(rule(*k, p));
        }
        c
    }

    fn decide(c: &FilterChain, path: &str, is_dir: bool) -> Decision {
        c.decide(&PathBuf::from(path), is_dir)
    }

    fn included(d: Decision) -> bool {
        matches!(d, Decision::Include)
    }

    #[test]
    fn empty_chain_includes_everything() {
        let c = FilterChain::new();
        assert!(included(decide(&c, "foo.txt", false)));
        assert!(included(decide(&c, "any/dir", true)));
    }

    #[test]
    fn pattern_without_slash_matches_basename_at_any_depth() {
        // `*.log` matches every .log file regardless of depth — the
        // common-case basename semantics.
        let c = chain(&[(RuleKind::Exclude, "*.log")]);
        assert!(!included(decide(&c, "app.log", false)));
        assert!(!included(decide(&c, "src/app.log", false)));
        assert!(!included(decide(&c, "a/b/c/app.log", false)));
        assert!(included(decide(&c, "app.txt", false)));
    }

    #[test]
    fn pattern_with_slash_is_anchored_to_scan_root() {
        // The presence of a `/` switches off the basename auto-mode.
        // `src/*.log` only matches under the top-level `src/`.
        let c = chain(&[(RuleKind::Exclude, "src/*.log")]);
        assert!(!included(decide(&c, "src/app.log", false)));
        assert!(included(decide(&c, "nested/src/app.log", false)));
        assert!(included(decide(&c, "app.log", false)));
    }

    #[test]
    fn double_star_explicitly_matches_recursively() {
        // Equivalent to basename mode for this case, but explicit.
        let c = chain(&[(RuleKind::Exclude, "**/*.log")]);
        assert!(!included(decide(&c, "app.log", false)));
        assert!(!included(decide(&c, "a/b/c/app.log", false)));
        assert!(included(decide(&c, "app.txt", false)));
    }

    #[test]
    fn directory_only_pattern_does_not_match_files() {
        // `tmp/` is a directory-only basename match. It matches dirs
        // named `tmp` at any depth, but never a regular file.
        let c = chain(&[(RuleKind::Exclude, "tmp/")]);
        assert!(!included(decide(&c, "tmp", true)));
        assert!(!included(decide(&c, "a/tmp", true)));
        assert!(!included(decide(&c, "a/b/tmp", true)));
        // A regular file named "tmp" is untouched.
        assert!(included(decide(&c, "tmp", false)));
    }

    #[test]
    fn file_pattern_does_not_apply_to_directories() {
        // Without this guard, a broad `--exclude '*'` would prune
        // every directory and we'd never reach a file that a later
        // `--include` wants to keep.
        let c = chain(&[(RuleKind::Exclude, "*")]);
        assert!(!included(decide(&c, "foo.txt", false)));
        assert!(!included(decide(&c, "a/foo.txt", false)));
        assert!(included(decide(&c, "any/subdir", true)));
    }

    #[test]
    fn last_match_wins() {
        // The canonical "only mkv files anywhere" example, in the
        // short basename form rather than `**/*` gymnastics.
        let c = chain(&[(RuleKind::Exclude, "*"), (RuleKind::Include, "*.mkv")]);
        assert!(included(decide(&c, "movie.mkv", false)));
        assert!(included(decide(&c, "sub/movie.mkv", false)));
        assert!(!included(decide(&c, "movie.avi", false)));
        assert!(!included(decide(&c, "sub/movie.avi", false)));
    }

    #[test]
    fn later_exclude_overrides_earlier_include() {
        let c = chain(&[
            (RuleKind::Include, "*.txt"),
            (RuleKind::Exclude, "secret.txt"),
        ]);
        assert!(!included(decide(&c, "secret.txt", false)));
        assert!(!included(decide(&c, "any/secret.txt", false)));
        assert!(included(decide(&c, "ok.txt", false)));
    }

    #[test]
    fn directory_include_rescues_descendant_files() {
        // The "rescue" case: a broad `--exclude '*'` excludes every
        // file, but a later directory-only include propagates down to
        // descendants via the ancestor walk inside FilterChain::decide.
        let c = chain(&[(RuleKind::Exclude, "*"), (RuleKind::Include, "photos/")]);
        // Files inside photos/, at any depth, are rescued.
        assert!(included(decide(&c, "photos/img.jpg", false)));
        assert!(included(decide(&c, "photos/sub/img.jpg", false)));
        // Files outside photos/ stay excluded.
        assert!(!included(decide(&c, "other/img.jpg", false)));
        assert!(!included(decide(&c, "img.jpg", false)));
    }

    #[test]
    fn later_specific_exclude_overrides_directory_rescue() {
        // Rescue is not unconditional: a more-specific rule that
        // comes after the directory include wins.
        let c = chain(&[
            (RuleKind::Exclude, "*"),
            (RuleKind::Include, "photos/"),
            (RuleKind::Exclude, "*.tmp"),
        ]);
        assert!(included(decide(&c, "photos/img.jpg", false)));
        assert!(!included(decide(&c, "photos/scratch.tmp", false)));
    }

    #[test]
    fn empty_pattern_is_rejected() {
        assert!(matches!(
            FilterRule::parse(RuleKind::Exclude, ""),
            Err(ParseError::Empty)
        ));
        assert!(matches!(
            FilterRule::parse(RuleKind::Exclude, "/"),
            Err(ParseError::Empty)
        ));
    }

    #[test]
    fn invalid_glob_is_rejected() {
        assert!(matches!(
            FilterRule::parse(RuleKind::Exclude, "foo[abc"),
            Err(ParseError::Glob(_))
        ));
    }
}

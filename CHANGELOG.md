# Changelog

Format based on [Keep a Changelog](https://keepachangelog.com).

## [Unreleased]

### Fixed

- **`--dupes-only`** output is now strictly NUL-delimited, as documented: every
  path is terminated by a NUL byte, making the stream safe for `xargs -0`.
  Previously a newline separated duplicate groups, which glued the last path of
  one group to the first path of the next under `xargs -0`. Group boundaries are
  no longer represented in this format — use `--json` if you need the grouping
- **`--dupes-only`** writes paths as raw bytes; non-UTF-8 file names are no
  longer mangled by lossy conversion
- **`--algo bytewise`** no longer misattributes unreadable files: when the file
  other entries were compared against could not be read, the readable entries
  were reported as unreadable while the unreadable file itself was reported as
  unique. Now exactly the file that failed to read lands in the unreadable list

### Removed

- **Library API**: `ScanError::Io`, `util::path_sort`, and `util::path_sort_paths`
  were never used by fifi and have been dropped

## [0.4.0] — 2026-05-23

### Breaking changes

- **Exit codes** swap meaning: `0` now means "no duplicates" (the clean,
  nothing-to-act-on state) and `1` means "duplicates were found". `2` for
  errors is unchanged. This matches the linter convention used by tools like
  `clippy` and `shellcheck`, and avoids the surprise of `set -e` scripts
  tripping on a successful scan that simply found nothing. If you scripted
  the previous behaviour, swap the branches: `fifi /backup && handle_dupes`
  becomes `fifi /backup || handle_dupes`.

## [0.3.2] — 2026-05-18

### Changed

- **Text output** now separates duplicate groups with a blank line. Easier to
  skim when many groups share a screen; JSON and `--dupes-only` output are
  unchanged.

## [0.3.1] — 2026-05-17

### Changed

- **Text output** now renders each duplicate group as a two-level tree. The
  first line (`─┬─── ` / `─┬─┬─ `) marks the original and starts a new group;
  further inodes use ` ├─── ` / ` └─── `. Hardlink aliases sit on level two
  (` │ └─ ` / `   └─ `) under their inode. Inodes with aliases end their
  marker in `┬` so the vertical to the first alias is continuous. All paths
  start in the same column, making them easier to compare by eye. JSON and
  `--dupes-only` output are unchanged.

## [0.3.0] — 2026-05-16

Adds a depth limit for directory descent. `--depth 0` matches the
no-recursion case some tools spell as the absence of `-r`; `--depth N`
allows N levels of subdirectory descent; omitted, the scan remains
unbounded as before.

### Added

- **`--depth N`** caps how far the scan descends below each named root.
  `--depth 0` scans only the files directly in the named directories
  (no recursion), `--depth 1` adds one level of subdirectories, and so
  on. Omitted, the scan remains unbounded as before. A file passed
  directly as a root is always included regardless of this setting.

## [0.2.0] — 2026-05-15

Both placeholder full-hash strategies from 0.1.0 are now implemented:
`--algo sha256` for cryptographic guarantees, `--algo bytewise` for
literal byte-for-byte comparison. The pipeline architecture didn't have
to change — these slot into the existing `FullHashStrategy` dispatch.

### Added

- **`--algo sha256`** now actually computes SHA-256 over the file content
  (was previously a placeholder that errored with "not yet implemented").
  Use it when you want cryptographic collision resistance for audit
  trails or in regulated environments where xxh3 is hard to justify.
- **`--algo bytewise`** compares files pairwise byte-for-byte instead of
  hashing. Slowest of the three options, but a literal rather than
  probabilistic guarantee: if fifi says two files are identical under
  `--algo bytewise`, every byte was compared and matched. Intended for
  paranoia-grade verification.

### Changed

- Error messages no longer carry the redundant `scan failed:` prefix.
  The underlying `ScanError` variants already self-describe (e.g.
  "I/O error on /path: ..." or "hash algorithm not yet implemented: X").

## [0.1.0] — 2026-05-15

Initial release. `fifi` is a Rust reimplementation of the Python `duplicates`
tool with parallel hashing, storage-aware deduplication, tree-style text
output, and a pluggable hash strategy that's ready to grow.

### Added

- **Three-phase duplicate detection** — group by size, partial hash (head + tail
  of files ≥ 64 KiB), full hash. Same algorithm as the Python original.
- **`xxh3` (xxh3-128) as the default hash**, both for the partial pass and the
  full pass. Drop-in replacement for SHA-256 with substantially higher
  throughput.
- **Pluggable full-hash algorithm** via `--algo {xxh3,sha256,bytewise}`. Only
  `xxh3` is implemented in this release; `sha256` and `bytewise` are reserved
  and exit with a clear "not yet implemented" error.
- **Parallel hashing** via rayon. Both partial and full hash phases process
  candidate groups in parallel.
- **Storage-aware deduplication** — files sharing `(dev, inode)` (i.e.
  hardlinks) collapse into one canonical entry with the other paths
  exposed as `aliases`. This answers the *"where am I wasting disk
  space?"* question accurately. Statistics
  (`total_files`, `total_bytes`, `duplicate_copies`, `duplicate_bytes`, ...)
  count storage units (one per inode), not directory paths — `duplicate_bytes`
  is the actually reclaimable amount of disk space.
- **Deterministic output order** — duplicate groups are emitted in natural
  order of their canonical path; entries within a group come canonical-first
  via the original-detection heuristic; `unique` and `unreadable` lists are
  sorted in natural path order. Output is stable across runs, suitable for
  `diff` and reproducible scripts.
- **Synthetic-inode safety net** — hardlink merging requires matching size,
  not just matching `(dev, ino)`. Filesystems that fake inode numbers
  (SMB/CIFS, WebDAV) sometimes report the same inode for unrelated files;
  this guard prevents fifi from silently treating them as the same file.
  When a mismatch is detected, the entries are kept separate and a warning
  is logged. Real hardlinks share their size by construction, so they
  continue to merge as expected.
- **`--per-path`** flips into per-directory-entry mode: every path on disk
  becomes a standalone entry, useful for the *"which directory entries hold
  the same content?"* question regardless of storage sharing.
- **Tree-style text output** — duplicate groups render with `├── ` / `└── `
  markers, hardlink aliases with `    = ` under their canonical entry.
- **`--dupes-only`** prints only the duplicate copies (originals excluded),
  NUL-delimited within each group, newline between groups.
- **`--json`** emits a structured result on stdout including a statistics
  block. Hash field carries a dynamic algorithm prefix (`xxh3:<hex>`); hardlink
  aliases nest under their canonical entry via an optional `aliases` field.
- **`--summary`** prints a one-line summary on stdout (when used alone) or
  stderr (when triggered alongside `-v`). Combine with `--json` to emit a
  stats-only JSON document (`{"statistics": {...}}`).
- **Verbosity flags** `-v` (info), `-vv` (debug), `-vvv` (trace), `-q` (errors
  only), respecting the `RUST_LOG` env var.
- **Original-detection heuristic** — within a duplicate group, the canonical
  ("original") file is chosen by `(age, path component count, path string
  length, lexicographic path)`. Older + shallower + shorter wins, with a
  deterministic final tiebreaker. "Age" uses the inode's birth time when
  the filesystem provides one (statx on Linux 4.11+ for ext4/btrfs/xfs/f2fs,
  st_birthtime on macOS); on filesystems without a birth time it falls
  back to `min(mtime, ctime)` so neither a chmod-bumped ctime nor a
  forward-dated mtime can fake a file into looking newer or older.
- **Grep-style exit codes** — `0` if duplicates were found, `1` if none, `2`
  on error. Lets you write `fifi -q /backup && echo HAS_DUPS`.

### Notes

- Library API is exposed via `fifi::scan(paths, &opts)` returning a `ScanResult`.
  Stable enough to use, but expect the surface to evolve before 1.0.
- POSIX-only (Linux, macOS). MS Windows is not supported.

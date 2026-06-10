# fifi

Find identical files in subdirectories. Fast, parallel, hardlink-aware.

`fi` for *file*, twice — because we're looking for duplicates, hence `fifi`.

## Why?

Plenty of duplicate finders exist (`fdupes`, `jdupes`, `rdfind` and others)
and they all work. `fifi` came out of wanting a handful of specific things
together:

- **xxh3-128** for hashing by default — duplicate detection doesn't need
  collision resistance against an adversary, and xxh3 runs at memory
  bandwidth. `--algo sha256` is available if you want cryptographic
  guarantees (audit logs, regulated environments); `--algo bytewise`
  skips hashing entirely and compares files pairwise byte-for-byte
  (slowest, but a literal rather than probabilistic guarantee).
- **Parallel hashing** via rayon — large filesystems use all your cores.
- **A clear semantic split** between *"where am I wasting disk space?"* and
  *"which directory entries hold the same content?"*. The default answers
  the first: hardlinks collapse into one entry with the other paths as
  aliases. Pass `--per-path` to answer the second instead.
- **Tree-style text output** that's easy to skim, with hardlink aliases
  rendered alongside their canonical entry rather than as a separate copy.
- **JSON output** with a statistics block, ready to pipe into `jq`.

## Usage

```
fifi [OPTIONS] PATH [PATH...]
```

### Examples

Default scan:

```
$ fifi /photos
─┬─── /photos/2023/scan.pdf
 ├─┬─ /photos/2024/scan.pdf
 │ └─ /photos/2024/scan.pdf.bak
 └─── /photos/archive/scan.pdf

─┬─── /photos/2024/IMG_3000.jpg
 └─── /photos/import/IMG_3000.jpg
```

Each group is a two-level tree. The first line (`─┬─── `) marks the original
inode and starts a new group; further inodes (`├─── ` / `└─── `) are the
duplicate copies. Level 2 (`│ └─ ` / `  └─ `) lists hardlink aliases under
their inode: additional paths that point to the same storage, not separate
duplicates. An inode with aliases ends its marker in `┬` instead of `─` so
the vertical to the first alias is continuous.

JSON for scripting (`duplicate_bytes` is the reclaimable amount of disk
space — bytes that would actually be freed by removing the duplicate copies):

```
$ fifi --json /photos | jq '.statistics.duplicate_bytes'
12856293
```

NUL-delimited copies for `xargs` deletion (be careful) — `--dupes-only` drops
the original from each group, `--print0` emits the flat NUL-delimited list:

```
$ fifi --dupes-only --print0 /photos | tr '\0' '\n' | head
```

### Options

| Option | Description |
|--------|-------------|
| `--follow` | Follow symlinks (ignored by default) |
| `--hidden` | Include hidden files and directories (ignored by default) |
| `--one-file-system` | Do not enter mounted file systems |
| `--depth N` | Limit subdirectory descent (`0` = no recursion, unbounded by default) |
| `--unique` | Also include unique files in the output |
| `--per-path` | Count one entry per path; don't merge files that share storage |
| `--algo ALGO` | Full-hash algorithm: `xxh3` (default), `sha256`, `bytewise` |
| `--json` | Emit results as JSON on stdout, with statistics |
| `--dupes-only` | Drop the original from each group; print only the redundant copies |
| `--print0`, `-0` | Emit a flat, NUL-delimited path list (for `xargs -0`) instead of the tree |
| `--summary` | Print only the final summary line |
| `-v`, `-vv`, `-vvv` | Increase log verbosity (info / debug / trace) |
| `-q`, `--quiet` | Suppress all but error output |

### Exit codes

- `0` — no duplicates (the scan ran cleanly and there is nothing to act on)
- `1` — duplicates were found
- `2` — an error occurred

This lets you write `fifi -q /backup || echo "duplicates found"` without parsing
output, the same way linters like `clippy` or `shellcheck` signal "something to
look at".

## How it works

Three phases:

1. **Group by size.** Files alone in their size bucket can't have duplicates.
   Empty files are always treated as unique.
2. **Partial hash** (only for files ≥ 64 KiB). xxh3 over the first and last
   4 KiB. Cheap prefilter against near-duplicate large files (videos, archives).
3. **Full hash** for whatever the partial pass couldn't separate. Uses the
   algorithm chosen via `--algo`.

Hardlinks (files sharing the same `(dev, ino)`) are collapsed into a
single entry before phase 1. Identical inodes are never re-hashed and
never counted as separate files. This is the right answer to the
**"where am I wasting disk space?"** question.

If you want the **"which directory entries hold the same content?"**
question instead — for example because you want to find every path that
points to the same data, regardless of whether it occupies extra space —
pass `--per-path`. Each path on disk then becomes a standalone entry, and
hardlinks are listed alongside true content copies.

The "original" within a duplicate group is the oldest file, breaking ties
by fewer path components, then shorter path string, then lexicographic
order. Deterministic and reasonable for real directories where the
canonical copy tends to live closer to the root. The duplicate groups
themselves come out in natural order of their canonical path, so output is
stable across runs and friendly to `diff` and `grep`.

"Oldest" is determined by the inode's birth time when the filesystem
provides one (via `statx()` on Linux ≥ 4.11, or `st_birthtime` on macOS —
supported by ext4, btrfs, xfs, f2fs, and APFS). POSIX doesn't mandate a
creation time, so older filesystems fall back to `min(mtime, ctime)`:
ctime advances on every inode-touch (chmod, chown, link-count change)
and mtime can be set arbitrarily by `touch -d`, so the smaller of the two
picks the older signal in either case.

### Network filesystems

Storage-aware mode relies on `(dev, ino)` from `stat()` to identify files
that share storage. This is reliable on local filesystems, NFS, and SSHFS
(the server's real inodes are passed through). It is **not reliable** on
SMB/CIFS or WebDAV, where inode numbers are often synthesized by the kernel
or by `davfs2` and the same fake inode can be reported for unrelated files.

`fifi` defends against the worst case (silently merging two distinct files
because their inodes happen to collide) by also requiring that members of a
`(dev, ino)` family share the same size. Genuine hardlinks always do, by
construction, so they're still merged correctly. When a size mismatch
appears, fifi keeps those entries separate and logs a warning.

If you don't trust the inodes on a network share, pass `--per-path` to
skip storage-aware deduplication entirely and treat every directory entry
as a standalone file.

## Roadmap

- **gitignore-style excludes** — `--exclude-from` with proper glob support.
- **Progress bars** via indicatif when stdout is a terminal.

## Installation

### From crates.io

```
cargo install fifi
```

### From source

```
cargo install --path .
```

### Pre-built binary

Download from the [releases page](https://github.com/sniner/fifi/releases).

## Requirements

- POSIX (Linux, macOS). MS Windows is not supported.
- Rust 1.85+ if building from source.

## License

GPL-3.0-or-later — see [LICENSE](LICENSE).

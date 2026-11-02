# Archive Utility

Faithful to macOS Archive Utility: opening an archive expands it next to itself (one top-level
item lands as itself, several land in a folder named after the archive, names that are taken get
" 2"), a progress window with Stop shows only for work that takes over half a second, encrypted
zip and 7-Zip archives ask for a password, and Files' Compress makes `Name.zip` (or `Archive.zip`).
Settings choose where expanded items and archives go, the archive format, whether the original
moves to the Trash and whether the result is revealed in Files (`FileManager1.ShowItems`).
Opening a file that is not an archive compresses it, as on macOS.

## One engine, in the desktop

The archive engine is `bande-fs` (`ops/archive.rs`, `ops/archive/sink.rs` in bande-linux/desktop),
the same code Files' `Task::Compress` and `Task::Extract` run, so Files and Archive Utility never
diverge. Files hands Compress to Archive Utility (`--compress`) and opens archives in the default
handler (`bande-mimeapps.list` maps the archive types to it); when Archive Utility is not
installed, Files compresses to zip and expands archives itself with the same engine.

## Libraries

| Format | Library | Why |
|---|---|---|
| zip read/write, ZipCrypto | `zip` 9 (already in the desktop) | pure Rust, maintained, `enclosed_name` and raw entries for the checks below. AES zip needs its `aes-crypto` feature, which wants `zeroize` 1.9 while both workspaces pin 1.8.1; macOS Archive Utility cannot open AES zips either |
| tar | `tar` 0.4 (already in the desktop) | used only to read and write headers; entries are written by our own sink, never by `unpack_in`, which keeps setuid when asked to keep permissions |
| gzip, bzip2, xz, zstd | `flate2`, `bzip2`, `lzma-rust2` 0.23, `ruzstd` (already in the desktop) | pure Rust decoders; xz and bzip2 also write. `lzma-rust2` moved from 0.24 to 0.23 to share one copy with `sevenz-rust2` |
| 7-Zip read, AES-256 | `sevenz-rust2` 0.24 (`aes256`, `bzip2`, `deflate`, `ppmd`) | pure Rust, 2 M downloads, 3 `unsafe` blocks, maintained with `lzma-rust2`; replaces Files' old call to an external `7z`/`bsdtar` |
| rar | none | `unrar` binds RARLAB's non-free source (Debian ships it in non-free); `rars` is six months old, 117 k lines with `unsafe`; libarchive reads rar but only through C. macOS Archive Utility does not open rar either |

Creating offers zip (the default), tar.gz, tar.bz2 and tar.xz; `ruzstd`'s encoder returns no
errors and panics on write failures, so tar.zst is read-only.

## Security

Every entry goes through `Sink`, which holds the staging folder open and creates each path
component with `mkdirat` and opens it with `O_NOFOLLOW | O_DIRECTORY`, so nothing is written
through a link. Entries with `..`, a root or a prefix are refused. A symbolic link is created, and
before the staging folder moves into place every link is resolved with `openat2(RESOLVE_BENEATH)`
(a dangling one lexically); one that leaves the folder refuses the archive. Hard links name an
entry already inside. Files and folders keep `rwx` bits only (`& 0o777`): setuid, setgid and
sticky never survive; folders get their modes at the end, with the owner's `rwx`.

Bombs: what comes out may not exceed the free space of the destination, nor a thousand times the
archive's size once past 64 MiB (deflate tops out near 1032:1, so only overlapping zip entries
and bzip2, xz or zstd bombs reach it). Zip and 7-Zip are checked against their declared sizes
before anything is written, every format against what is actually written. A refusal is an
`io::Error` carrying `bande_fs::Refusal`, whose text is the alert's message; the staging folder
is removed, so a refused archive leaves nothing.

## Gaps

- rar, AES zip, zstd writing (above); split and self-extracting archives; `.cpgz`/`.cpio`.
- No "Keep expanding if possible" (a `.tar` inside a `.zip` stays a `.tar`).
- The progress window lists work but has no per-item names or time left.

# Lighthouse — Layer Browser

The control plane can browse the filesystem inside an image layer without
pulling it, extracting it, or running it. This document describes how the
browser selects a decompressor, walks the tar stream, merges overrides, and
contains the safety and resource guards.

Implementation: `src/api/layers.rs`. The raw OCI surface (manifests, blobs) is
part of the OCI module; this document covers the `/api` browsing endpoints.

---

## 1. Endpoints

| Method | Path | Description |
|---|---|---|
| GET | `/api/repositories/{ns}/{*repo}/layers/{digest}/tree?path=` | One directory level of the layer. |
| GET | `/api/repositories/{ns}/{*repo}/layers/{digest}/file?path=` | A single file's bytes (`Content-Type` guessed). |
| GET | `/api/repositories/{ns}/{*repo}/layers/{digest}/download` | The raw, still-compressed layer archive. |
| GET | `/api/repositories/{ns}/{*repo}/manifests/{digest}` | Raw manifest / index JSON. |
| GET | `/api/repositories/{ns}/{*repo}/manifests/{digest}/references` | The manifest's referenced descriptors. |
| GET | `/api/blobs/{digest}` | Raw blob bytes. |
| GET | `/api/blobs/{digest}/json` | A blob parsed as JSON (config blobs). |

`tree` returns `[{ name, path, kind: "file"|"dir"|"symlink", size, mode,
link_target, link_resolved, link_kind }]`; `mode` is the raw tar mode (e.g. `420`
for `0644`). `size` is the uncompressed byte count for a file or symlink, and for
a directory the recursive total of every sized descendant, computed once when the
index is built (§6). For a symlink, `link_resolved` is the normalized layer path
the link points at — following symlink chains, with `.`/`..` folded and `..` never
escaping the layer root — and `link_kind` is the resolved entry's kind (`file` or
`dir`); both are `null` when the link is dangling, cyclic, or escapes the layer,
and are `null` for every non-symlink entry. `file` returns the entry bytes with a
MIME type guessed from the path via `mime_guess` (falling back to
`application/octet-stream`). `download` streams the original archive unchanged,
with a `Content-Disposition: attachment` header whose filename is
`<algorithm>-<encoded>.<ext>` — the layer digest with `:` replaced by `-`, plus an
extension derived from the media type (`tar.gz` for gzip, `tar.zst` for zstd,
`tar` for a plain tar, and `bin` when the media type is not a tar).

---

## 2. Decompression selection

A layer's compression is detected from the archive's leading magic bytes, not
from its declared media type:

| Leading magic bytes | Decompressor |
|---|---|
| `28 B5 2F FD` (zstd) | `zstd::stream::read::Decoder` |
| `1F 8B` (gzip) | `flate2::read::GzDecoder` |
| anything else | none (plain `tar`) |

A zstd SKIPPABLE frame prefix (`50`–`5F 2A 4D 18`) is also recognized as zstd.
The blob is content-addressed and its digest was verified over the raw bytes on
push, so the content is authoritative; the stored media type is advisory only.
It is still used to pick the `download` filename extension (§1) and to document
what a client intended, but browsing no longer depends on it. A wrong or missing
media type therefore no longer breaks browsing: a plain tar declared as zstd (or
vice versa) is detected correctly, and if the bytes are not a valid tar the read
fails cleanly with a `500` rather than leaking a panic.

---

## 3. How a request is served

1. Resolve and authorize the repository with `permissions::repository_access`;
   a caller without pull access gets `404` (never `403`).
2. Parse the digest, confirm the blob exists and is linked to that repository.
3. Open the blob, convert the `tokio` file to a `std` file, and hand the work to
   `tokio::task::spawn_blocking` so decompression and tar parsing never block the
   async runtime.
4. Read the archive **in memory** through a `CappedReader`; nothing is ever
   written to disk and no `Archive::unpack` is used.

### Merging, overrides and whiteouts

`tree` scans the whole archive once (on a cache miss) and folds entries into a
map keyed by normalized path, applying OCI layer rules in stream order:

- A repeated path **overrides** the earlier entry (later wins).
- `.wh.<name>` removes `<name>` from its directory (deleted in a lower layer).
- `.wh..wh..opq` marks its directory opaque: every previously seen entry directly
  inside it is removed.

`tree` then projects the merged map onto the requested directory, synthesizing
`dir` entries for prefixes that never appeared as explicit directory headers.
`file` reads the stream and returns the first matching regular file, **stopping
as soon as the target is found** so it never decompresses the rest of a large
layer.

GNU long-name (`L`) headers are honoured for the path of the following entry.
PAX extended headers (`x`/`g`) and GNU long-link headers are skipped; PAX `path=`
overrides are not applied. This keeps the reader simple and is sufficient for
ordinary container layers.

---

## 4. Safety rules

Every requested and archive path is sanitized before use:

- **Absolute paths** (`/etc/passwd`) are rejected with `400`.
- **Parent traversal** (`../`, `a/../../b`) is rejected with `400`.
- **NUL bytes** in a path are rejected; paths longer than 4096 bytes are rejected.
- Empty and `.` components are normalized away.
- Archive entries whose paths are absolute or contain `..` are skipped entirely.
- `file` refuses to follow symlinks (`400`), so a symlink can never be used to
  escape the layer; `tree` reports symlinks (`kind: "symlink"` with
  `link_target`) and additionally resolves each link to its normalized target as
  metadata (`link_resolved`/`link_kind`), but never reads through it.
- Entry types other than regular files/directories/symlinks (devices, FIFOs) are
  not browsed as files.

Because authorization happens before any bytes are read, an unprivileged caller
cannot probe layer contents: a private repository's layers return `404`.

---

## 5. Resource caps

Tar is a sequential format, so a malicious or accidental decompression bomb is a
real concern. The browser enforces:

| Cap | Value | Purpose |
|---|---|---|
| `MAX_ENTRIES` | 100,000 entries | Bound header/entry processing per request. |
| `MAX_SCAN_BYTES` | 512 MiB | Bound total decompressed bytes read per request. |
| `MAX_FILE_BYTES` | 5 MiB | Largest single file returned by `file`. |
| `MAX_TREE_ENTRIES` | 10,000 | Largest single directory listing returned. |
| `MAX_PATH_LEN` | 4,096 bytes | Reject absurd paths before lookup. |

Exceeding the entry, byte or file caps produces `413 payload_too_large`; an
unvisited path produces `404`; an unsafe path produces `400`. A capped read is
surfaced as an `io::Error` with a sentinel message and mapped to `413`.

The caps are deliberately generous enough for real images and small enough that
a single request cannot exhaust memory or CPU.

---

## 6. Caching

A tar stream is sequential and has no central directory, so building the merged
path map means decompressing the whole archive. `tree` memoizes that map per
layer digest in `src/layer_cache.rs`: only the first request for a digest pays
the scan, and every later directory listing of that layer is served from memory.

Recursive directory sizes are folded into the cached index at the same time the
map is merged: every sized entry is added to each of its ancestor directories, so
a directory's total is one lookup rather than a re-walk. Those totals are
charged to the index's estimated weight, so they fall under the same LRU and TTL
bounds as the map itself.

The cache is bounded and self-expiring:

| Bound | Default | Behaviour |
|---|---|---|
| `LAYER_CACHE_MAX_BYTES` | 64 MiB | Estimated total footprint; least-recently-used indices are evicted first. An index whose own weight exceeds the budget is never cached, so one huge layer cannot flush every other layer. |
| `LAYER_CACHE_TTL_SECS` | 900 s | An index older than this is a miss and is rebuilt. Expired indices are reclaimed by a sweep scheduled for the earliest entry's expiry, never on a fixed cadence. |

Concurrent misses for one digest are collapsed onto a single scan: the first
caller builds on the blocking pool while the rest wait on that digest's build
lock, then observe the freshly cached index. Entries are dropped explicitly when
their blob is removed — the garbage collector reports the digests it swept and
the OCI blob delete invalidates directly. The database checks in step 1 mean a
deleted layer can never be served from a stale entry regardless.

`file` is not cached: returning a single file's bytes would still require
decompressing everything before it, so it keeps its early-exit scan.

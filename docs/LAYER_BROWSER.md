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
link_target }]`; `mode` is the raw tar mode (e.g. `420` for `0644`). `file`
returns the entry bytes with a MIME type guessed from the path via
`mime_guess` (falling back to `application/octet-stream`). `download` streams the
original archive unchanged.

---

## 2. Decompression selection

A layer's compression is inferred from its stored media type:

| Media type contains | Decompressor |
|---|---|
| `zstd` | `zstd::stream::read::Decoder` |
| `gzip` | `flate2::read::GzDecoder` |
| anything else | none (plain `tar`) |

This covers `application/vnd.oci.image.layer.v1.tar`,
`...tar+gzip`, `...tar+zstd`, `application/vnd.docker.image.rootfs.diff.tar.gzip`
and the foreign-layer variants. A wrong or missing media type is treated as a
plain tar; if the bytes are not a valid tar the read fails cleanly with a `500`
rather than leaking a panic.

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

`tree` scans the whole archive once and folds entries into a map keyed by
normalized path, applying OCI layer rules in stream order:

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
  `link_target`) but never follows them.
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

## 6. Cost and caching opportunity

A tar stream is sequential and has no central directory, so **each request
rescans from the beginning of the layer**. `tree` must always scan the whole
archive to resolve overrides and whiteouts; `file` stops at the first match but
still decompresses every preceding entry.

This is acceptable for interactive browsing because:

- nothing is buffered except the requested file (≤ 5 MiB),
- the scan runs on a blocking thread pool, and
- the layer bytes are read from the local content store.

If repeated browsing of the same large layer becomes hot, the natural
optimization is a per-digest, bounded in-memory index cache: the first `tree`
builds an ordered `(path, kind, size, mode, link_target, uncompressed_offset)`
table while streaming (still respecting the caps), and subsequent `tree`/`file`
requests serve from that table without re-decompressing. The cache could be
keyed by layer digest, sized by entry count, and invalidated by the garbage
collector when the blob is removed. The current implementation deliberately does
not cache so that behaviour stays predictable and memory use is bounded by the
request.

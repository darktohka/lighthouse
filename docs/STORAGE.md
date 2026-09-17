# Lighthouse — Storage & Garbage Collection

This document describes the on-disk content store, how content is addressed,
the metadata reference graph that ties content together, and the mark-and-sweep
garbage collector. The storage layer (`src/storage/`) and the schema
(`migrations/0001_init.sql`) are the source of truth; this is the narrative form.

---

## 1. Directory layout

Everything lives under a single flat host directory, `DATA_DIR` (default
`./data`). Nothing is hidden inside Docker volumes, so backups and `rsync`
mirroring are ordinary file operations.

```text
data/
├── blobs/<algorithm>/<hex[0..2]>/<hex>/data
├── uploads/<uuid>/data
├── uploads/<uuid>/startedat
└── tmp/
```

| Path | Contents |
|---|---|
| `blobs/<algorithm>/<hex[0..2]>/<hex>/data` | The immutable bytes of a content-addressed blob. |
| `uploads/<uuid>/data` | The bytes received so far for an in-progress upload. |
| `uploads/<uuid>/startedat` | RFC 3339 timestamp used to expire abandoned uploads. |
| `tmp/` | Scratch space; uploads are moved out of here atomically. |

The two-character shard (`hex[0..2]`) keeps any single directory to a manageable
number of entries; `Storage::blob_path` derives the whole path from a digest.
A blob is written exactly once and never mutated: a later upload of the same
content is discarded and the existing file is kept
(`Storage::put_blob_from_path`).

---

## 2. Digest addressing

A digest is written `algorithm:encoded` and modelled by the `Digest` newtype
(`src/oci/digest.rs`):

- The algorithm matches `^[a-z0-9]+(?:[+._-][a-z0-9]+)*$`.
- The encoded part matches `^[a-zA-Z0-9=_-]+$`.
- Known algorithms (`sha256`, `sha512`, `blake3`) additionally require lowercase
  hexadecimal of a fixed length; unknown but grammar-conforming algorithms are
  accepted for forward compatibility.
- `Digest::from_bytes` uses **sha256** as the canonical algorithm.
- `Digest::verifier()` produces a streaming verifier, so uploads are hashed while
  they are written and never buffered in memory.

Content is deduplicated by digest: two repositories, or two tags, that reference
the same layer share one file on disk. The `blobs` table stores one row per
digest with its size and (optional) media type; `blob_repositories` records every
repository that links it.

---

## 3. Reference graph

Metadata is split across two edge tables so the graph can be walked without
parsing manifest bodies at collection time:

```text
tags ──▶ manifests ──▶ manifest_blobs   ──▶ blobs   (config / layer / subject)
                 └────▶ manifest_children ──▶ manifests (index / manifest list)
```

- `manifest_blobs(manifest_id, blob_id, role)` — the config and layer edges of a
  single-image manifest. `role` is `config`, `layer` or `subject`.
- `manifest_children(parent_manifest_id, child_manifest_id, platform_*)` — the
  child edges of an index or Docker manifest list, carrying `os`, `architecture`
  and `variant` for platform resolution.
- `manifest_repositories` — which repositories link a manifest (a manifest may
  live in several repositories).
- `blob_repositories` — which repositories link a blob. A blob with zero links is
  dangling.
- `tags` — named pointers to manifests; the roots of user-visible content.

Reference edges are written transactionally when a manifest is stored
(`Registry::put_manifest` → `record_manifest_references`), so a manifest and its
graph edges are always consistent.

### Reachability

A blob is *reachable* from a tag when some path exists:

```text
tag.manifest ──▶ (manifest_children)* ──▶ manifest_blobs ──▶ blob
```

The control-plane size queries express this as a recursive CTE
(`api::load_tag_blob_rows`):

```sql
WITH RECURSIVE reach(tag_id, manifest_id) AS (
    SELECT id, manifest_id FROM tags
    UNION
    SELECT r.tag_id, mc.child_manifest_id
    FROM reach r JOIN manifest_children mc ON mc.parent_manifest_id = r.manifest_id
), tag_blob(tag_id, blob_id) AS (
    SELECT DISTINCT r.tag_id, mb.blob_id
    FROM reach r JOIN manifest_blobs mb ON mb.manifest_id = r.manifest_id
)
SELECT tb.tag_id, t.repository_id, tb.blob_id, b.size
FROM tag_blob tb JOIN tags t ON t.id = tb.tag_id JOIN blobs b ON b.id = tb.blob_id;
```

### Size accounting

- `total_size` for a tag is the sum of `blobs.size` over its reachable set
  (config + layers; for an index, the union of its children's reachable blobs).
- `unique_size` counts only blobs referenced by **exactly one tag anywhere in the
  registry** (global uniqueness). `shared_size = total_size - unique_size`.
- Repository totals are computed over the union of its tags' reachable blobs;
  repository `unique_size` is the sum of globally-exclusive blobs.

Global uniqueness is deliberate: it is the storage that deleting a single tag
would actually reclaim, regardless of whether the blob is shared with a tag in a
different repository.

---

## 4. Garbage collection

`storage::gc::collect(&registry, &storage, dry_run)` is a mark-and-sweep pass
over the reference graph (described in `ARCHITECTURE.md §9`):

1. **Roots** — every manifest referenced by a tag, plus every manifest linked to
   a repository (`manifest_repositories`).
2. **Mark** — walk `manifest_blobs` and `manifest_children` transitively from the
   roots; the marked set contains manifests and blobs.
3. **Sweep** — delete every blob not marked (its row, its `blob_repositories`
   links, and its file on disk), then repeatedly delete manifests that are
   unreachable: no repository link, no tag and no surviving parent index.

The sweep runs inside a transaction; `dry_run` rolls it back and only reports the
projected counts. Deleting a parent index cascades its child edges, which may
make further manifests eligible on the next pass — hence the loop.

The control plane triggers collection after repository deletion, namespace
deletion, single tag deletion and both batch-delete endpoints, so unreferenced
blobs are reclaimed immediately rather than lingering until a scheduled run.
In-progress uploads are never touched.

`GcReport` reports `marked`, `blobs_deleted`, `manifests_deleted` and
`bytes_reclaimed`.

---

## 5. Uploads

Uploads are resumable and out-of-order:

- `POST` creates `uploads/<uuid>/` with an empty `data` file and a `startedat`
  marker.
- `PATCH` appends contiguous chunks; the durable file size is the authoritative
  offset, so gaps and rewrites are rejected.
- `PUT` verifies the content against the client-supplied digest while streaming,
  then atomically renames `uploads/<uuid>/data` into the blob store. The upload
  directory is always removed, even when the digest already existed.
- Stale uploads are expired using `startedat` and `upload_session_ttl_secs`.

Because the digest is verified at finalization, a partially written upload can
never leave a half-registered blob behind.

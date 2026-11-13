# Lighthouse — OCI Distribution Compliance

This document records how the `/v2/` surface maps onto the OCI Distribution
Specification and the legacy Docker Registry HTTP API V2, and what has been
verified against a real Docker client.

---

## 1. Endpoint surface

Every `/v2` response carries `Docker-Distribution-API-Version: registry/2.0`,
including errors and the router fallback.

| Method | Path | Success | Notes |
|---|---|---|---|
| any | `/v2/` | 200 `{}` | returns **401 + the configured `WWW-Authenticate` challenge** when anonymous (Bearer realm=`<base_url>/api/auth/token` by default, or Basic with `REGISTRY_AUTH_CHALLENGE`), so `docker login` can validate credentials |
| GET | `/v2/<name>/tags/list` | 200 | `?n`/`?last`, `Link: rel="next"` |
| GET/HEAD | `/v2/<name>/manifests/<reference>` | 200 | `Content-Type`, `Content-Length`, `Docker-Content-Digest`, `ETag`, `304` on `If-None-Match` |
| PUT | `/v2/<name>/manifests/<reference>` | 201 | `Location` (canonical digest URL), `Docker-Content-Digest` |
| DELETE | `/v2/<name>/manifests/<reference>` | 202 | tag or digest reference; also triggers a garbage-collection pass (subject to the GC grace window) so unreferenced blobs are reclaimed from the database and disk |
| GET/HEAD | `/v2/<name>/blobs/<digest>` | 200 / 206 / 304 | `Range`, `Accept-Ranges`, `Content-Range`, `ETag` |
| DELETE | `/v2/<name>/blobs/<digest>` | 202 | `Content-Length: 0` |
| POST | `/v2/<name>/blobs/uploads/` | 202 | start; `?mount=&from=` → 201; `?digest=` monolithic → 201 |
| GET/HEAD | `/v2/<name>/blobs/uploads/<uuid>` | 204 | `Range: 0-<size-1>`, `Location`, `Docker-Upload-UUID` |
| PATCH | `/v2/<name>/blobs/uploads/<uuid>` | 202 | chunked/resumable; `Content-Range` validation |
| PUT | `/v2/<name>/blobs/uploads/<uuid>?digest=` | 201 | digest verified cryptographically |
| DELETE | `/v2/<name>/blobs/uploads/<uuid>` | 204 | cancel |
| GET | `/v2/_catalog` | 200 | `?n`/`?last`, `Link: rel="next"` |
| GET | `/v2/<name>/referrers/<digest>` | 200 | OCI 1.1 referrers index; `?artifactType=` filter (`OCI-Filters-Applied: artifactType`) |

Repository names may be nested (`alice/more/complicated/app`); the router
disambiguates a single catch-all into the concrete endpoint.

---

## 1a. in-toto attestations

Attestations (SLSA provenance, SBOMs, …) are recognised, surfaced in the control
plane and served back untouched. The registry never verifies a signature: an
attestation is opaque content addressed by digest.

Both of the shapes clients use are supported:

**In-index (BuildKit `--provenance` / `--sbom`).** Attestations are extra entries
in the image index, with the placeholder platform `unknown/unknown` and the
annotations

```
vnd.docker.reference.type:   attestation-manifest
vnd.docker.reference.digest: sha256:<digest of the image manifest it belongs to>
```

Each such entry is a manifest whose `artifactType` is
`application/vnd.docker.attestation.manifest.v1+json` and whose layers are
`application/vnd.in-toto+json` statements, one per predicate. The
`vnd.docker.reference.digest` annotation is what ties an attestation to the
platform it was built for.

**Referrers (OCI 1.1).** The attestation is a separate manifest with a `subject`
pointing at the image manifest, discovered through the referrers API.

Media types and annotations:

| Kind | Value |
|---|---|
| Statement layer | `application/vnd.in-toto+json` (also `application/vnd.in-toto.<predicate>+json`) |
| Attestation manifest `artifactType` | `application/vnd.docker.attestation.manifest.v1+json` |
| Predicate annotation | `in-toto.io/predicate-type` |
| Reference annotations | `vnd.docker.reference.type`, `vnd.docker.reference.digest` |

`GET /v2/<name>/referrers/<digest>` returns an image index whose `manifests`
array holds one descriptor per referrer, each carrying its own `artifactType`
and annotations. A subject with no referrers returns `200` with an empty
`manifests` array — never `404`, so clients do not fall back to tag scanning. A
digest that is not present in the repository is `404 MANIFEST_UNKNOWN`.

### Control plane

`GET /api/repositories/{namespace}/{repo}/tags/{tag}` gains an `attestations`
array, always present (possibly empty):

```json
{
  "attestations": [
    {
      "digest": "sha256:a77201ec…",
      "media_type": "application/vnd.oci.image.manifest.v1+json",
      "artifact_type": "application/vnd.docker.attestation.manifest.v1+json",
      "size": 837,
      "kinds": ["provenance"],
      "predicates": [
        {
          "predicate_type": "https://slsa.dev/provenance/v1",
          "kind": "provenance",
          "digest": "sha256:fc4f36c839eb…",
          "media_type": "application/vnd.in-toto+json",
          "size": 1741
        }
      ],
      "subject_digest": "sha256:15b657ae…",
      "referenced_digest": "sha256:15b657ae…",
      "target": { "digest": "sha256:15b657ae…", "os": "linux", "architecture": "amd64", "variant": null }
    }
  ]
}
```

- `kinds` are the deduplicated short labels derived from each statement's
  predicate type, in statement order: `https://slsa.dev/provenance/v1` →
  `provenance`, `https://spdx.dev/Document` → `sbom`.
- `target` is the image platform the attestation was built for. It is `null` only
  when the attestation names no subject at all.
- **Attestation manifests are excluded from `platforms` and `platform_details`.**
  The index's `unknown/unknown` entries are not image platforms, so they no
  longer appear as bogus platform badges.

The UI marks a platform that has attestations with a shield icon on its badge
and in the platform scope row; the predicate labels are exposed through the
tooltip and to assistive technology rather than as visible text. The
**Attestations** tab (last) lists one row per attestation and offers **View
JSON**, which opens the attestation manifest and each in-toto statement in the
same JSON viewer the Manifest and Config tabs use.

---

## 2. Status codes and error envelope

Errors use the OCI envelope:

```json
{ "errors": [ { "code": "BLOB_UNKNOWN", "message": "blob unknown to registry", "detail": { "digest": "sha256:…" } } ] }
```

| Code | HTTP | Code | HTTP |
|---|---|---|---|
| `UNKNOWN` | 500 | `MANIFEST_INVALID` | 400 |
| `UNSUPPORTED` | 405 | `MANIFEST_UNVERIFIED` | 400 |
| `UNAUTHORIZED` | 401 | `MANIFEST_BLOB_UNKNOWN` | 400 |
| `DENIED` | 403 | `BLOB_UNKNOWN` | 404 |
| `UNAVAILABLE` | 503 | `BLOB_UPLOAD_UNKNOWN` | 404 |
| `TOOMANYREQUESTS` | 429 | `BLOB_UPLOAD_INVALID` | 404 |
| `DIGEST_INVALID` | 400 | `NAME_UNKNOWN` | 404 |
| `SIZE_INVALID` | 400 | `MANIFEST_UNKNOWN` | 404 |
| `RANGE_INVALID` | 416 | `PAGINATION_NUMBER_INVALID` | 400 |
| `NAME_INVALID` / `TAG_INVALID` | 400 | | |

Authorization: anonymous → `401` with the configured challenge
(`REGISTRY_AUTH_CHALLENGE`, Bearer by default); authenticated but unauthorized
→ `403 DENIED`. Private resources are hidden as `404` on the control-plane API
so they are not enumerable.

---

## 3. Content negotiation

`Accept` is parsed with quality values, wildcards and `application/*` support.

- The stored media type is served when the client accepts it.
- Stored OCI manifest/index not accepted by the client → `404 MANIFEST_UNKNOWN`
  with a descriptive message (never `406`).
- Stored Docker manifest list requested without list support → the default
  `linux/amd64` child manifest is served when present.
- Unknown or extra JSON fields and unknown layer/config media types are accepted.

Supported media types:

| OCI | Docker |
|---|---|
| `application/vnd.oci.image.manifest.v1+json` | `application/vnd.docker.distribution.manifest.v2+json` |
| `application/vnd.oci.image.index.v1+json` | `application/vnd.docker.distribution.manifest.list.v2+json` |
| `application/vnd.oci.image.config.v1+json` | `application/vnd.docker.container.image.v1+json` |
| `application/vnd.oci.image.layer.v1.tar` / `+gzip` / `+zstd` | `application/vnd.docker.image.rootfs.diff.tar.gzip` / `foreign.diff.tar.gzip` |

---

## 4. Hashing and verification

- Digests follow the OCI grammar
  `^[a-z0-9]+(?:[+._-][a-z0-9]+)*:[a-zA-Z0-9=_-]+$`.
- Known algorithms require lowercase hex of a fixed length: `sha256` 64,
  `sha512` 128, `blake3` 64. Grammar-valid unknown algorithms are accepted.
- **Every uploaded blob is hashed while it is written and the digest is verified
  before commit**; a mismatch is rejected with `DIGEST_INVALID` and the upload is
  discarded.
- Manifest digests are computed over the exact received bytes; a digest
  reference that does not match the body is rejected.

---

## 5. Storage semantics

- Blobs are content-addressed and deduplicated by digest:
  `DATA_DIR/blobs/<algorithm>/<hex[0..2]>/<hex>/data`.
- Upload sessions live in `DATA_DIR/uploads/<uuid>/` until commit.
- Reference links are stored relationally (`blob_repositories`,
  `manifest_repositories`, `manifest_blobs`, `manifest_children`, `tags`).
- Deleting a tag or image removes its links, and unreferenced blobs are reclaimed
  by the mark-and-sweep collector (`docs/STORAGE.md`).

---

## 6. Verified against the Docker client

Executed against Docker Engine 29.8.0 and `buildx` v0.35.0 on a local instance:

| Scenario | Result |
|---|---|
| `docker login` against the registry | `Login Succeeded` (validated via the `/v2/` 401 challenge) |
| Push `FROM scratch` image | 3 requests logged: upload start → PATCH → PUT 201 |
| Pull back and compare digest | Digest identical; blob dedup reported "Already exists" |
| Layer bytes recovered from `DATA_DIR` | Tar extracted, file content matched |
| `docker buildx imagetools create` (manifest list) | Pushed and inspected; two platform entries |
| `docker pull` of the manifest list | Succeeded |
| `HEAD manifests/<tag>` | `Content-Type`, `Content-Length`, `Docker-Content-Digest`, `ETag` correct |
| `Range: bytes=0-4` on a layer | `206` with the gzip header bytes |
| `If-None-Match: "<digest>"` | `304` |
| `GET /v2/_catalog?n=1` | `Link: </v2/_catalog?n=1&last=alice/demo>; rel="next"` |
| `GET /v2/<name>/tags/list?n=1` with two tags | `Link: <…?n=1&last=v1>; rel="next"` |
| `?n=abc` | `400 PAGINATION_NUMBER_INVALID` |
| `DELETE manifests/v2` | `202`, tag removed from listings |
| Unknown path under `/v2/` | `404` OCI envelope, never the SPA |
| Anonymous pull of a private repository | `401` |
| Authenticated non-owner (bob) on alice's repository | `403` |
| Push to own namespace | `202` |
| Audit trail | `registry_events` rows for `blob.push`, `manifest.push`, `blob.pull`, `manifest.pull` with actor, IP and user agent |
| Pull statistics | `pull_stats` daily rollups per repository and per tag |
| Login history | `login_events` rows for password and Basic-auth use |
| `buildx --provenance=mode=max --sbom=true` push (2 platforms) | Index accepted with both `unknown/unknown` attestation entries; tag detail reports an attestation per platform with kinds `[sbom, provenance]` |
| Tag detail of `registry.tohka.us/base/planka:latest` copied with `imagetools create` | `platforms` = `linux/amd64`, `linux/arm64` only; one `provenance` attestation targeting each |
| `GET /v2/<name>/referrers/<image digest>` | `200` index listing the attestation manifest with `artifactType` |
| `?artifactType=` mismatch | `200` with an empty `manifests` array |
| Referrers of a digest absent from the repository | `404 MANIFEST_UNKNOWN` |

The automated suite additionally covers chunked/resumable uploads, out-of-order
`Content-Range` (416), `Content-Length` mismatch (400), cross-repo mounts, upload
cancellation, digest mismatch, missing-blob manifest rejection, multi-arch index
round-trips, Docker schema2 and manifest-list handling, and nested repository
routing.

---

## 7. Known deviations from the Go reference

- The reference implementation does not answer a monolithic
  `POST …/uploads/?digest=` with `201`; Lighthouse implements it as the
  specification documents it, and also supports the POST → PUT flow.
- Where the reference returns `401` for an insufficient scope with a Bearer
  challenge, Lighthouse returns `403 DENIED` for authenticated-but-forbidden
  requests. The advertised `401` challenge is configurable via
  `REGISTRY_AUTH_CHALLENGE` (`bearer`, `basic` or `both`); Docker clients handle
  both schemes.
- The reference supports redirect-to-storage and token-server auth; Lighthouse
  streams blobs directly and implements the token-server flow at
  `/api/auth/token`. Basic credentials are accepted in every challenge mode.

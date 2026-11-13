-- OCI 1.1 referrers support.
--
-- A manifest may declare a `subject`: the descriptor of the artifact it is an
-- attachment for. BuildKit uses this for referrer-style attestations, and the
-- OCI 1.1 referrers API resolves `GET /v2/<name>/referrers/<digest>` by reverse
-- lookup. Storing the reference as a column lets that be an indexed query
-- instead of a full manifest scan.
ALTER TABLE manifests ADD COLUMN subject_digest TEXT;

CREATE INDEX idx_manifests_subject ON manifests (subject_digest);

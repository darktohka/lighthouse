-- Hidden repositories are excluded from anonymous control-plane browsing of the
-- web `/api` surfaces (listings, counts, detail, tags, layers, activity and
-- public profile stats). OCI authorization is unchanged: `hidden` means
-- "unlisted", not "private", so anonymous `/v2` pulls still succeed.
ALTER TABLE repositories ADD COLUMN is_hidden INTEGER NOT NULL DEFAULT 0;

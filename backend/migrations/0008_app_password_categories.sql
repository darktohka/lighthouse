-- Owner-scoped categories for grouping app passwords in the control plane.
-- Deleting a category never touches credentials: the reference is cleared
-- instead (ON DELETE SET NULL), and a category with credentials must be
-- emptied by hand.
CREATE TABLE app_password_categories (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name          TEXT    NOT NULL,
    created_at    TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_app_password_categories_owner_name
    ON app_password_categories (owner_user_id, name COLLATE NOCASE);

ALTER TABLE app_passwords
    ADD COLUMN category_id INTEGER REFERENCES app_password_categories (id) ON DELETE SET NULL;

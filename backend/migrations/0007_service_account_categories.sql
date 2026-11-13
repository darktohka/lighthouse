-- Owner-scoped categories for grouping service accounts in the control plane.
-- Deleting a category never touches accounts: the reference is cleared instead
-- (ON DELETE SET NULL), and a category with accounts must be emptied by hand.
CREATE TABLE service_account_categories (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name          TEXT    NOT NULL,
    created_at    TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_service_account_categories_owner_name
    ON service_account_categories (owner_user_id, name COLLATE NOCASE);

ALTER TABLE service_accounts
    ADD COLUMN category_id INTEGER REFERENCES service_account_categories (id) ON DELETE SET NULL;

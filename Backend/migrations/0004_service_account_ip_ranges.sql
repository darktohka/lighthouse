CREATE TABLE service_account_ip_ranges (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    service_account_id INTEGER NOT NULL REFERENCES service_accounts (id) ON DELETE CASCADE,
    cidr               TEXT    NOT NULL,
    created_at         TEXT    NOT NULL
);
CREATE UNIQUE INDEX idx_service_account_ip_ranges_unique
    ON service_account_ip_ranges (service_account_id, cidr);

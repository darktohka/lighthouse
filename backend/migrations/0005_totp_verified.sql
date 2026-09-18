-- Lighthouse — deferred TOTP activation.
--
-- Enrolment is split into verify-then-confirm: `verify_setup` validates a live
-- code and stamps `totp_verified_at`, and `confirm_enable` later flips
-- `totp_enabled` with no code. This lets a user take as long as they need to
-- store the backup codes without needing a fresh (possibly expired) code.
--
-- `users` is only ever ALTERed: it has inbound foreign keys from many tables,
-- so a table rebuild would be unsafe.

-- NULL until a pending enrolment has been confirmed with a live code; cleared
-- again once the factor is enabled or the enrolment is abandoned/disabled.
ALTER TABLE users ADD COLUMN totp_verified_at TEXT;

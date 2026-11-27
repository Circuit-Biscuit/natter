-- Two-factor sign-in with an authenticator app (TOTP, RFC 6238): the shared secret (base32; set but not enabled =
-- set up, not confirmed), the last time step used (each code works once), and single-use recovery codes, hashed.
ALTER TABLE users ADD COLUMN totp_secret TEXT;
ALTER TABLE users ADD COLUMN totp_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN totp_last_step INTEGER NOT NULL DEFAULT 0;

CREATE TABLE totp_recovery (
    user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT    NOT NULL,
    PRIMARY KEY (user_id, code_hash)
);

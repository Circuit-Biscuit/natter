-- An optional e-mail address per account (only its owner sees it), for resetting a lost password, and the pending
-- reset codes: one per account, stored hashed, with an expiry and a count of wrong tries.
ALTER TABLE users ADD COLUMN email TEXT;
CREATE UNIQUE INDEX users_email ON users(email) WHERE email IS NOT NULL;

CREATE TABLE password_resets (
    user_id    INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    code_hash  TEXT    NOT NULL,
    expires_at INTEGER NOT NULL,
    tries      INTEGER NOT NULL DEFAULT 0
);

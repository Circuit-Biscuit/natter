-- Open sign-up on a server that sends e-mail: codes confirming an address before an account gets it. One per
-- address, stored hashed, with an expiry and a count of wrong tries.
CREATE TABLE email_codes (
    email      TEXT    PRIMARY KEY,
    code_hash  TEXT    NOT NULL,
    expires_at INTEGER NOT NULL,
    tries      INTEGER NOT NULL DEFAULT 0
);

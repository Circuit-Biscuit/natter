-- Reports of messages to the server's admins, with a copy of the message as reported (it may be gone by the time an
-- admin looks), one per person per message; blocks between people; suspended accounts can't sign in.
CREATE TABLE reports (
    id          INTEGER PRIMARY KEY,
    reporter_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    author_id   INTEGER REFERENCES users(id) ON DELETE SET NULL,
    channel_id  INTEGER NOT NULL,
    message_id  INTEGER NOT NULL,
    content     TEXT    NOT NULL,
    reason      TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    resolved_at INTEGER,
    resolved_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    resolution  TEXT
);
CREATE INDEX reports_open ON reports(resolved_at);
CREATE UNIQUE INDEX reports_once ON reports(reporter_id, message_id);

CREATE TABLE blocks (
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blocked_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, blocked_id)
);

ALTER TABLE users ADD COLUMN suspended INTEGER NOT NULL DEFAULT 0;

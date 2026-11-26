-- Laklak schema v1. Ids are laklak_proto::Id (time-ordered), times are unix ms.

CREATE TABLE uploads (
    id           INTEGER PRIMARY KEY,
    token        TEXT    NOT NULL UNIQUE,
    owner_id     INTEGER NOT NULL,
    message_id   INTEGER REFERENCES messages(id) ON DELETE CASCADE,
    filename     TEXT    NOT NULL,
    content_type TEXT    NOT NULL,
    size         INTEGER NOT NULL,
    width        INTEGER,
    height       INTEGER,
    used         INTEGER NOT NULL DEFAULT 0, -- avatar or space icon
    created_at   INTEGER NOT NULL
);
CREATE INDEX uploads_message ON uploads(message_id);

CREATE TABLE users (
    id            INTEGER PRIMARY KEY,
    username      TEXT    NOT NULL UNIQUE,
    display_name  TEXT    NOT NULL,
    password_hash TEXT    NOT NULL,
    avatar_id     INTEGER REFERENCES uploads(id) ON DELETE SET NULL,
    about         TEXT    NOT NULL DEFAULT '',
    status        TEXT    NOT NULL DEFAULT 'online',
    custom_status TEXT,
    admin         INTEGER NOT NULL DEFAULT 0,
    settings      TEXT    NOT NULL DEFAULT '{}',
    created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
    id         INTEGER PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT    NOT NULL UNIQUE,
    device     TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);

CREATE TABLE spaces (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    icon_id    INTEGER REFERENCES uploads(id) ON DELETE SET NULL,
    owner_id   INTEGER NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL
);

CREATE TABLE members (
    space_id  INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role      TEXT    NOT NULL,
    nickname  TEXT,
    joined_at INTEGER NOT NULL,
    PRIMARY KEY (space_id, user_id)
);
CREATE INDEX members_user ON members(user_id);

CREATE TABLE bans (
    space_id   INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason     TEXT    NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    PRIMARY KEY (space_id, user_id)
);

CREATE TABLE invites (
    code       TEXT    PRIMARY KEY,
    space_id   INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    creator_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    uses       INTEGER NOT NULL DEFAULT 0,
    max_uses   INTEGER,
    expires_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX invites_space ON invites(space_id);

CREATE TABLE channels (
    id              INTEGER PRIMARY KEY,
    space_id        INTEGER REFERENCES spaces(id) ON DELETE CASCADE,
    kind            TEXT    NOT NULL,
    name            TEXT    NOT NULL,
    topic           TEXT    NOT NULL DEFAULT '',
    position        INTEGER NOT NULL DEFAULT 0,
    parent_id       INTEGER,
    read_only       INTEGER NOT NULL DEFAULT 0,
    last_message_id INTEGER,
    dm_key          TEXT    UNIQUE -- "small:big" user ids of a 1:1 DM
);
CREATE INDEX channels_space ON channels(space_id);

CREATE TABLE dm_members (
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (channel_id, user_id)
);
CREATE INDEX dm_members_user ON dm_members(user_id);

CREATE TABLE messages (
    id               INTEGER PRIMARY KEY,
    channel_id       INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    author_id        INTEGER NOT NULL REFERENCES users(id),
    kind             TEXT    NOT NULL DEFAULT 'default',
    content          TEXT    NOT NULL,
    edited_at        INTEGER,
    reply_to         INTEGER,
    mention_everyone INTEGER NOT NULL DEFAULT 0,
    pinned_at        INTEGER,
    nonce            TEXT
);
CREATE INDEX messages_channel ON messages(channel_id, id);
CREATE INDEX messages_author ON messages(author_id);
CREATE UNIQUE INDEX messages_nonce ON messages(author_id, nonce) WHERE nonce IS NOT NULL;

-- Who actually got pinged by a message. Mention counts are derived from this, never stored,
-- so deletes, edits and mark-unread can't leave a counter wrong.
CREATE TABLE pings (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL,
    PRIMARY KEY (user_id, message_id)
);
CREATE INDEX pings_message ON pings(message_id);

CREATE TABLE reactions (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    emoji      TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (message_id, user_id, emoji)
);

CREATE TABLE read_states (
    user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id   INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    last_read_id INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, channel_id)
);

CREATE VIRTUAL TABLE messages_fts USING fts5(
    content, content = 'messages', content_rowid = 'id', tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;
CREATE TRIGGER messages_fts_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES ('delete', old.id, old.content);
END;
CREATE TRIGGER messages_fts_au AFTER UPDATE OF content ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES ('delete', old.id, old.content);
    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;

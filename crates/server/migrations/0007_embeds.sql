-- Link previews: per message (a JSON array of Embed; "[]" = looked, nothing to show) and per URL (shared cache).
CREATE TABLE message_embeds (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    data       TEXT    NOT NULL
);
CREATE TABLE embed_cache (
    url        TEXT    PRIMARY KEY,
    data       TEXT,
    fetched_at INTEGER NOT NULL
);

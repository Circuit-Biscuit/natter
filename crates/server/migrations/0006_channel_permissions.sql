-- Channel permission overrides (Discord-style): allow / deny bits per role, @everyone (target = the space id) or member.
CREATE TABLE channel_overwrites (
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    target_id  INTEGER NOT NULL,
    kind       TEXT    NOT NULL,
    allow      INTEGER NOT NULL DEFAULT 0,
    deny       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (channel_id, target_id)
);
CREATE INDEX channel_overwrites_target ON channel_overwrites(target_id);

-- The new View channels permission (1 << 16): everyone keeps seeing what they saw. New spaces set it explicitly
-- (the column default from 0005 predates it).
UPDATE spaces SET everyone_permissions = everyone_permissions | 65536;

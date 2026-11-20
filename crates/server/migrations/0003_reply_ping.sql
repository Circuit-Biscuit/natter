-- Replies that ping the replied-to author ("@ ON").
ALTER TABLE messages ADD COLUMN reply_ping INTEGER NOT NULL DEFAULT 0;

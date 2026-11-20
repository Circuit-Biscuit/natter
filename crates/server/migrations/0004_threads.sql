-- Threads are channels of kind 'thread': parent_id = the text channel, space_id = its space.
-- Their message count is counted, not stored.
ALTER TABLE channels ADD COLUMN starter_id INTEGER;
ALTER TABLE channels ADD COLUMN owner_id INTEGER;
ALTER TABLE channels ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;
-- One thread per message.
CREATE UNIQUE INDEX channels_starter ON channels(starter_id) WHERE starter_id IS NOT NULL;
CREATE INDEX channels_parent ON channels(parent_id);

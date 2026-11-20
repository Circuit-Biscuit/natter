-- Daily Word: one puzzle per language per day, the same for everyone on the server.

-- Server secrets for the game (the shuffled order of answers comes from a random seed kept here).
CREATE TABLE game_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- A player's game of one day in one language.
CREATE TABLE word_plays (
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    lang        TEXT    NOT NULL,
    day         INTEGER NOT NULL,
    guesses     TEXT    NOT NULL DEFAULT '[]',
    hard        INTEGER NOT NULL DEFAULT 0,
    solved      INTEGER NOT NULL DEFAULT 0,
    finished_at INTEGER,
    space_id    INTEGER,
    PRIMARY KEY (user_id, lang, day)
);
CREATE INDEX word_plays_day ON word_plays(lang, day);

-- Where a space's results and daily summary go, and which summaries were posted.
ALTER TABLE spaces ADD COLUMN game_channel_id INTEGER;
CREATE TABLE game_summaries (
    space_id INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    day      INTEGER NOT NULL,
    PRIMARY KEY (space_id, day)
);

-- App accounts (the Daily Word bot) that post but never sign in.
ALTER TABLE users ADD COLUMN bot INTEGER NOT NULL DEFAULT 0;

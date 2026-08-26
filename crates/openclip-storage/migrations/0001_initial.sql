PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY NOT NULL,
    game        TEXT NOT NULL,
    started_at  TEXT NOT NULL,
    ended_at    TEXT
);

CREATE TABLE IF NOT EXISTS matches (
    id            TEXT PRIMARY KEY NOT NULL,
    session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    map           TEXT,
    mode          TEXT NOT NULL,
    started_at    TEXT NOT NULL,
    ended_at      TEXT,
    score_team_a  INTEGER,
    score_team_b  INTEGER
);

CREATE INDEX IF NOT EXISTS idx_matches_started_at ON matches(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_matches_session_id ON matches(session_id);

CREATE TABLE IF NOT EXISTS rounds (
    id          TEXT PRIMARY KEY NOT NULL,
    match_id    TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    number      INTEGER NOT NULL,
    started_at  TEXT NOT NULL,
    ended_at    TEXT,
    UNIQUE(match_id, number)
);

CREATE TABLE IF NOT EXISTS events (
    id                TEXT PRIMARY KEY NOT NULL,
    match_id          TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    round_id          TEXT REFERENCES rounds(id) ON DELETE SET NULL,
    source_game       TEXT NOT NULL,
    event_type        TEXT NOT NULL,
    occurred_at       TEXT NOT NULL,
    match_offset_ms   INTEGER,
    metadata_json     TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_events_match_time
    ON events(match_id, match_offset_ms);

CREATE TABLE IF NOT EXISTS moments (
    id                    TEXT PRIMARY KEY NOT NULL,
    match_id              TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    round_id              TEXT REFERENCES rounds(id) ON DELETE SET NULL,
    start_offset_ms       INTEGER NOT NULL,
    end_offset_ms         INTEGER NOT NULL,
    classification        TEXT NOT NULL,
    importance            REAL NOT NULL,
    is_favorite           INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_moments_match_importance
    ON moments(match_id, importance DESC);

CREATE TABLE IF NOT EXISTS moment_events (
    moment_id  TEXT NOT NULL REFERENCES moments(id) ON DELETE CASCADE,
    event_id   TEXT NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    PRIMARY KEY(moment_id, event_id)
);

CREATE TABLE IF NOT EXISTS clips (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    moment_id   TEXT NOT NULL UNIQUE REFERENCES moments(id) ON DELETE CASCADE,
    path        TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS settings (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    value_json  TEXT NOT NULL
);


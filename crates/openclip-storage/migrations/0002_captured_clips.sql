CREATE TABLE IF NOT EXISTS captured_clips (
    id TEXT PRIMARY KEY NOT NULL,
    match_id TEXT REFERENCES matches(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
    width INTEGER NOT NULL CHECK (width > 0),
    height INTEGER NOT NULL CHECK (height > 0),
    frames_per_second INTEGER NOT NULL CHECK (frames_per_second > 0),
    codec TEXT NOT NULL,
    encoder TEXT NOT NULL,
    has_game_audio INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_captured_clips_match_created
    ON captured_clips(match_id, created_at DESC);

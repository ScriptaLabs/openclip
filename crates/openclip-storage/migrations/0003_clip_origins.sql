CREATE TABLE IF NOT EXISTS clip_origins (
    clip_id TEXT PRIMARY KEY NOT NULL REFERENCES captured_clips(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    moment_id TEXT UNIQUE REFERENCES moments(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_clip_origins_moment ON clip_origins(moment_id);

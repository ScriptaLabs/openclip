-- Manual replays can be favorites too, so retention keeps them. Applied by the storage crate only
-- when the column is missing, because SQLite has no ADD COLUMN IF NOT EXISTS.
ALTER TABLE captured_clips ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;

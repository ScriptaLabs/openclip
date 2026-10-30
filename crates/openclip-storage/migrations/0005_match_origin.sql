-- Where a match was played: official, workshop or community. Applied by the storage crate only when
-- the column is missing, because SQLite has no ADD COLUMN IF NOT EXISTS.
ALTER TABLE matches ADD COLUMN origin TEXT NOT NULL DEFAULT 'official';

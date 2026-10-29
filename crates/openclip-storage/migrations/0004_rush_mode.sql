-- Matches played on Rush before the app knew the mode were stored as "unknown". The Rush map
-- (rush_001) exists only in that mode, so those rows can be labelled correctly. Safe to repeat.
UPDATE matches
SET mode = 'rush'
WHERE mode = 'unknown'
  AND (map = 'rush' OR map LIKE 'rush\_%' ESCAPE '\');

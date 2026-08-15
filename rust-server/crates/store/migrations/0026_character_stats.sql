-- The eight base stats a character has levelled into.
--
-- Stored rather than derived from the level, because the gain per level is a roll within a range:
-- two characters of the same class at the same level are not obliged to have the same numbers, and
-- recomputing from the level would flatten every character onto the average and undo every potion
-- ever drunk. Without this column a character's stats were reseeded from its class on every arrival,
-- which quietly discarded everything levelling had produced.
--
-- Empty means "not recorded yet", which reads as the class's starting values, so characters made
-- before this column keep working and are seeded the first time they are saved.

ALTER TABLE character ADD COLUMN IF NOT EXISTS stats integer[] NOT NULL DEFAULT '{}';

ALTER TABLE character ADD CONSTRAINT character_stats_shape
    CHECK (cardinality(stats) IN (0, 8));

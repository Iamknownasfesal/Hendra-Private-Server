-- A guild's level, which decides how large its hall is.
--
-- Raised by buying an upgrade from the hall merchant, so it is stored rather than derived: the
-- purchase is the event, and a level computed from fame would go back down when fame did.

ALTER TABLE guild ADD COLUMN IF NOT EXISTS level smallint NOT NULL DEFAULT 0;

ALTER TABLE guild ADD CONSTRAINT guild_level_in_range CHECK (level BETWEEN 0 AND 3);

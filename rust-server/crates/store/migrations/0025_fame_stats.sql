-- What a character has done, which is what its death is worth.
--
-- On the character rather than the account: the bonuses ask what *this* character did, and a
-- counter shared across characters would pay every one of them for the first one's dungeon runs.
--
-- Columns rather than a blob, because the questions asked of these are questions a query can
-- answer: how accurate was my best character, who has finished every dungeon. A blob would make
-- each of those a full scan and a decode.

ALTER TABLE character ADD COLUMN IF NOT EXISTS shots             integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN IF NOT EXISTS shots_that_hit    integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN IF NOT EXISTS abilities_used    integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN IF NOT EXISTS tiles_seen        integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN IF NOT EXISTS teleports         integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN IF NOT EXISTS potions_drunk     integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN IF NOT EXISTS monster_kills     integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN IF NOT EXISTS god_kills         integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN IF NOT EXISTS cube_kills        integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN IF NOT EXISTS oryx_kills        integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN IF NOT EXISTS quests_completed  integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN IF NOT EXISTS level_up_assists  integer NOT NULL DEFAULT 0;

-- Which kinds of dungeon have been finished, one bit each. A set rather than a count per kind,
-- because the only question asked is whether every kind has been done at least once.
ALTER TABLE character ADD COLUMN IF NOT EXISTS dungeons_completed bigint NOT NULL DEFAULT 0;

-- The bonuses a death earned, kept with the death so a graveyard can say why a number is what it
-- is. Text rather than a table: they are read together, written once, and never queried by name.
ALTER TABLE death ADD COLUMN IF NOT EXISTS bonuses text NOT NULL DEFAULT '';

-- What the rows that predate the stats column can still say about themselves.
--
-- `0026` added `stats` with a default of `{}`, so every character that already existed carries no
-- record of what levelling gave it. The row is not silent, though: `max_hp` and `max_mp` are two of
-- the eight stats and they are held in their own columns, so those two survive. They are written
-- into their slots here.
--
-- The remaining six are left NULL rather than filled with a number. The database has no idea what a
-- character's class starts at or gains per level — the class is a catalog identity, not a table —
-- and any number invented here would be indistinguishable from one the character earned. NULL says
-- "not recorded", which the server reads as an instruction to reconstruct that stat from the class
-- at the level the row records. A zero would say the character has no speed and no attack, and the
-- class's starting value would say a level-twenty wizard is a level-one wizard.

UPDATE character
SET stats = ARRAY[max_hp, max_mp, NULL, NULL, NULL, NULL, NULL, NULL]::integer[]
WHERE cardinality(stats) = 0;

-- A character wears its account's name, and never one of its own.
--
-- `realm/entities/player/Player.cs:423` is `Name = client.Account.Name;`, and `DbChar`
-- (`common/DbModels.cs:618-891`) has no name field at all: the nameplate, `/who`, the portrait and
-- the death notice are all reading one string, and it belongs to the account. A column here was a
-- second copy of that string, taken once at creation and never told when the account was renamed,
-- and every character created without one was stamped with the literal "Adventurer".
--
-- Dropping it is what makes the two names one again, for the rows that already exist as well as
-- the ones to come.

ALTER TABLE character DROP COLUMN IF EXISTS name;

-- The two potion stacks a character carries outside its inventory.
--
-- Health and magic potions do not occupy slots. They sit in a counter each, with a ceiling, which
-- is what lets a character carry forty of them into a dungeon without a bag full of nothing else.
--
-- Held on the character rather than in inventory_slot because they are not slots: there is no
-- index to move them between, and a move path that had to special-case two of them would be a
-- move path with two ways to be wrong.
ALTER TABLE character ADD COLUMN health_potions integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN magic_potions integer NOT NULL DEFAULT 0;

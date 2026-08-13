-- What an account can spend.
--
-- On the account rather than the character, because that is what the game does: a purchase made by
-- one character is paid for by every character, and a dead character does not take the gold with
-- it.
ALTER TABLE account ADD COLUMN gold integer NOT NULL DEFAULT 0;
ALTER TABLE account ADD COLUMN fame integer NOT NULL DEFAULT 0;
ALTER TABLE account ADD COLUMN tokens integer NOT NULL DEFAULT 0;

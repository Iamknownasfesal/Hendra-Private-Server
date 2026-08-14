-- Linking an account to a Discord user.
--
-- One account has at most one Discord id, and one Discord id belongs to at most one account: the
-- link exists so a bot can say who somebody is, and a link that pointed two ways would let one
-- person answer for another. Both directions are enforced by the schema rather than by the code
-- that writes it, because a check in the writer is a check that a second writer can race.

ALTER TABLE account ADD COLUMN IF NOT EXISTS discord_id text;

CREATE UNIQUE INDEX IF NOT EXISTS account_discord_id
    ON account (discord_id)
    WHERE discord_id IS NOT NULL;

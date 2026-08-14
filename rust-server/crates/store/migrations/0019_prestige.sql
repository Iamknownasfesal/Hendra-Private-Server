-- Prestige: what a character's fame becomes when it is given up.
--
-- Held on the account rather than the character, because that is the point of it: a character is
-- spent to earn it, and what is earned has to outlive what was spent.

ALTER TABLE account ADD COLUMN IF NOT EXISTS prestige integer NOT NULL DEFAULT 0;

-- What has ever been earned, which is never spent. Two numbers rather than one because a shop that
-- reduced a lifetime total would make the total mean nothing.
ALTER TABLE account ADD COLUMN IF NOT EXISTS total_prestige integer NOT NULL DEFAULT 0;

ALTER TABLE account ADD CONSTRAINT account_prestige_not_negative CHECK (prestige >= 0);

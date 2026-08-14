-- The graveyard: one row per character that has died, kept forever.
--
-- Separate from the character row even though `character.alive` already says a character is dead,
-- because the two answer different questions. The character says what it is now; this says what
-- happened, and when, and to what. A character can be deleted and its death still happened.
--
-- Written in the same transaction that marks the character dead, so there is no state where one is
-- true and the other is not.

CREATE TABLE IF NOT EXISTS death (
    id           bigserial PRIMARY KEY,

    account_id   bigint NOT NULL REFERENCES account (id) ON DELETE CASCADE,

    -- Not a foreign key on purpose. A character may be deleted, and the death it had is still a
    -- thing that happened: a graveyard that emptied itself when somebody tidied up would be no
    -- graveyard at all.
    character_id bigint NOT NULL,

    -- What it was, remembered here rather than looked up, because the class of a deleted character
    -- cannot be looked up afterwards. The class is an identity, as it is everywhere else: a runtime
    -- number means nothing once the content is reordered.
    name         text     NOT NULL,
    class        uuid     NOT NULL,
    level        smallint NOT NULL,

    -- What it finished with, after the bonuses. Held apart from the fame it had while alive, since
    -- that is the number the graveyard is ranked by.
    final_fame   integer  NOT NULL,

    killed_by    text     NOT NULL,

    -- Whether this was the first character on the account ever to die, which is a bonus that can
    -- only be earned once and so has to be remembered rather than recomputed.
    first_born   boolean  NOT NULL DEFAULT false,

    at           timestamptz NOT NULL DEFAULT now()
);

-- "My graveyard", which is what a player asks for.
CREATE INDEX IF NOT EXISTS death_by_account ON death (account_id, at DESC);

-- "The best deaths on the server", which is what a leaderboard asks for.
CREATE INDEX IF NOT EXISTS death_by_fame ON death (final_fame DESC, at DESC);

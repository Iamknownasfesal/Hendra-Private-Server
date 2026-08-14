-- The lists an account keeps about other people.
--
-- Two of them, and they do different jobs. Ignoring somebody hides what they say; locking somebody
-- stops them arriving where you are. The original keeps both, and keeping them in one table with a
-- kind rather than two tables means adding a third list later is a value rather than a migration.
--
-- One row per (owner, other, kind), enforced by the key rather than by the code that writes it: a
-- check in the writer is a check a second writer can race past, and a doubled ignore would be
-- removed once and stay in force.

CREATE TABLE IF NOT EXISTS account_list (
    owner_id  bigint    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    other_id  bigint    NOT NULL REFERENCES account (id) ON DELETE CASCADE,

    -- 0 ignored, 1 locked out.
    kind      smallint  NOT NULL,

    at        timestamptz NOT NULL DEFAULT now(),

    PRIMARY KEY (owner_id, other_id, kind),

    -- Listing yourself is never meaningful and would quietly hide your own messages.
    CONSTRAINT account_list_not_self CHECK (owner_id <> other_id)
);

-- "Is this person on my list" is the question every message and every teleport asks, so it is the
-- one that gets the index.
CREATE INDEX IF NOT EXISTS account_list_owner ON account_list (owner_id, kind);

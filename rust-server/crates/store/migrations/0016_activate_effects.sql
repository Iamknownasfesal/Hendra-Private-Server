-- What an ability changes that the world does not own.
--
-- A dye, a skin, a pet and a boost all outlive the room they were used in, which is why the world
-- hands them back rather than carrying them out. They had nowhere to go until now.

-- Two dye slots, matching the two the game has always had: the cloth and the accessory.
ALTER TABLE character ADD COLUMN dye_cloth integer NOT NULL DEFAULT 0;
ALTER TABLE character ADD COLUMN dye_accessory integer NOT NULL DEFAULT 0;

-- Which skin a character wears. Zero is the class's own appearance.
ALTER TABLE character ADD COLUMN skin integer NOT NULL DEFAULT 0;

-- A second row of carried slots, bought once and kept.
ALTER TABLE character ADD COLUMN has_backpack boolean NOT NULL DEFAULT false;

-- Pets, which belong to the account rather than a character: one bought by a wizard follows the
-- warrior too, and a death does not take it.
CREATE TABLE pet (
    id           bigserial PRIMARY KEY,
    account_id   bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    kind         uuid NOT NULL,
    skin         integer NOT NULL DEFAULT 0,

    -- Kept beyond the world it was summoned in.
    permanent    boolean NOT NULL DEFAULT false,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX pet_by_account ON pet (account_id);

-- Time-limited multipliers on what an account earns.
--
-- One row per running boost rather than a column each, so two of the same kind extend rather than
-- overwrite, and so a kind added later needs no migration.
CREATE TABLE account_boost (
    id           bigserial PRIMARY KEY,
    account_id   bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    kind         text NOT NULL,
    multiplier   real NOT NULL,
    expires_at   timestamptz NOT NULL
);

CREATE INDEX account_boost_live ON account_boost (account_id, expires_at);

-- Destinations an account has unlocked.
CREATE TABLE unlocked_portal (
    account_id   bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    portal       text NOT NULL,

    PRIMARY KEY (account_id, portal)
);

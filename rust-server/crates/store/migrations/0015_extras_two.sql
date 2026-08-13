-- The rest of what the old app server offered.
--
-- Grouped into one migration because they share a shape: small tables an operator fills, read by
-- endpoints that would otherwise have to be recompiled to change what they say.

-- Translatable strings, so a client can be localised without a client release.
CREATE TABLE language_string (
    language     text NOT NULL,
    key          text NOT NULL,
    value        text NOT NULL,

    PRIMARY KEY (language, key)
);

-- What an account may buy with real money. Prices live here rather than in the client, because a
-- client that decides its own prices is a client that decides its own prices.
CREATE TABLE credit_offer (
    id           bigserial PRIMARY KEY,
    name         text NOT NULL,
    credits      integer NOT NULL CHECK (credits > 0),
    price_cents  integer NOT NULL CHECK (price_cents >= 0),
    available    boolean NOT NULL DEFAULT true
);

-- Quests, and which of them an account has finished.
CREATE TABLE quest (
    id           bigserial PRIMARY KEY,
    key          text NOT NULL UNIQUE,
    title        text NOT NULL,
    goal         integer NOT NULL DEFAULT 1 CHECK (goal > 0),

    -- Weekly quests rotate; the rest are permanent.
    weekly       boolean NOT NULL DEFAULT false
);

CREATE TABLE quest_progress (
    account_id   bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    quest_id     bigint NOT NULL REFERENCES quest(id) ON DELETE CASCADE,
    progress     integer NOT NULL DEFAULT 0,
    finished_at  timestamptz,

    PRIMARY KEY (account_id, quest_id)
);

-- A picture an account uploaded, kept as bytes.
--
-- In the database rather than on disk because it is small, rarely read, and one fewer thing that
-- has to be backed up separately from everything it belongs to.
CREATE TABLE picture (
    account_id   bigint PRIMARY KEY REFERENCES account(id) ON DELETE CASCADE,
    kind         text NOT NULL,
    bytes        bytea NOT NULL,
    uploaded_at  timestamptz NOT NULL DEFAULT now()
);

-- Skins an account owns, and whether it has confirmed its age.
CREATE TABLE owned_skin (
    account_id   bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    skin         uuid NOT NULL,

    PRIMARY KEY (account_id, skin)
);

ALTER TABLE account ADD COLUMN age_verified boolean NOT NULL DEFAULT false;
ALTER TABLE account ADD COLUMN credits integer NOT NULL DEFAULT 0;

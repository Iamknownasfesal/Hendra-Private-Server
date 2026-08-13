-- The durable half of the server.
--
-- Nothing here is read during a tick. Live state — positions, health, what is in flight — lives in
-- memory owned by a world task, and this is touched at boundaries: login, logout, death, and the
-- periodic checkpoint. The exception is item movement, which is transactional precisely because it
-- is the one place where being wrong duplicates something.

CREATE TABLE account (
    id            bigserial PRIMARY KEY,
    name          text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),

    -- How many vault chests this account has paid for.
    vault_chests  smallint NOT NULL DEFAULT 4,

    banned        boolean NOT NULL DEFAULT false
);

-- Names are compared without regard to case, so `Fesal` and `fesal` cannot both exist. Enforced by
-- the index rather than by the code that inserts, because the code that inserts is not the only
-- code that ever will.
CREATE UNIQUE INDEX account_name_unique ON account (lower(name));

CREATE TABLE character (
    id            bigserial PRIMARY KEY,
    account_id    bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,

    object_type   integer NOT NULL,
    name          text NOT NULL,

    hp            integer NOT NULL,
    max_hp        integer NOT NULL,
    mp            integer NOT NULL DEFAULT 100,
    max_mp        integer NOT NULL DEFAULT 100,

    level         smallint NOT NULL DEFAULT 1,
    experience    integer NOT NULL DEFAULT 0,
    fame          integer NOT NULL DEFAULT 0,

    -- A dead character is kept rather than deleted: the graveyard is part of the game.
    alive         boolean NOT NULL DEFAULT true,

    created_at    timestamptz NOT NULL DEFAULT now(),
    last_seen     timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT character_hp_sane CHECK (hp <= max_hp AND max_hp > 0)
);

CREATE INDEX character_by_account ON character (account_id) WHERE alive;

-- One row per occupied slot rather than an array column.
--
-- An array would make a whole inventory one value, which sounds convenient until two slots have to
-- move atomically and the lock is the entire inventory. A row per slot means a move locks exactly
-- the two rows it touches.
CREATE TABLE inventory_slot (
    character_id  bigint NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    slot          smallint NOT NULL,
    item_type     integer NOT NULL,

    PRIMARY KEY (character_id, slot)
);

CREATE TABLE vault_slot (
    account_id    bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    slot          smallint NOT NULL,
    item_type     integer NOT NULL,

    PRIMARY KEY (account_id, slot)
);

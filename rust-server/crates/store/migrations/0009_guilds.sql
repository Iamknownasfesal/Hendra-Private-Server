-- Guilds, and who is in them.
--
-- Membership is a column on the account rather than a join table, because an account is in one
-- guild or none: a table would allow a state the game has no meaning for, and every read would
-- have to rule it out.
CREATE TABLE guild (
    id           bigserial PRIMARY KEY,
    name         text NOT NULL,
    board        text NOT NULL DEFAULT '',
    fame         integer NOT NULL DEFAULT 0,
    created_at   timestamptz NOT NULL DEFAULT now()
);

-- Guild names are unique regardless of case, so two guilds cannot be told apart only by shouting.
CREATE UNIQUE INDEX guild_name_unique ON guild (lower(name));

ALTER TABLE account ADD COLUMN guild_id bigint REFERENCES guild(id) ON DELETE SET NULL;

-- What a member may do. Higher ranks may do everything a lower one may.
ALTER TABLE account ADD COLUMN guild_rank smallint NOT NULL DEFAULT 0;

CREATE INDEX account_by_guild ON account (guild_id);

-- The gift chest: where something bought outside the world arrives.
--
-- A separate place from the vault, because a gift is not something the player put there, and mixing
-- the two would let a full vault refuse a purchase that has already been paid for.

CREATE TABLE IF NOT EXISTS gift_slot (
    account_id bigint   NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    slot       smallint NOT NULL,
    item       uuid     NOT NULL,
    at         timestamptz NOT NULL DEFAULT now(),

    PRIMARY KEY (account_id, slot)
);

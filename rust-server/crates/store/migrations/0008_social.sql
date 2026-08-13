-- Friends, and messages left for someone who is not here.
--
-- A friendship is one row per direction rather than one shared row. Two rows are what let each
-- side remove the other without deciding for them, and what makes "who are my friends" a lookup on
-- one column rather than a search on two.
CREATE TABLE friendship (
    account_id   bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    friend_id    bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    accepted     boolean NOT NULL DEFAULT false,
    created_at   timestamptz NOT NULL DEFAULT now(),

    PRIMARY KEY (account_id, friend_id),

    -- Befriending yourself is not a request anyone means to make.
    CHECK (account_id <> friend_id)
);

CREATE INDEX friendship_by_friend ON friendship (friend_id);

-- Messages waiting for someone who was not online to hear them.
CREATE TABLE private_message (
    id           bigserial PRIMARY KEY,
    from_id      bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    to_id        bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    body         text NOT NULL,
    sent_at      timestamptz NOT NULL DEFAULT now(),
    read_at      timestamptz
);

CREATE INDEX private_message_inbox ON private_message (to_id, read_at);

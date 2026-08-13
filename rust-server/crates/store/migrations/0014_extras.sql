-- News, and the daily calendar.
--
-- News is a table rather than a file so it can change without a deployment, which is the whole
-- reason a server has news at all.
CREATE TABLE news (
    id           bigserial PRIMARY KEY,
    title        text NOT NULL,
    body         text NOT NULL,
    posted_at    timestamptz NOT NULL DEFAULT now(),

    -- Shown in the client's own news panel rather than at the title screen.
    in_game      boolean NOT NULL DEFAULT false
);

CREATE INDEX news_recent ON news (in_game, posted_at DESC);

-- Which days an account has claimed.
--
-- One row per claim rather than a counter, because "have they claimed today" and "how many days in
-- a row" are different questions and a counter can only answer the second.
CREATE TABLE daily_claim (
    account_id   bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    claimed_on   date NOT NULL,

    PRIMARY KEY (account_id, claimed_on)
);

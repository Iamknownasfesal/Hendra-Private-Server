-- An address to reach an account at, and the tokens sent to it.
--
-- The address is nullable because an account can exist without one: requiring it at registration
-- would put a delivery failure between a player and their first character.
ALTER TABLE account ADD COLUMN email text;
ALTER TABLE account ADD COLUMN email_verified boolean NOT NULL DEFAULT false;

CREATE UNIQUE INDEX account_email_unique ON account (lower(email)) WHERE email IS NOT NULL;

-- One-time tokens, stored hashed.
--
-- Hashed for the same reason a password is: a leaked table must not be a set of working links. The
-- token itself exists only in the message that was sent.
CREATE TABLE email_token (
    token_hash   text PRIMARY KEY,
    account_id   bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,

    -- 'verify' or 'reset'. A token for one purpose must not work for the other.
    purpose      text NOT NULL,

    expires_at   timestamptz NOT NULL,
    used_at      timestamptz
);

CREATE INDEX email_token_by_account ON email_token (account_id, purpose);

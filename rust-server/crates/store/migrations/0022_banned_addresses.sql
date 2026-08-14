-- Addresses kept out, rather than accounts.
--
-- Separate from the account ban because it answers a different question: an account ban stops one
-- person playing, and an address ban stops whoever is behind it making another account. Held as
-- text rather than inet so an address family the server has not seen still stores.

CREATE TABLE IF NOT EXISTS banned_address (
    address text PRIMARY KEY,
    at      timestamptz NOT NULL DEFAULT now()
);

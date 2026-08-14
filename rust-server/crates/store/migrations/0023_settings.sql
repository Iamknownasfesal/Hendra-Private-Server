-- Settings an administrator changes without restarting, of which there is one today.
--
-- A table rather than a column on something, because none of these belong to an account or a
-- character: the welcome message belongs to the server.

CREATE TABLE IF NOT EXISTS setting (
    name  text PRIMARY KEY,
    value text NOT NULL,
    at    timestamptz NOT NULL DEFAULT now()
);

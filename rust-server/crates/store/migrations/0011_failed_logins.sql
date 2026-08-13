-- Failed login attempts, so the limit is the same however many app servers there are.
--
-- Per-process throttling means two servers behind a load balancer allow twice the attempts, and
-- four allow four times. The database is the one thing they share.
--
-- Keyed by the lowercased name rather than by account id, because a name that does not exist has
-- to be counted the same as one that does. Counting only real accounts would make the limiter
-- answer the question the login endpoint refuses to.
CREATE TABLE failed_login (
    name         text NOT NULL,
    at           timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX failed_login_recent ON failed_login (name, at);

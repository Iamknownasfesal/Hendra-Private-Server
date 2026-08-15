-- One account plays in one place at a time.
--
-- Everything durable here is written for a single writer per account. Two sessions on one account
-- are two checkpoints writing one character row and two hands in one vault, and the read-validate-
-- write discipline that makes a move safe stops being enough the moment there are two of them.
--
-- Durable rather than in-process, because the question is not "is this account playing on this
-- server" but "is it playing anywhere". A lock held in a process disappears with the process, and
-- says nothing about the one next to it.
--
-- The token is what makes releasing safe. A session releases only the lock it took: one that has
-- already been taken over must not be able to unlock the session that took over from it, and one
-- whose process died leaves a lock that simply expires.

CREATE TABLE IF NOT EXISTS account_lock (
    account_id bigint PRIMARY KEY REFERENCES account(id) ON DELETE CASCADE,

    -- Which session holds it. Renewing and releasing both name it.
    token uuid NOT NULL,

    -- When it stops counting. A row past this is a lock nobody holds: the process that took it is
    -- gone, and the account has to be playable again without an administrator.
    expires_at timestamptz NOT NULL
);

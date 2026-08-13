-- How far an account has taken each class.
--
-- A class unlocks by levelling the one before it, so the question "may this account make a
-- Knight" is "what is the best Warrior it has ever had". That cannot be answered from the
-- character table: characters are deleted, and deleting one must not take the unlock with it.
--
-- One row per account and class, holding the high-water mark rather than a history. The history
-- would be larger and answer no question anyone asks.
CREATE TABLE class_progress (
    account_id   bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    object_type  integer NOT NULL,
    best_level   smallint NOT NULL DEFAULT 0,
    best_fame    integer NOT NULL DEFAULT 0,
    updated_at   timestamptz NOT NULL DEFAULT now(),

    PRIMARY KEY (account_id, object_type)
);

-- Classes bought outright rather than earned.
--
-- Separate from the level a class was reached at, because they are different facts: one is a
-- purchase and the other is progress, and a refund should not silently erase levelling.
CREATE TABLE class_unlock (
    account_id   bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    object_type  integer NOT NULL,
    unlocked_at  timestamptz NOT NULL DEFAULT now(),

    PRIMARY KEY (account_id, object_type)
);

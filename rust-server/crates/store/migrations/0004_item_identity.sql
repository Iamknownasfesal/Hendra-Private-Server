-- Items are referred to by identity rather than by runtime number.
--
-- A runtime number is assigned at load and is stable only while nothing collides with it. Adding
-- content can move a probed one, and every row holding the old number would then name a different
-- item, or none. An identity never moves.
--
-- The columns are replaced rather than added alongside. Two ways to name the same item is two ways
-- for them to disagree, and there is no data here worth the ambiguity.
ALTER TABLE inventory_slot DROP COLUMN item_type;
ALTER TABLE inventory_slot ADD COLUMN item uuid;

ALTER TABLE vault_slot DROP COLUMN item_type;
ALTER TABLE vault_slot ADD COLUMN item uuid;

ALTER TABLE character DROP COLUMN object_type;
ALTER TABLE character ADD COLUMN class uuid NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000';

ALTER TABLE class_progress DROP COLUMN object_type;
ALTER TABLE class_progress ADD COLUMN class uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';

ALTER TABLE class_unlock DROP COLUMN object_type;
ALTER TABLE class_unlock ADD COLUMN class uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';

-- The primary keys named the dropped columns, so they are rebuilt around the identities.
ALTER TABLE class_progress ADD PRIMARY KEY (account_id, class);
ALTER TABLE class_unlock ADD PRIMARY KEY (account_id, class);

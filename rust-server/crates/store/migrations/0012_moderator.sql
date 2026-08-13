-- Who may moderate.
--
-- A rank rather than a flag, so "may mute" and "may ban" can differ without a second column, and
-- so raising someone to moderator is one change rather than several that can disagree.
ALTER TABLE account ADD COLUMN admin_rank smallint NOT NULL DEFAULT 0;

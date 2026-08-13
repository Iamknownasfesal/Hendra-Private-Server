-- When an account may speak again.
--
-- A time rather than a flag, because almost every mute is temporary and a flag needs a second
-- system to decide when to clear it. A mute that has expired is simply a time in the past.
ALTER TABLE account ADD COLUMN muted_until timestamptz;

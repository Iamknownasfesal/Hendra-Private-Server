-- Passwords, added after the fact.
--
-- Nullable rather than defaulted: an account created before authentication existed has no password
-- and should not be able to log in with a guessable one. Those accounts can set a password later,
-- and until they do the login path refuses them by name rather than by silently comparing against
-- something.
ALTER TABLE account ADD COLUMN password_hash text;

-- When the password was last changed, so a session minted before it can be refused.
ALTER TABLE account ADD COLUMN password_changed_at timestamptz;

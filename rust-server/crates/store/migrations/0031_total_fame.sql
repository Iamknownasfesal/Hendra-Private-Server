-- The fame an account has ever earned, alongside the fame it still has to spend.
--
-- `DbAccount` keeps `fame` and `totalFame` as two separate Redis fields and
-- `Database.UpdateFame` (`common/Database.cs:812-833`) writes both: every credit raises the
-- lifetime total, while a debit is applied only to the spendable balance because the increment on
-- `totalFame` is guarded by `if (amount > 0)`. The character list reports the two as
-- `<TotalFame>` and `<Fame>` (`server/XmlModels.cs:228-237`), and a wardrobe or a vault chest paid
-- for out of the second must not shrink the first.
ALTER TABLE account ADD COLUMN IF NOT EXISTS total_fame integer NOT NULL DEFAULT 0;

ALTER TABLE account ADD CONSTRAINT account_total_fame_not_negative CHECK (total_fame >= 0);

-- Existing accounts have no record of what they have already spent, so the lifetime total starts at
-- the larger of the balance they are holding and the fame their dead characters were worth. Both
-- are lower bounds on what was actually earned; picking the larger keeps the total from ever
-- reading below the balance beside it, which is the one thing the pair may never do.
--
-- The cap sits outside the `COALESCE` rather than inside it: `LEAST` skips nulls instead of
-- answering null, so an account with no deaths sums to null and `LEAST(null, 2147483647)` would
-- hand every such account the largest integer there is.
UPDATE account
SET total_fame = GREATEST(
        account.fame,
        LEAST(
            COALESCE((SELECT SUM(GREATEST(death.final_fame, 0))
                      FROM death WHERE death.account_id = account.id), 0),
            2147483647
        )::integer
    )
WHERE total_fame = 0;

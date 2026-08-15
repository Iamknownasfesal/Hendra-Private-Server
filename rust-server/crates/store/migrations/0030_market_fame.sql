-- What the original's market charges in, and what it remembers about the last thing listed.
--
-- A price of nothing is a price. `/market` reads its price with `^(\d+) (\d+)$`, so a negative one
-- cannot be typed at all, and `AddToMarket` refuses only `price < 0`: giving something away for
-- zero fame is allowed, and the check here was refusing it.
ALTER TABLE listing DROP CONSTRAINT listing_price_check;
ALTER TABLE listing ADD CONSTRAINT listing_price_check CHECK (price >= 0);

-- The id of the last listing this account created, kept after that listing is gone.
--
-- `DbAccount.LastMarketId` on the account rather than derived from what is still for sale, and the
-- difference is visible: taking back the newest listing is free, and once it is taken back the one
-- before it still costs the five fame fee. Reading "the newest still open" instead would make every
-- withdrawal free, one after another.
ALTER TABLE account ADD COLUMN last_market_id bigint NOT NULL DEFAULT 0;

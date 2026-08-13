-- Items listed for sale.
--
-- The listing holds the item, not the seller. When something is listed it leaves the seller's
-- inventory entirely: an item that existed in both places would be an item that could be sold and
-- kept, and no amount of care at the buying end can fix that.
--
-- Cancelling gives it back; buying gives it to the buyer. Exactly one of the two happens, which is
-- what the status column is for.
CREATE TABLE listing (
    id           bigserial PRIMARY KEY,
    seller_id    bigint NOT NULL REFERENCES account(id) ON DELETE CASCADE,
    item         uuid NOT NULL,
    currency     smallint NOT NULL,
    price        integer NOT NULL CHECK (price > 0),

    -- 'open', 'sold' or 'cancelled'. A listing leaves 'open' exactly once.
    status       text NOT NULL DEFAULT 'open',

    listed_at    timestamptz NOT NULL DEFAULT now(),
    closed_at    timestamptz
);

CREATE INDEX listing_open ON listing (status, item);
CREATE INDEX listing_by_seller ON listing (seller_id, status);

# `hendra-store`, against the persistence page

12 files, against [page 34](../mechanics/34-persistence.md) and
[page 39](../mechanics/39-fork-economy.md).

The backend is different by design — Postgres and sqlx where the original has Redis hashes, sets and
lists — so the comparison is about what the two guarantee, not about key names.

On that basis this crate is **stronger than the original everywhere it differs**, and the differences
are worth writing down so nobody "restores parity" by weakening them.

| | Original | Here |
| --- | --- | --- |
| Passwords | SHA-1, unsalted | Argon2 |
| Spending | `AddCondition` compare-and-set on a watched key | conditional `UPDATE ... WHERE balance >= price`, one statement |
| Lifetime totals | `total` fields only ever rise | `GREATEST` in the upsert, so two characters finishing at once cannot lower one |
| Item moves | read-validate-write | transactional, locking the rows it touches |
| Account locks | `lock:<id>` with a 60-second TTL and a GUID token | row-level, held for the transaction |

The conditional-spend property is the one that matters most and it is present: two requests cannot
both see enough prestige and both be granted, because the balance check and the deduction are the
same statement. Page 34 records the original reaching for the same guarantee by a different route.

## Bugs on pages 34 and 39 that are correctly absent

Checked by grep, and none of these exist here:

- **`SetGifts` returning `false` whenever handed a transaction** — so a gift written inside one is
  reported as failed and retried.
- **`MissedHitDetection`**, 40 missed hits in a refreshed 45-second window, which no longer bans and
  is left in place doing nothing.
- **`client.Player.Credits -= -75000`** — a subtraction of a negative, which grants credits where it
  reads as taking them.
- **The prestige shop that never flushes the account it debits**, so the spend is lost on the next
  load.
- **`if (buyAmount != 15 || buyAmount != 40)`** — always true, which makes the entire mark shop
  unreachable.

The last three are this fork's own additions rather than upstream's, and page 39 records the whole
economy around them as "mostly broken". None of it was carried over.

## A live secret that was not carried over

`wServer/discord/SendWebHook.cs` has a Discord webhook id and token in the source, committed. There
is no equivalent in this tree — `grep -rn "discord.com/api/webhooks" crates` finds nothing.

Worth stating positively rather than leaving implied, because the C# tree is still present in the
repository and that secret is still live in its history. Deleting the C# tree does not revoke it.

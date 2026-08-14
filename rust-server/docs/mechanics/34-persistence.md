# What is stored, and how

Read from `common/Database.cs`.

Everything is Redis. There are no tables and no schema: hashes keyed by name, sets for membership,
lists for ordering, and plain strings for counters and locks.

## The key space

```
logins                       hash  UPPER(uuid)      -> login blob
names                        hash  UPPER(ign)       -> account id
guilds                       hash  UPPER(guildName) -> guild id
nextAccId nextGuildId marketNextId    string counters
account.<id>                 hash  the account
guild.<id>                   hash  the guild
char.<accId>.<charId>        hash  one character
death.<accId>.<charId>       hash  one death record
alive.<accId>                set   char ids currently alive
dead.<accId>                 list  char ids, newest pushed left
classStats.<accId>           hash  per class best level / best fame
vault.<accId>                hash  vault.<n> -> that chest's items
lock:<accId>                 string  session lock, 60s TTL
dLock:<discordId>            string  the same lock by Discord id
regLock nameLock             string  global locks, 60s TTL
mutes:<ip>                   string  presence is the mute; TTL is the duration
ips                          the ip-ban table
missedHitDetections:<accId>  string  counter, 45s TTL
collectedTaxes legends market
```

Names, uuids and guild names are all keyed **upper-cased**, so they are case-insensitive and
case-preserving: the display form lives in the record, the key is the fold.

## Locking

One account lock, 60-second TTL, holding a fresh GUID as its value. Every operation that releases or
renews it is a **transaction conditioned on the value still being that GUID**, so a lock that expired
and was retaken by someone else is never released by the previous holder.

`AcquireLock` takes **two** keys when the account has a Discord id — `lock:<accId>` and
`dLock:<discordId>` — in one transaction conditioned on both being absent. That is what makes one
Discord account one session across renamed or alternate game accounts.

The lock is not renewed automatically. `RenewLock` exists and something has to call it; at 60 seconds
a session that stops renewing becomes stealable, which is exactly what `ConnectManager` relies on when
it reports "seconds until timeout".

Two global locks, `regLock` and `nameLock`, taken with the same pattern by string key.

## Compare-and-set, done by hand

There is no ORM. Concurrency is `AddCondition` on the value the caller last read:

```csharp
var currentAmount = GetCurrencyAmount(acc, currency);
trans.AddCondition(Condition.HashEqual(acc.Key, fields[1], currentAmount.Value));
trans.HashIncrementAsync(key, fields[1], amount);
```

Note this is a condition **plus an increment**, not a write: the condition makes it fail if the value
moved since the caller read it, and the increment is still atomic if it does not. The belt and braces
are deliberate, because the caller's in-memory `acc` object is also being updated from the result.

`SetGifts` uses the same shape on the raw gift byte string, which means **the entire gift list is one
value** and two concurrent gift additions cannot both succeed.

`SetGifts` returns `transaction == null && t.Execute()` — so when the caller **passes** a transaction,
it always returns `false` even on success, because the execution has not happened yet. Every caller
that passes a transaction and checks the result reads a failure. `Player.Market`'s removal path does
exactly this.

## Currency

```
Gold      -> totalCredits, credits
Fame      -> totalFame,    fame
GuildFame -> totalFame,    fame   (on guild.<id>)
Prestige  -> totalPrestige, prestige
```

The **total** field is only incremented when the amount is positive. So `total` is lifetime earned and
the other is the balance, and spending never reduces the lifetime figure. That is the whole
distinction, and it is why a leaderboard reads `total`.

`GuildFame` reads `guildId` off the account and **returns silently** if it is not positive — the caller
gets a completed task and no indication that nothing happened.

`UpdateCurrency(DbAccount, CurrencyType, ITransaction)` at the top of the file is
`throw new NotImplementedException()`. Dead overload.

## Characters

Creating one checks, in order: the alive set is smaller than `MaxCharSlot`; the skin is owned and
belongs to this class; and the class is either already unlocked, or its `Unlock` prerequisite class has
reached the required `BestLevel` and the descriptor is not `Restricted`.

Starting stats are `playerDesc.Stats[i].StartingValue` for the eight, unless `NewCharacters.Maxed` is
set, in which case every stat starts at `MaxValue`.

Inventory is the class's `Equipment` list resized to `InventorySize`, with the tail filled with
`0xffff`. `Utils.ResizeArray` does not fill, which is why the loop after it exists — and it starts at
`givenItems.Length`, so a class whose equipment list is *longer* than the inventory is truncated
silently.

`SaveCharacter` writes the character and the class stats in **one transaction**, optionally conditioned
on still holding the account lock. That is the only place the two are kept consistent with each other.

## Death

```
character.Dead = true
finalFame = stats.CalculateTotal(...)     see the fame page
character.FinalFame = finalFame
SaveCharacter(...)
write death.<accId>.<charId>
remove from alive.<accId>, push onto dead.<accId>
UpdateFame(account, finalFame)
if in a guild: UpdateGuildFame + UpdatePlayerGuildFame
```

The final fame is credited to **the account, the guild, and the player's personal guild contribution**,
all three, from the same number. `DeleteCharacter` removes from both the alive set and the dead list,
so a character can be purged from either state.

## The god-mode counter that no longer bans

```csharp
public Task<bool> MissedHitDetection(DbAccount acc, int misses)
{
    ... if (r.Result < 40) return false;
    Log.Warn($"[Missed Detection ...] Kicked.");
    //Ban(acc.AccountId, "Auto ban for use of god mode.");
    //BanIp(acc.IP, "Auto ban for use of god mode.");
```

40 missed hits inside a 45-second sliding window used to auto-ban and now only logs and returns true
for the caller to act on. The key's TTL is refreshed on **every** call, so the window is 45 seconds
since the last miss, not since the first — a steady trickle of misses never expires the counter.

This is the original's whole answer to godmode, and it is the reason
[the verification page](26-verification.md) exists.

## Bans and mutes

- **Bans** are a flag plus `notes` and `banLiftTime` on the account. `banLiftTime = -1` is permanent.
  Nothing here enforces the lift time; something else must read it.
- **IP bans** are a flag on `DbIpInfo`, a separate record per IP that also accumulates the set of
  account ids seen from that IP (`LogAccountByIp`). That set is how one ban reaches an alt.
- **Mutes** are the *existence* of `mutes:<ip>` with a TTL. There is no stored duration and no unmute
  operation: `/unmute` re-mutes for one second and lets the key expire, which fires the
  keyspace-expiry event that actually unmutes connected players.

## Wipes

`Wipe` and `WipeAccount` walk `1..nextAccId` and construct every account. `WipeAccount` kills every
alive character through the **real death path**, so a wipe still writes death records and still awards
fame before zeroing it — the order matters and is probably not intended.

`ResetFame` and `RemoveAllGold` are the same shape. All three are `O(accounts)` synchronous loops with
`_server.Keys(pattern:)` scans, which is a `KEYS` on a live server.

`WipeAccount` sets `fame` and `totalFame` to zero **twice**, and misses `guildFame`.

## Accounts

Guest names are a fixed list of 45, chosen by `(uint)uuid.GetHashCode() % 45` on registration and by
`accountId % 45` on un-naming — **two different derivations**, so unnaming an account usually changes
its guest name.

Passwords are `SHA1(password + salt)`, base64, with a 16-byte non-zero salt from
`RNGCryptoServiceProvider`. **SHA-1 with a single round** is the whole of it; ours must not copy that.

`Verify` and `CreateGuestAccount` both re-apply "all classes unlocked" and "all skins for rank >= 10"
on every login, so those settings take effect retroactively without a migration. `CreateGuestAccount`
deletes `classStats.0` when classes are not unlocked — a hard-coded account id 0 for the guest.

## The record layer

Read from `common/DbModels.cs`.

Every record is a `RedisObject`: the whole hash is read once into a dictionary of
`field -> (bytes, dirty)`, properties read and write that dictionary, and `FlushAsync` writes back
**only the dirty fields**. So an object is a snapshot plus a change set, and two objects on the same
key do not see each other's writes until one flushes and the other reloads.

`Reload(field)` re-reads one field, which is why `AddGifts` can call `acc.Reload("gifts")` without
throwing away everything else the caller has set.

Types are encoded by hand: ints and strings as UTF-8 text, bools as one byte, `DateTime` as
`ToBinary`, and `ushort[]`/`int[]` as a raw `BlockCopy`. So an item array is `2 * n` bytes with no
length prefix, and the length is inferred from the value's size.

**The dirty check is broken.** `SetValue` guards with `_entries.ContainsKey(Key)` — the *Redis key*,
not the field name it was passed:

```csharp
if (!_entries.ContainsKey(Key) || _entries[Key].Key == null || !buff.SequenceEqual(_entries[Key].Key))
    _entries[key] = new KeyValuePair<byte[], bool>(buff, true);
```

`Key` is `"account.5"`, never a field, so `ContainsKey` is always false and the write always happens.
The intended "skip a write that changes nothing" optimisation never fires. Harmless, and worth knowing
before anyone tries to explain a flush that writes everything.

`ReloadAsync` swallows every exception with a bare `catch {}` and leaves the old snapshot in place, so
a failed reload is indistinguishable from an unchanged record.

### The records

| Key | Holds |
| --- | --- |
| `account.<id>` | 40-odd fields; see below |
| `char.<acc>.<id>` | type, level, exp, fame, finalFame, items, hp, mp, stats[8], tex1/2, skin, fameStats, createTime, lastSeen, dead, hpPotCount, mpPotCount, hasBackpack, xpBoost, ldBoost, ltBoost |
| `death.<acc>.<id>` | objType, level, totalFame, killer, firstBorn, deathTime |
| `classStats.<acc>` | one JSON `{BestLevel, BestFame}` per class object type |
| `vault.<acc>` | `vault.<n>` -> that chest's `ushort[]` |
| `guild.<id>` | name, level, fame, totalFame, members[], allies[] (unimplemented), board |
| `ips` | hash of ip -> JSON `{Accounts, Banned, Alpha, Notes}` |
| `market` | hash of id -> 18 packed bytes |

**Rank is `max(DiscordRank, LegacyRank)`.** The Discord rank is read from a separate `discordRank`
hash at construction and is not stored on the account, so an account's effective rank can change
without its record changing.

**The ban lift is applied on read.** `DbAccount`'s constructor checks `banLiftTime`, and if it has
passed, clears `Banned` and flushes — so an expired ban is lifted by the next thing that loads the
account, not by any scheduled job. Loading with a `field` filter skips this entirely.

`Emotes` is a comma-separated string. `PrivateMessages` is JSON with a `NeedsFix()` migration path for
an older layout.

The three boost timers (`xpBoost`, `ldBoost`, `ltBoost`) and the two potion stack counts live on the
**character**, not the account, so they die with it.

`DbClassStats.Update` records `max(character.Fame, character.FinalFame)` — the live fame for a living
character and the bonused total for a dead one, which is why the First Born bar on
[the fame page](32-fame-bonuses.md) is measured against bonused totals.

`DbVaultSingle` **creates the chest if it does not exist**, writing eight `0xffff` in a fire-and-forget
transaction from its constructor. So merely constructing one is a write.

`RInventory.Items` defaults to **24** slots and `DbVault`'s indexer defaults to **8** — the same
underlying field read two different ways, which is the hangover the vault comments describe.

### The market's on-disk form

18 bytes, little-endian, unaligned:

```
0  id        uint32
4  itemId    uint16
6  price     int32
10 insertTime int32
14 accountId int32
```

`DbMarket` keeps the whole market in memory as a `List<PlayerShopItem>`, ordered by insert time at
load and appended to thereafter, and `Remove` is `List.Remove` by reference equality. Every query goes
to the list, not to Redis.

## What this server does differently

We are on Postgres with sqlx and real migrations, so most of this is structure rather than mechanics.
The parts that are mechanics:

- **`total` fields only rise.** Spending does not reduce lifetime earned, and leaderboards read the
  lifetime figure.
- **Death credits the same final fame to account, guild and personal guild contribution.**
- **The alive set and the dead list are separate**, and the dead list is ordered newest first.
- **Class unlock is by another class's best level**, checked at creation time, and `Restricted` blocks
  it outright.
- **A mute's storage is its expiry.** Ours should not store a duration it then has to poll.
- **Character creation truncates an over-long equipment list silently.** Ours should report it.

And the things to fix rather than copy: SHA-1 password hashing, `SetGifts` returning false whenever it
is handed a transaction, the two different guest-name derivations, and a 45-second window that is
refreshed rather than fixed.

# Forging, prestige, and the other currencies

Read from `networking/handlers/ForgeItemHandler.cs`, `PrestigeHandler.cs`, `PrestigeBuyHandler.cs`,
`SorForgeRequestHandler.cs`, `QoLActionHandler.cs`, `WeeklyQuestHandler.cs`.

**None of this is original RotMG.** It is this fork's own economy, and it is worth writing down
because it is what the players of this server actually use, and because most of it is broken in ways
that would be inherited silently.

## The currencies

| Currency | Stored | Earned |
| --- | --- | --- |
| Gold (`credits`) | account | purchases |
| Fame | account and character | dying, see [page 32](32-fame-bonuses.md) |
| Prestige | account | trading in 1,500 character fame at a time |
| Onrane | account | not read here |
| Sor fragments | account | not read here |

`Elite` accounts pay some prices in Onrane instead of Gold.

## Forging

Three recipes, dispatched on what is in the first slot.

### Sor Crystal + shard -> a random legendary from that shard's family

```
CosmicShard   -> one of 55
FuryShard     -> one of 19
ZolShard      -> one of 11
StoneShard    -> one of 4
AncientShard  -> one of 6
NecroSword    -> Nemesis            (fixed)
HunterNeck    -> Predator Neck      (fixed)
WarpedWorlds  -> Dreamweaver        (fixed)
EmpWhip       -> Abyssal Whip       (fixed)
```

Uniform choice with a fresh `new Random()` **per call**, which on .NET Framework is seeded from the
system clock — two forges in the same tick get the same item.

The result replaces the crystal and the shard is cleared. Success is announced to the whole guild, and
to everyone in the world who is **not** in that guild.

### Shine + legendary -> reroll into any legendary

Costs **75,000 gold**, or **30 Onrane** for an `Elite` account. Eleven items are excluded from
rerolling (`_rerollExclude`): the shards themselves, the Shine, the Sor Crystal, and the four
fixed-recipe results.

```csharp
client.Player.Manager.Database.UpdateCredit(client.Account, -75000);
client.Player.Credits -= -75000;
```

The second line **adds** 75,000 to the number the client is shown while the database is charged
75,000. The player sees their gold go up by 75k until something reloads it.

### Two of the same potion -> the next tier up

Keyed on `slotA.ObjectType * slotB.ObjectType`, so it fires when both slots hold the **same** item —
and also, in principle, for any other pair whose product collides.

```
two Life potions      -> Greater Life
... the eight stats, plus Might, Luck, Resistance, Protection
two Greaters          -> the matching Vial, and costs 1 Onrane
```

Vials are only reachable when the account has at least one Onrane, and the Onrane is deducted **even
when the pair does not match any vial recipe** — the `UpdateOnrane(-1)` sits outside the switch.

### What is not checked

The handler reads `client.Player.Inventory[packet.SorSlot.SlotId]` before checking anything, so an
**out-of-range or empty slot throws** and the connection is dropped by the packet dispatcher. The only
validation is that each slot's current object type matches what the packet claims — which stops a
stale packet, and nothing else. There is no check that the two slots differ.

## Prestige

```
1,500 character fame  ->  1 prestige point
```

`PrestigeRequest` loops, taking 1,500 at a time, then resets the character completely: experience 0,
level 1, fame 0, every fame statistic zeroed, and all eight base stats back to the class's
`StartingValue`. **Equipment is untouched**, so a prestiged character keeps its gear at level 1.

Three faults:

- **Fame of exactly 1,499 does nothing.** The guard is `< 1499` to refuse and `>= 1500` to act, so
  1,499 falls between and the request is silently ignored.
- The account's prestige is written **but the character save happens before the disconnect and the
  updates after it** — `client.Disconnect()` is called and then four more `ForceUpdate` calls run
  against a disconnected client.
- `SaveToCharacter` is called three times and `CalculateFame` four, in a sequence that reads as
  someone adding calls until it worked.

## The prestige shop

```
BuyId 1  ->  50 prestige
BuyId 2  ->  150 prestige
BuyId 3  ->  150 prestige
BuyId 4  ->  150 prestige
```

The item is delivered as a **gift**, so it arrives in the gift chest.

The cost is checked against `Account.Prestige` and subtracted from `Player.Prestige`, then the account
is assigned from the player — and **nothing flushes the account**. The spend is a dirty field that is
never written, so it survives only until the next reload. In practice the items are free.

`BuyId` 5 through 8 pass the range check at the top and then fall into the trailing `else`, so they
error rather than doing anything.

## Raids

`LaunchRaid` needs **20 stars**, **10,000 gold**, and no raid already running
(`Manager._isRaidLaunched`, a flag on the manager, so it is server-wide).

Two raids, each with an Ultra variant, each launching a portal at the fixed position **(149, 114)**
marked `PlayerOpened` with the launcher as `Opener`. Two timers are set: the portal is removed after
the portal descriptor's own `Timeout` seconds, and `_isRaidLaunched` is cleared after a flat **60
seconds** — so a second raid can be launched long before the first portal closes.

The launch is announced server-wide through the cross-server chat bus and shown as a green
notification locally.

```csharp
player.Client.Manager.Database.UpdateCredit(player.Client.Account, -gold);
player.Credits = player.Client.Account.Credits - gold;
```

The database call already decremented the account; reading it and subtracting `gold` again shows the
player **20,000 gold poorer than they are**.

The token check reads `if (player.startRaid1(player) == false) { launch } else { "You need the
correct token" }`, so the method returns *true* when the token is missing — a name that says the
opposite of what it does.

## Alerts

`AlertNotice` requires an alert token and 1,000 gold, spends **only the token**, and then picks one of
four worlds uniformly:

```
KrakenLair  TheHollows  HiddenTempleBoss  FrozenIsland
```

An 8-second timer then reconnects the player into it. The gold check is made and the gold is never
taken.

## Marks and nodes are unreachable

```csharp
if (buyAmount != 15 || buyAmount != 40) {
    player.SendError("Inproper purchase cost.");
    return;
}
```

No number is both 15 and 40, so the condition is **always true**. Every mark and node purchase errors
out before anything else runs. The eleven node ids (15 Onrane) and seven mark ids (40 Onrane) below it
are dead code, as is `NodeSet`'s four-slot fill.

## Lootboxes

```
1 Bronze                 free
2 Silver                 free
3 Gold                   free
4 Elite   + 5 Onrane
5 (Kantos box)  600 Kantos
```

Each spends its own counter, then calls `player.Unbox(type)`. Case 5 has no box counter at all — the
currency *is* the key.

## The rest

- **`SorForgeRequest`**: 20 Onrane to ascend a Sor Crystal.
- **`QoLAction` id 1**: 30 Sor fragments become one Sor Crystal, delivered as a gift. Any other id is
  "Inproper action ID."
- **`WeeklyQuestRedeem`**: the handler body is **empty**. The quest data loads from `quests.xml`
  (`common/resources/WeeklyQuest.cs`, with a per-quest `maxQuestDone` cap enforced as a Redis
  transaction condition), and nothing in the world server ever redeems one.

## What this server does differently

We have none of this, and adding it is a product decision rather than a fidelity one. If it is added:

- **Forging is a table plus a currency check**, and the table is the interesting part — it should live
  in content, not in a switch statement full of hex constants.
- **Prestige resets base stats and level but keeps equipment.** That is the actual rule.
- Fix the boundary at 1,499, charge the currency once and only on success, flush what you charge, and
  never touch a client after disconnecting it.
- **Do not seed a fresh RNG per call.** Two forges in the same millisecond returning the same item is
  the kind of thing players notice and exploit.

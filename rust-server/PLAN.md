# Finishing the server

Everything left between here and a server that plays the whole game, with the client cutover
deliberately excluded and left until last.

Phases 0–5 are done: workspace, content, protocol, transport, simulation, behaviour language,
persistence, authentication. What follows is phases 6–11.

**Where we are:** ~57% of the old server by subsystem. 455 tests. The parts that were hard —
transport, simulation, behaviours, dupe-safety, content — are done and better than what they
replace. What remains is mostly breadth.

**Sizing** is relative, not calendar: **S** is an afternoon, **M** is a day or two, **L** is
several days, **XL** is a week or more.

---

## A correction, and what it taught

An earlier version of this plan said ~60% and quoted 629 enemies and 3,457 behaviour uses. Both
were wrong. Generating the content files to look at exposed four bugs in the C# reader — hex
literals, nested static calls, arithmetic, and preprocessor directives — each of which drops a
whole enemy rather than one argument. The real corpus is **748 enemies and 8,631 behaviour uses**.
The measurement had been taken against a corpus missing 42% of its content.

They were dropped in silence, which is the part worth carrying forward: the count looked exactly
like success. Every phase below that claims a percentage should be able to say where the
denominator came from, and every converter should report what it could not read.

---

## Ordering, and why

Phase 6 comes first because it is the cheapest work with the largest effect. The behaviour language
is at 100% of the content's 8,631 uses, but much of what those behaviours ask for currently lands
nowhere: `conditional_effect(invulnerable)` is the most-used behaviour in the game and right now it
marks a boss invulnerable while leaving it perfectly killable. A few days of work makes months of
already-built work real.

Phase 7 follows because items are what a player touches every minute. Phase 8 is what makes the
world a game rather than a room. Phase 9 is independent of all of it and blocks nothing, so it goes
late. Phase 10 runs alongside anything. Phase 11 is the end.

---

## Phase 6 — Make the simulation mean what it says

The theme: things that are *recorded* but not *acted on*.

### 6.1 Condition effects act — **M** — **DONE**

Effects were applied, expired, and went out in snapshots, and nothing read them.

`crates/sim/src/effects.rs` derives a `Rules` struct from a `ConditionSet` once per use, so the hot
paths multiply by one number rather than testing eight bits. Immunities refuse an effect at
`give_effect` rather than letting it land and be ignored. Five mutations confirm the tests bite.

One correction to what this plan said: **ground damage was already implemented and tested** —
`apply_hazards` has applied it all along, and `standing_in_lava_costs_hit_points_and_eventually_kills`
covers it. 8.5 is smaller than stated.

| Effect | What it must do |
|---|---|
| Invulnerable, Invincible | Refuse damage in `projectile.rs::can_hit` and `world.rs::explode` |
| Armored, ArmorBroken | Scale defence in `after_defence` |
| Paralyzed, Petrify, Stasis | Refuse movement in `resolve_move` and `Action::Move` |
| Slowed, Speedy, NinjaSpeedy | Scale the movement multiplier |
| Quiet | Refuse ability use (once 7.1 lands) |
| Sick | Refuse healing |
| Bleeding, Healing | Per-second hp change in `apply_hazards` |
| Dazed | Scale weapon cooldown |
| Stunned | Refuse shooting |
| Weak, Damaging, Berserk | Scale outgoing damage |
| Confused, Drunk, Hallucinating, Blind, Darkness | Client-side only — send, do nothing |
| Curse, Hexed | Scale incoming damage |
| Invisible | Drop from other players' snapshots |
| Paused | Skip the entity in `think` and `advance` |
| \*Immune | Refuse the matching effect in `give_effect` |

### 6.2 The stat system — **M**

Eight stats exist in the content and nothing reads them. `PlayerDesc` already carries base, maximum
and per-level growth from phase 5's class work.

- `Stats` on `Entity`: the eight base values, plus the equipment and boost layers kept separately
  so a ring coming off subtracts exactly what it added.
- Derived: damage from Attack, defence from Defense, movement from Speed, weapon cooldown from
  Dexterity, hp/mp regeneration from Vitality/Wisdom.
- `StatBoost` is parsed in the catalog and never read — a +6 DEX ring is currently decoration.

**Verify:** equipping and unequipping a ring returns every stat to exactly its starting value over
a thousand random cycles. Two rings stack; the same ring twice does not double-count.

### 6.3 Experience, levels and fame — **M**

`level` and `experience` columns exist and never change.

- XP on kill, split by damage contribution, scaled by `exp_multiplier` and level difference.
  `no_experience` (already on `Entity`) suppresses it, so summons cannot be farmed.
- Level-up rolls each stat within its class's `min_increase..max_increase`, capped at the maximum.
- Fame from the same event.
- **Call `hendra_characters::record_progress`** — it exists, is tested, and nothing calls it, so
  class unlocks can never advance through play. One line at the save path.

**Verify:** a character that kills its way to level 20 has stats inside its class's declared bounds
and never above the maximum. Killing a warrior to 20 unlocks the knight through the app server, end
to end.

### 6.4 Loose ends — **S**

- `toss_object`'s telegraph is parsed and ignored, so thrown attacks land instantly and are
  unavoidable rather than dodgeable. Needs a delayed spawn queue on `World`.
- `take_announcements` and `take_ground_changes` exist, are bounded, and nothing calls them —
  bosses are talking to a queue. Needs two server messages and a drain in `world_task`.
- `App::new` builds an empty catalog, so an app server built that way silently offers zero classes.
  The binary uses `with_content`, so production is fine; the easy constructor is the wrong one.
- `apply_setpiece` compiles to a ground transform, which is **wrong rather than missing** — a
  setpiece stamps a prefab map, it does not paint a circle. Fix or make it unsupported until 8.3.
- 9 dangling transitions and 18 shadowed state names are bugs in the original content. They are
  reported at conversion; decide whether to fix them in the `.beh` files now that those are the
  source of truth.

**Exit criteria:** every behaviour the content uses has an observable effect on the world. Nothing
is recorded and ignored.

---

## Phase 7 — Items and abilities

`Player.UseItem.cs` is 1,390 lines and the largest single unported file. Attack it the way the
behaviours were attacked: count what the content actually uses, rank, implement, measure.

**Measured with `cargo run --release -p hendra-content --example activates`** — 1,609 items, 1,393
of them with an ability, 1,942 activates across 43 distinct kinds:

```
 690  IncrementStat          35.5%      26  StatBoostSelf          86.9%
 470  Dye                    59.7%      24  Pet                    88.1%
 191  UnlockSkin             69.6%      23  Shoot                  89.3%
 144  CreatePet              77.0%      20  Magic                  90.3%
  56  ConditionEffectSelf    79.9%      18  ConditionEffectAura    91.2%
  55  Create                 82.7%      14  Token                  92.0%
  29  Heal                   84.2%      13  BulletNova / HealNova  93.3%
  26  GenericActivate        85.5%      … 25 more to 100%
```

The shape of that curve decides the order. Six kinds reach 83%, and three of those six are
cosmetic.

### 7.1 The activate framework — **M**

Mirror the behaviour crate: `ActivateDesc` (already parsed) compiles to an `Effect` enum, runs
against a small `UseContext`, and produces the same `Action` set the behaviours emit. Reuse is the
point — `Heal`, `ConditionEffect`, `Shoot` and `Create` already have world-side implementations
from phase 6 and the behaviour work.

Add a `use_item` client message with the MP cost, cooldown and `Quiet` checks.

### 7.2 The effects — **L**

In ranked order. `IncrementStat` alone is 35% and depends entirely on 6.2, which is why the stat
system comes first. `Dye` and `UnlockSkin` are 30% between them and need only storage plus a
snapshot field — cheap, and they clear a third of the list.

`CreatePet`/`Pet`/`PermaPet`/`PetSkin` is 9% and a subsystem of its own; defer it behind the rest
without blocking anything.

**Verify:** `activates.rs` reports the percentage implemented. Target 100%, same as behaviours.

### 7.3 Stacking, consumables and bags — **S/M**

`ItemStacker.cs` is 63 lines. Potions stack to a limit, consumables decrement, bag types decide
which colour bag loot drops in. Soulbound is already refused by the store's move path.

### 7.4 Vendors and gift chests — **M**

842 lines of vendor logic. Needs currency (gold, fame, tokens) on the account and a purchase path
with the same row-locking discipline the item moves already use.

**Exit criteria:** every activate the content uses does something, and a player can equip, use, buy
and stack items.

---

## Phase 8 — The realm

### 8.1 Oryx and the realm event manager — **L**

899 lines. Populates the realm by terrain and probability, counts enemies, announces events, closes
the realm, spawns the castle. `spawn_probability`, `per_realm_max` and `terrain` are already parsed
and unused.

### 8.2 Portals and dungeon lifecycle — **M**

`PortalMonitor.cs` is 248 lines. Portals opened by a boss death already work; what is missing is
the lifetime, the player count, and closing a dungeon when it empties. `Worlds::get_or_start_for`
already gives personal instances.

### 8.3 Setpieces — **M**

Needs a setpiece format (reuse HMAP) and a stamp operation. Fixes the wrong `apply_setpiece` from
6.4.

### 8.4 The remaining entity kinds — **M**

Decoy, Trap, Sign, GiftChest, ConnectedObject, Wall, GuildHallPortal. Mostly small and independent;
several fall out of 7.2.

### 8.5 Terrain on the wire — **M**

Ground damage already works — that was a mistake in an earlier draft of this plan. What is missing
is that terrain never reaches clients at all, which is server-side work the client cutover depends
on and why it belongs here.

**Exit criteria:** a realm populates, a dungeon opens and closes, and standing in lava hurts.

---

## Phase 9 — Social

Independent of everything above and blocks nothing, which is why it is late rather than early.

- **9.1 Chat — M.** 380 lines. Chat currently routes to the world and stops there. Needs
  say/tell/guild/global scoping, rate limiting, a mute list, moderation commands.
- **9.2 Guilds — L.** Schema, ranks, hall worlds, the board.
- **9.3 Friends and private messages — M.**
- **9.4 Market — L.** 450 lines. Every listing is an item move, so it must go through the same
  row-locking discipline the trade path proved — the single most dupe-prone thing left to build.

**Exit criteria:** two players can talk, form a guild, and trade through the market without an item
ever existing twice.

---

## Phase 10 — Finish the app server

Eight of 48 endpoints are done, and they are the ones that matter. The remaining forty:

1. **Needed to play:** `app/init`, `app/getServerXmls`, a server list (a client has no way to learn
   where the game server is), `char/list` extensions, `account/purchaseCharSlot`
2. **Expected:** `account/verify`, `sendVerifyEmail`, `forgotPassword`, `resetPassword`, `setName`,
   `char/fame`, `fame/list`
3. **Optional:** Discord, credits, daily calendar, weekly quests, pictures, global news

**M for the first group, L for the rest.**

Also here: rate limiting is per-process, so two app servers behind a load balancer means double the
limit. Fine at one instance, wrong at four — the fix is a `failed_login` table or Redis.

---

## Phase 11 — Retire the C# tree

Only once every phase above is done and the client has been cut over.

- Delete `Server-Side/`, keeping `XmlDatas` until the content pipeline needs nothing from it.
- **`Server-Side/server/server.json` carries the production IP `37.123.96.189`.** Scrub it before
  the repository goes anywhere public — this should not wait for phase 11 if publication is on the
  table sooner.
- Move deployment, the Dockerfile and CI to the Rust tree.

---

## Cross-cutting, throughout

**Protocol.** Each phase adds messages. Add them as needed rather than designing them up front —
the client is not listening yet, so changing one costs nothing today, and that window is worth
using.

**The verification discipline stays.** Every protection gets a mutation check: break it, watch
exactly the right test fail, restore it. Fourteen of these have caught real bugs, several after the
work looked finished. It is not ceremony.

**Silence is the enemy.** Every converter and loader reports what it could not handle. The four
parser bugs cost 119 enemies and went unnoticed because the total looked like success.

**The tick budget is a constraint.** 23× headroom at 200 players today. Run
`cargo run --release -p hendra-sim --example tick_budget` after any phase that touches the loop.

**Coverage examples earn their place.** `behavior/examples/gaps.rs` and `content/examples/activates.rs`
turn "implement the things" into a number. Phase 10 should have one for endpoints.

---

## What this adds up to

| Phase | Content | Size |
|---|---|---|
| 6 | Effects, stats, levelling | **M–L** |
| 7 | Items and abilities | **L–XL** |
| 8 | The realm | **L–XL** |
| 9 | Social | **XL** |
| 10 | App server | **M–L** |
| 11 | Retirement | **S** |

Phase 6 is next and has the best return. After it the server is roughly 65% and — more importantly
— everything it claims to do, it does.

The client cutover is not in this plan by request. It remains the only thing between all of this
and a game anyone can play.

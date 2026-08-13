# Finishing the server

Everything left between here and a server that plays the whole game, with the client cutover
deliberately excluded and left until last.

Phases 0–5 are done: workspace, content, protocol, transport, simulation, behaviour language,
persistence, authentication. What follows is phases 6–11.

**Where we are:** ~60% of the old server by subsystem. 429 tests. The parts that were hard —
transport, simulation, behaviours, dupe-safety, content — are done and better than what they
replace. What remains is mostly breadth.

**Sizing** is relative, not calendar: **S** is an afternoon, **M** is a day or two, **L** is
several days, **XL** is a week or more.

---

## Ordering, and why

Phase 6 comes first because it is the cheapest work with the largest effect. The behaviour
language is already at 100% of the content's 3,457 uses, but a third of what those behaviours ask
for currently lands nowhere: `conditional_effect(invulnerable)` is the single most-used behaviour
in the game and right now it marks a boss invulnerable while leaving it perfectly killable. A few
days of work makes months of already-built work real.

Phase 7 follows because items are what a player touches every minute. Phase 8 is what makes the
world a game rather than a room. Phase 9 is independent of all of it and blocks nothing, so it
goes late. Phase 10 runs alongside anything. Phase 11 is the end.

---

## Phase 6 — Make the simulation mean what it says

The theme: things that are *recorded* but not *acted on*.

### 6.1 Condition effects act — **M**

Effects are applied, expire, and go out in snapshots. Nothing reads them.

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
| Curse, Hexed | Scale incoming damage / force a shape |
| Invisible | Drop from other players' snapshots |
| Paused | Skip the entity in `think` and `advance` |
| \*Immune | Refuse the matching effect in `give_effect` |

One `EffectRules` struct derived from a `ConditionSet` once per entity per tick, so the hot paths
ask a precomputed struct rather than testing bits repeatedly.

**Verify:** a boss with `conditional_effect(invulnerable)` survives a hundred shots and dies to the
hundred-and-first once the state ends. A paralysed player cannot move. Mutation-check each.

**Files:** `crates/sim/src/effects.rs` (new), `projectile.rs`, `world.rs`

### 6.2 The stat system — **M**

Eight stats exist in the content and nothing reads them. `PlayerDesc` already carries base, max and
per-level growth from phase 5's class work.

- `Stats` on `Entity`: the eight base values, plus the equipment and boost layers kept separately
  so a ring coming off subtracts exactly what it added.
- Derived values: damage multiplier from Attack, defence from Defense, movement from Speed, weapon
  cooldown from Dexterity, hp/mp regeneration from Vitality/Wisdom.
- `StatBoost` is parsed in the catalog and never read — a +6 DEX ring is currently decoration.

**Verify:** equipping and unequipping a ring returns every stat to exactly its starting value, over
a thousand random equip/unequip cycles. Two rings stack; the same ring twice does not double-count.

**Files:** `crates/sim/src/stats.rs` (new), `crates/server/src/session.rs`

### 6.3 Experience, levels and fame — **M**

`level` and `experience` columns exist and never change.

- XP on kill, split by damage contribution, scaled by the enemy's `exp_multiplier` and level
  difference. `no_experience` (already on `Entity`) suppresses it, so summons cannot be farmed.
- Level-up rolls each stat within its class's `min_increase..max_increase`, capped at the maximum.
- Fame from the same event, by the game's formula.
- **Call `hendra_characters::record_progress`** — it exists, is tested, and nothing calls it, so
  class unlocks can never advance through play. One line at the save path.

**Verify:** a character that kills its way to level 20 has stats inside the class's declared
bounds and never above the maximum. Killing a warrior to 20 unlocks the knight through the app
server, end to end.

**Files:** `crates/sim/src/world.rs`, `crates/server/src/world_task.rs`, `crates/store/src/model.rs`

### 6.4 Loose ends from phase 6's own work — **S**

- `toss_object`'s telegraph is parsed and ignored, so thrown attacks land instantly and are
  unavoidable rather than dodgeable. Needs a delayed spawn queue on `World`.
- `take_announcements` and `take_ground_changes` exist, are bounded, and nothing calls them —
  bosses are talking to a queue. Needs two server messages and a drain in `world_task`.
- `App::new` builds an empty catalog, so an app server constructed that way silently offers zero
  classes. The binary uses `with_content` and exits on a bad directory, so production is fine, but
  the easy constructor is the wrong one.

**Exit criteria for phase 6:** every behaviour the content uses has an observable effect on the
world. Nothing is recorded and ignored.

---

## Phase 7 — Items and abilities

`Player.UseItem.cs` is 1,390 lines and the largest single unported file. Attack it the way the
behaviours were attacked: count what the content actually uses, rank, implement, measure.

**Measured usage across the content** (the ranking that should drive the work):

```
 722  IncrementStat        56  ConditionEffectSelf     13  HealNova
 470  Dye                  55  Create                  13  BulletNova
 191  UnlockSkin           29  Heal                    11  VampireBlast
 144  CreatePet            26  StatBoostSelf           11  PoisonGrenade
  26  GenericActivate      24  Pet                     10  Trap / Teleport
  23  Shoot                20  Magic                   10  StatBoostAura
  18  ConditionEffectAura  14  Token                   10  StasisBlast
                                                       10  Decoy / Lightning
```

### 7.1 The activate framework — **M**

Mirror the behaviour crate's shape: `ActivateDesc` (already parsed) compiles to an `Effect` enum,
runs against a small `UseContext`, and produces the same `Action` set the behaviours emit. Reuse is
the point — `Heal`, `ConditionEffect`, `Shoot` and `Create` already have world-side implementations
from phase 6 and the behaviour work.

Add a `use_item` client message and the MP cost, cooldown and `Quiet` checks that go with it.

### 7.2 The effects themselves — **L**

Ranked as above. The first six cover roughly 80% of uses. `Dye` and `UnlockSkin` are cosmetic and
need only storage plus a snapshot field. `CreatePet`/`Pet` is a subsystem of its own and can be
deferred behind the rest without blocking anything.

**Verify:** a coverage example like `crates/behavior/examples/gaps.rs`, reporting the percentage of
activate uses implemented. Target 100%, same as behaviours.

### 7.3 Stacking, consumables and bags — **S/M**

`ItemStacker.cs` is 63 lines. Potions stack to a limit, consumables decrement, bag types decide
which colour bag loot drops in. Soulbound is already refused by the store's move path.

### 7.4 Vendors and gift chests — **M**

842 lines of vendor logic. Needs currency (gold, fame, tokens) on the account, a purchase path with
the same row-locking discipline the item moves already use, and the merchant entity kinds.

**Exit criteria for phase 7:** every activate the content uses does something, and a player can
equip, use, buy and stack items.

---

## Phase 8 — The realm

### 8.1 Oryx and the realm event manager — **L**

899 lines. Populates the realm with enemies by terrain and probability, counts them, announces
events, closes the realm, spawns the castle. `spawn_probability`, `per_realm_max` and `terrain` are
already parsed in the catalog and unused.

### 8.2 Portals and dungeon lifecycle — **M**

`PortalMonitor.cs` is 248 lines. Portals opened by a boss death already work (phase's behaviour
work); what is missing is the lifetime, the player count, and closing a dungeon when it empties.
`Worlds::get_or_start_for` already gives personal instances.

### 8.3 Setpieces — **M**

`apply_setpiece` compiles to a ground transform today, which is wrong — a setpiece stamps a small
prefab map into the world. Needs a setpiece format (reuse HMAP) and a stamp operation.

### 8.4 The remaining entity kinds — **M**

Decoy, Trap, Sign, GiftChest, ConnectedObject, Wall, GuildHallPortal. Mostly small and
independent; several fall out of phase 7's activate work.

### 8.5 Ground damage and terrain on the wire — **M**

`min_damage`/`max_damage` on tiles are parsed and unapplied, so lava is currently decorative.
Terrain never reaches clients at all — this is server-side work that the client cutover depends on,
which is why it belongs here rather than in the client phase.

**Exit criteria for phase 8:** a realm populates, a dungeon opens and closes, and standing in lava
hurts.

---

## Phase 9 — Social

Independent of everything above and blocks nothing, which is why it is late rather than early.

- **9.1 Chat — M.** `ChatManager.cs` is 380 lines. Chat currently routes to the world and stops
  there. Needs say/tell/guild/global scoping, rate limiting, a mute list, and the moderation
  commands.
- **9.2 Guilds — L.** Schema, ranks, hall worlds, the board. Seven packet types in the old
  protocol.
- **9.3 Friends and private messages — M.**
- **9.4 Market — L.** `Market.cs` is 450 lines. Listings, escrow, fees. Every listing is an item
  move, so it must go through the same row-locking discipline that the trade path proved — this is
  the single most dupe-prone thing left to build.

**Exit criteria for phase 9:** two players can talk, form a guild, and trade through the market
without an item ever existing twice.

---

## Phase 10 — Finish the app server

Eight of 48 endpoints are done, and they are the ones that matter. The remaining forty in rough
priority:

1. **Needed to play:** `app/init`, `app/getServerXmls`, a server list (a client has no way to learn
   where the game server is), `char/list` extensions, `account/purchaseCharSlot`
2. **Expected:** `account/verify`, `account/sendVerifyEmail`, `account/forgotPassword`,
   `account/resetPassword`, `account/setName`, `char/fame`, `fame/list`
3. **Optional:** Discord linking, credits, daily calendar, weekly quests, pictures, global news

**Size: M for the first group, L for the rest.**

Also here: rate limiting is per-process, so two app servers behind a load balancer means double the
limit. Fine at one instance, wrong at four — the fix is a `failed_login` table or Redis.

---

## Phase 11 — Retire the C# tree

Only once every phase above is done and the client has been cut over.

- Delete `Server-Side/`, keeping `XmlDatas` until the content pipeline is confirmed to need nothing
  from it.
- `Server-Side/server/server.json` still carries the production IP `37.123.96.189`. Scrub it before
  the repository goes anywhere public.
- Move deployment, the Dockerfile and CI to the Rust tree.

---

## Cross-cutting, throughout

**Protocol.** Each phase adds messages. Add them to `crates/net/src/message.rs` as they are needed
rather than designing them up front — the client is not listening yet, so the cost of changing one
is currently zero, and that is a window worth using.

**The verification discipline stays.** Every protection gets a mutation check: break it, watch
exactly the right test fail, restore it. Ten of these have already caught real bugs, including two
this week that the tests found after I believed the work was finished. It is not ceremony.

**The tick budget is a constraint, not an observation.** 23× headroom at 200 players today.
`cargo run --release -p hendra-sim --example tick_budget` after any phase that touches the loop.

**Coverage examples earn their place.** `crates/behavior/examples/gaps.rs` turned "implement the
behaviours" from a guess into a number. Phase 7 should have the same for activates, and phase 10
for endpoints.

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

Phase 6 is the one to do next and the one with the best return. After it, the server is roughly
70% and — more importantly — everything it claims to do, it does.

The client cutover is not in this plan by request. It remains the only thing standing between all
of this and a game anyone can play.

# Finishing the server

Everything left between here and a server that plays the whole game, with the client cutover
deliberately excluded and left until last.

Phases 0–5 are done: workspace, content, protocol, transport, simulation, behaviour language,
persistence, authentication. What follows is phases 6–11.

**Where we are:** ~57% of the old server by subsystem. 455 tests.

**Sizing** is relative, not calendar: **S** is an afternoon, **M** is a day or two, **L** is
several days, **XL** is a week or more.

Boxes are ticked only when the work is written, tested, mutation-checked and committed. New tasks
found while completing one are added to this file at the same time.

---

## A correction, and what it taught

An earlier version of this plan said ~60% and quoted 629 enemies and 3,457 behaviour uses. Both
were wrong. Generating the content files to look at exposed four bugs in the C# reader, each of
which drops a whole enemy rather than one argument. The real corpus is **748 enemies and 8,631
behaviour uses**; the measurement had been taken against a corpus missing 42% of its content.

They were dropped in silence, which is the part worth carrying forward: the count looked exactly
like success. Every phase below that claims a percentage should be able to say where the
denominator came from, and every converter should report what it could not read.

---

## Ordering, and why

Phase 6 comes first because it is the cheapest work with the largest effect: the behaviour language
is at 100% of the content's uses, but much of what those behaviours ask for lands nowhere. Phase 7
follows because items are what a player touches every minute. Phase 8 makes the world a game rather
than a room. Phase 9 blocks nothing, so it goes late. Phase 10 runs alongside anything. Phase 11 is
the end.

---

## Phase 6 — Make the simulation mean what it says

Things that are recorded but not acted on.

### 6.1 Condition effects — **M**

- [x] `Rules` derived from a `ConditionSet`, so hot paths do arithmetic rather than bit tests
- [x] Invulnerable and Invincible refuse damage at `can_hit` and in explosions
- [x] Armored and ArmorBroken change defence
- [x] Paralyzed, Petrify and Stasis refuse movement, with their own `MoveRefusal`
- [x] Slowed, Speedy and NinjaSpeedy scale movement
- [x] Stunned refuses shooting; Sick refuses healing; Paused skips thinking entirely
- [x] Bleeding and Healing move health per second, carrying a fraction between ticks
- [x] Immunities refuse their effect at `give_effect`
- [x] Weak, Damaging, Berserk, Curse, Hexed and Dazed scale damage and cooldown in `Rules`
- [ ] Weak, Damaging and Berserk applied to outgoing damage at the point a shot is fired
- [ ] Dazed applied to weapon cooldown
- [ ] Invisible drops the entity from other players' snapshots
- [ ] Quiet refuses ability use (needs 7.1)

Confused, Drunk, Hallucinating, Blind and Darkness are client-side only and correctly do nothing
here.

### 6.2 The stat system — **M**

- [x] `Stats` with base, equipment and boost layers kept apart
- [x] Base capped at the class ceiling; equipment and boosts reach past it
- [x] `equipment_boosts` sums what is worn, ignoring boosts that name no real stat
- [x] Derived values: damage, rate of fire, movement, regeneration
- [ ] `Stats` on `Entity`, replacing the bare `speed` field
- [ ] Recompute the equipment layer whenever a worn slot changes
- [ ] Movement uses `movement_speed()` rather than a constant
- [ ] Weapon cooldown uses `rate_of_fire()`
- [ ] Damage uses `damage_multiplier()`
- [ ] Health and magic regenerate between fights
- [ ] Stats reach the client in the snapshot

### 6.3 Experience, levels and fame — **M**

- [ ] XP on kill, split by damage contribution, scaled by `exp_multiplier`
- [ ] `no_experience` suppresses it, so summons cannot be farmed
- [ ] Level-up rolls each stat inside its class's declared range
- [ ] Fame from the same event
- [ ] Call `hendra_characters::record_progress` so class unlocks advance through play
- [ ] Persist level, experience and fame at the checkpoint

### 6.4 Loose ends — **S**

- [ ] `toss_object`'s telegraph: a delayed spawn queue on `World`, so thrown attacks are dodgeable
- [ ] Drain `take_announcements` into a server message, so bosses are heard
- [ ] Drain `take_ground_changes` into a server message
- [ ] `App::new` builds an empty catalog and silently offers zero classes; remove or rename it
- [ ] `apply_setpiece` compiles to a ground transform, which is wrong rather than missing. Make it
      unsupported until 8.3 so it reports instead of doing the wrong thing
- [ ] Decide whether to fix the 9 dangling transitions and 18 shadowed state names in the `.beh`
      files, now that those are the source of truth

**Exit criteria:** every behaviour the content uses has an observable effect on the world.

---

## Phase 7 — Items and abilities

Measured with `cargo run --release -p hendra-content --example activates`: 1,609 items, 1,393 with
an ability, 1,942 activates across 43 kinds.

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

### 7.1 The activate framework — **M**

- [ ] `ActivateDesc` compiles to an `Effect` enum, mirroring the behaviour crate's shape
- [ ] A `use_item` client message
- [ ] MP cost, cooldown and `Quiet` checks
- [ ] `activates.rs` reports the percentage implemented

### 7.2 The effects, in ranked order — **L**

- [ ] `IncrementStat` (35%, depends on 6.2)
- [ ] `Dye` and `UnlockSkin` (30%, storage plus a snapshot field)
- [ ] `ConditionEffectSelf`, `ConditionEffectAura`, `Create`, `Heal`, `Shoot` (reuse phase 6)
- [ ] `CreatePet`, `Pet`, `PermaPet`, `PetSkin` (9%, a subsystem of its own; defer behind the rest)
- [ ] The remaining 25 kinds to 100%

### 7.3 Stacking, consumables and bags — **S/M**

- [ ] Potions stack to a limit
- [ ] Consumables decrement on use
- [ ] Bag type decides which colour bag loot drops in

### 7.4 Vendors and gift chests — **M**

- [ ] Currency on the account: gold, fame, tokens
- [ ] A purchase path with the same row locking the item moves use
- [ ] Merchant and gift chest entity kinds

---

## Phase 8 — The realm

### 8.1 Oryx and the realm event manager — **L**

- [ ] Populate by terrain and probability, using `spawn_probability`, `per_realm_max` and `terrain`
- [ ] Count enemies and announce events
- [ ] Close the realm and spawn the castle

### 8.2 Portals and dungeon lifecycle — **M**

- [ ] Portal lifetime and player count
- [ ] Close a dungeon when it empties

### 8.3 Setpieces — **M**

- [ ] A setpiece format, reusing HMAP
- [ ] A stamp operation, which fixes the wrong `apply_setpiece` from 6.4

### 8.4 The remaining entity kinds — **M**

- [ ] Decoy, Trap, Sign, GiftChest, ConnectedObject, Wall, GuildHallPortal

### 8.5 Terrain on the wire — **M**

- [ ] Send terrain to clients, which the cutover depends on

Ground damage already works and is tested; an earlier draft of this plan was wrong about that.

---

## Phase 9 — Social

Independent of everything above and blocks nothing.

### 9.1 Chat — **M**

- [ ] say, tell, guild and global scoping
- [ ] Rate limiting and a mute list
- [ ] Moderation commands

### 9.2 Guilds — **L**

- [ ] Schema, ranks, hall worlds, the board

### 9.3 Friends and private messages — **M**

- [ ] Friend list and requests
- [ ] Private messages

### 9.4 Market — **L**

- [ ] Listings and escrow through the same row locking the trade path proved
- [ ] Fees

The single most dupe-prone thing left to build: every listing is an item move.

---

## Phase 10 — Finish the app server

Eight of 48 endpoints are done, and they are the ones that matter.

### Needed to play — **M**

- [ ] A server list, so a client can learn where the game server is
- [ ] `app/init` and `app/getServerXmls`
- [ ] `char/list` extensions and `account/purchaseCharSlot`

### Expected — **M**

- [ ] `account/verify`, `sendVerifyEmail`, `forgotPassword`, `resetPassword`, `setName`
- [ ] `char/fame` and `fame/list`

### Optional — **L**

- [ ] Discord linking, credits, daily calendar, weekly quests, pictures, global news

### Operations

- [ ] Shared login throttling: per-process today, so two app servers behind a load balancer double
      the limit. Needs a `failed_login` table or Redis
- [ ] A coverage example for endpoints, as `gaps.rs` and `activates.rs` do for their areas

---

## Phase 11 — Retire the C# tree

Only once every phase above is done and the client has been cut over.

- [ ] **Scrub the production IP `37.123.96.189` from `Server-Side/server/server.json`.** Do this
      before the repository goes anywhere public, not at the end
- [ ] Delete `Server-Side/`, keeping `XmlDatas` until the content pipeline needs nothing from it
- [ ] Move deployment, the Dockerfile and CI to the Rust tree

---

## Cross-cutting, throughout

**Protocol.** Add messages as each phase needs them rather than designing them up front. The client
is not listening yet, so changing one costs nothing today.

**Mutation checks.** Every protection gets one: break it, watch exactly the right test fail, restore
it. Nineteen so far have caught real bugs, several after the work looked finished.

**Silence is the enemy.** Every converter and loader reports what it could not handle. The four
parser bugs cost 119 enemies and went unnoticed because the total looked like success.

**The tick budget is a constraint.** 24× headroom at 200 players today. Run
`cargo run --release -p hendra-sim --example tick_budget` after any phase that touches the loop.

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

The client cutover is not in this plan by request. It remains the only thing between all of this
and a game anyone can play.

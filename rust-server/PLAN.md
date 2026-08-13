# Finishing the server

Everything left between here and a server that plays the whole game, with the client cutover
deliberately excluded and left until last.

Phases 0–5 are done: workspace, content, protocol, transport, simulation, behaviour language,
persistence, authentication. What follows is phases 6–11.

**Where we are:** ~97% of the old server by subsystem. 646 tests.

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
- [x] Invincible refuses the hit entirely; Invulnerable takes the hit and its effects but no damage
- [x] Armored doubles defence and ArmorBroken removes it, matching `StatsManager`
- [x] Paralyzed, Petrify and Stasis refuse movement, with their own `MoveRefusal`
- [x] Slowed, Speedy and NinjaSpeedy scale movement
- [x] Stunned refuses shooting; Sick refuses healing and zeroes vitality; Paused and Stasis refuse damage and thinking
- [x] Bleeding and Healing move health per second, carrying a fraction between ticks
- [x] Immunities refuse their effect at `give_effect`
- [x] Petrify and Curse scale damage taken; Hexed does nothing in the original and so does nothing here
- [x] Weak, Damaging and Berserk read by the stat formulas, as the original applies them
- [x] Dazed applied to rate of fire
- [x] Invisible drops the entity from other players' snapshots, filtered by the server so a client cannot be made to reveal it
- [x] Quiet refuses ability use, checked in `use_item` alongside the magic cost

Confused, Drunk, Hallucinating, Blind and Darkness are client-side only and correctly do nothing
here.

### 6.2 The stat system — **M**

- [x] `Stats` with base, equipment and boost layers kept apart
- [x] Base capped at the class ceiling; equipment and boosts reach past it
- [x] `equipment_boosts` sums what is worn, ignoring boosts that name no real stat
- [x] Derived values read from `StatsManager` rather than invented
- [x] `Stats` on `Entity`, replacing the bare `speed` field
- [x] Recompute the equipment layer whenever a worn slot changes
- [x] Movement uses `movement_speed()` rather than a constant
- [x] Weapon cooldown uses `shot_cooldown_ms()`, compounding the weapon's rate with dexterity
- [x] Damage uses `damage_multiplier()`, applied when the shot is made
- [x] Health and magic regenerate between fights
- [x] Stats reach the client in the snapshot, alongside a texture field for phase changes

### 6.3 Experience, levels and fame — **M**

- [x] XP on kill, shared with everyone within 25 tiles and capped at a tenth of a level, scaled by `exp_multiplier`. The original does not split by damage contribution, which an earlier draft of this plan claimed
- [x] `no_experience` suppresses it, so summons cannot be farmed; a paused player earns none
- [x] Level-up rolls each stat inside its class's declared range, capped at twenty levels
- [x] Fame from lifetime experience, with the game's milestones and stars
- [x] Call `hendra_characters::record_progress` so class unlocks advance through play
- [x] Persist level, experience and fame at the checkpoint, read from the world rather than remembered

### 6.5 Content identity — **M**

Content no longer needs a hand-picked hex number. An author writes a name; the catalog assigns the
runtime number from the content's identity, deterministically and independent of load order.

- [x] A UUID identity per object and tile, written as `uuid="..."` or derived from the name
- [x] `type="0x..."` is optional, so the legacy files keep working unedited
- [x] The catalog assigns unwritten numbers from 0x1000 upward, clear of everything the legacy
      files use
- [x] `cargo run -p hendra-content --example new_id` prints a fresh identity for new content
- [x] A number reached by probing is reported, because it depends on what else is loaded
- [x] **Store identities rather than numbers in the database.** `inventory_slot`, `vault_slot`,
      `character`, `class_progress` and `class_unlock` hold UUIDs, and the server translates at the
      seam between the world, which draws by number, and the durable side, which does not. An empty
      slot is `NULL` rather than zero, and both dupe protections were re-checked against that

### 6.4 Loose ends — **S**

- [x] `toss_object`'s telegraph: a delayed spawn queue on `World`, so thrown attacks are dodgeable
- [x] Drain `take_announcements` into the chat message, heard nearby or worldwide
- [x] Drain `take_ground_changes` into a new `Ground` message, surfaced to Godot
- [x] `App::new` removed; `with_content` is the only way to build one
- [x] `apply_setpiece` now reports itself unsupported rather than painting a circle of one tile,
      which is a different operation and was silently wrong. Four uses, restored in 8.3
- [x] Decided: leave them. The `.beh` files are regenerated from the C# on every conversion, so a
      hand edit is lost at the next run. The 18 renames already preserve the C# semantics, and the
      9 dangling transitions are bugs in the original that throw `KeyNotFoundException` when their
      enemy spawns, so dropping them is strictly better than reproducing the crash. Revisit in
      phase 11, when the `.beh` files become the source rather than an output

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

- [x] `ActivateDesc` compiles to an `Effect` enum, mirroring the behaviour crate's shape
- [x] A `use_item` client message, naming a slot rather than an item
- [x] MP cost, cooldown and `Quiet` checks, enforced by the world
- [x] `activates.rs` reports the percentage implemented: **100% of 1,942 uses, 43 of 43 kinds**

### 7.2 The effects, in ranked order — **L**

- [x] `IncrementStat` (35%, depends on 6.2)
- [x] `Dye` and `UnlockSkin` read; storage and the snapshot field are still to come
- [x] `ConditionEffectSelf`, `ConditionEffectAura`, `Create`, `Heal`, `Shoot` (reuse phase 6)
- [x] `CreatePet`, `Pet`, `PermaPet`, `PetSkin` read as an `Effect::Pet`; the pet subsystem itself is 8.4
- [x] The remaining 25 kinds to 100%

### 7.3 Stacking, consumables and bags — **S/M**

- [x] Potions stack to a limit, held on the character rather than in a slot, with the ceiling enforced in the statement so two pickups cannot both take the last place
- [x] Consumables are taken from the slot after they are used
- [x] Bag type decides which colour bag loot drops in, taking the best thing in the bag

### 7.4 Vendors and gift chests — **M**

- [x] Currency on the account: gold, fame, tokens, with the balance checked in the statement
- [x] A purchase path that pays and delivers in one transaction, reusing the claim `give_item` proved
- [x] Gift chests are containers, which the loot path already builds. A merchant is a fixture whose purchase goes through 7.4's `buy_item`

---

## Phase 8 — The realm

### 8.1 Oryx and the realm event manager — **L**

- [x] Populate by terrain, with `per_realm_max` as a ceiling. `spawn_probability` is read where the content gives one, which is nowhere in the shipped files: the original carries its weights in a hardcoded table, and reproducing it here would put content back in the code
- [x] `enemy_count` and `count_of_kind` for a realm measuring itself, and `announce` for what the realm says. An announcement with no speaker is named after the world rather than attributed to an entity, which would be a lie the client repeats
- [x] Close the realm once nine tenths is cleared, measured against the fullest it has been so a realm that never filled still closes. Spawning the castle is a `stamp` of a setpiece, which 8.3 built

### 8.2 Portals and dungeon lifecycle — **M**

- [x] Portal lifetime, which the death-effect portals already carry as `expires_in_ms`
- [x] Close a dungeon when it empties, after a minute so walking out and back returns you to the same room. The entry world is exempt, and closed worlds are dropped from the registry when the next one starts

### 8.3 Setpieces — **M**

- [x] A setpiece is an HMAP like any other map, so no new format was needed
- [x] `World::stamp` writes every square a piece names, centred on the point and clearing what stood there. Wiring `apply_setpiece` back to it needs the content's setpiece files, which are not in this tree

### 8.4 The remaining entity kinds — **M**

- [x] Decoy and Trap as their own kinds, so a decoy is shot by enemies and not by its owner. Sign, Wall and ConnectedObject are already fixtures; GiftChest is a container; GuildHallPortal is a portal, and 9.2's halls reach it through `get_or_start_for`

### 8.5 Terrain on the wire — **M**

- [x] Terrain is sent on join, one run-length encoded row at a time, and ground changes update what a later joiner is told. The tick budget went from 24x to 18x headroom at 200 players, which is the cost of holding the tile grid

Ground damage already works and is tested; an earlier draft of this plan was wrong about that.

---

## Phase 9 — Social

Independent of everything above and blocks nothing.

### 9.1 Chat — **M**

- [x] say and tell scoping. Guild scoping waits on 9.2, and global waits on somewhere to put it
- [x] Rate limiting on a sliding window, and a mute read fresh so it takes effect without a reconnect
- [x] `/mute`, `/unmute`, `/ban` and `/unban`, on two ranks so silencing and removing are separate powers. A player without the rank is told the command does not exist rather than that they may not use it

### 9.2 Guilds — **L**

- [x] Schema, four ordered ranks, and the board. Hall worlds already work through `get_or_start_for`, which gives a personal instance per key

### 9.3 Friends and private messages — **M**

- [x] Friend list and requests, two rows per friendship so each side can remove the other without deciding for them
- [x] Private messages that wait for someone not online, readable and deletable only by who they were sent to

### 9.4 Market — **L**

- [x] Listings hold the item, so it exists in exactly one place. A sale closes the listing first, which is what makes the race resolve at all
- [x] A fee taken from the seller rather than added to the price, so what a buyer is quoted is what a buyer pays

The single most dupe-prone thing left to build: every listing is an item move.

---

## Phase 10 — Finish the app server

Eight of 48 endpoints are done, and they are the ones that matter.

### Needed to play — **M**

- [x] A server list at `GET /servers`, unauthenticated because a client needs it before it has anywhere to send a password. Read from `HENDRA_GAME_SERVERS` so moving a server is configuration, not a new client
- [x] `GET /init`, which answers protocol, servers and class count in one request. `getServerXmls` is not needed: the client reads content from its own assets and the server reads the same files
- [x] `char/list` already returns what a select screen needs. Character slots are a currency purchase, which 7.4 built

### Expected — **M**

- [x] `POST /name`. Email verification and password reset need a mail sender, which is deployment rather than server work and is recorded below
- [x] `GET /fame`, an account's characters best first

### Needs something outside the server

- [x] Email verification and password reset. The tokens, expiry, single use and purpose separation
      are all here and tested; only delivery leaves the process, behind a `Mail` trait. Without a
      sender configured, links are logged and the server says so at startup rather than pretending
      to have sent them

### Optional — **L**

- [x] Global news, in-game news and the daily calendar. `endpoints` now reports **26 of 40**.
- [x] Language strings, credit offers, quests and weekly quests, age confirmation, skins and
      pictures. `endpoints` reports **35 of 40**.
- [ ] **The last five need something this server cannot provide.** `registerDiscord` and
      `unregisterDiscord` need a Discord application and OAuth secrets; `getTextures` serves sprite
      sheets the client ships and the server has no copy of; `security/gameData` and
      `security/securityProtocols` served the old client-side anti-cheat, which this server
      replaced by not trusting the client at all, so implementing them would mean reintroducing the
      thing they existed to support

### Operations

- [x] Shared login throttling through a `failed_login` table. The in-process limiter stays as the
      fast path; the shared count is what makes the limit mean the same thing with four servers
- [x] A coverage example for endpoints: `cargo run -p hendra-app --example endpoints` reports **12 of 40**

---

## Phase 11 — Retire the C# tree

Only once every phase above is done and the client has been cut over.

- [x] **Scrubbed the production IP from the working tree.** It appeared three times, not once:
      `Server-Side/server/server.json`, `Server-Side/wServer/wServer.json` and
      `Client-Side/.../ProductionSetup.as`. All now say `127.0.0.1`.
- [ ] **The IP is still in git history.** Removing it needs a history rewrite, which rewrites
      shared commits and is not a call to make without asking. If the repository is going public,
      do this before it does; if the address has changed since, it may not be worth the disruption
- [ ] Delete `Server-Side/`, keeping `XmlDatas` until the content pipeline needs nothing from it
- [x] A two-stage `rust-server/Dockerfile` and `.github/workflows/rust.yml`. CI runs fmt, clippy with warnings denied, the full test suite against a real Postgres, all three coverage examples and the tick budget, in the order that fails fastest. The Dockerfile is not build-verified: the Docker CLI is present but its daemon is not running here, so `docker build` could not be run

---

## Cross-cutting, throughout

**Protocol.** Add messages as each phase needs them rather than designing them up front. The client
is not listening yet, so changing one costs nothing today.

**Mutation checks.** Every protection gets one: break it, watch exactly the right test fail, restore
it. Nineteen so far have caught real bugs, several after the work looked finished.

**Read the original before writing a rule.** Every constant that governs combat comes from
`wServer`, not from judgement. An earlier pass invented armour as `+20` where the original doubles
defence, a damage floor of 15% where it is 25%, and Slowed as a multiplier where it holds speed at
the base. All of it looked reasonable and all of it was wrong.

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

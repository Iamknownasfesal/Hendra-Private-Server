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

Three items are ticked as **decisions** rather than as code: the production IP in git history, when
to delete the C# tree, and the five endpoints that need services this server does not have. Each
records what was decided and why, so a later reader knows they were considered rather than missed.

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

## Phase 12 — Connect what was built but not wired

Found by asking whether "plan complete" meant "server complete". It did not: four things were
written, tested and never called.

### 12.1 Abilities that change something durable — **M**

- [x] The session carries out what the world hands back. It returned `Dye`, `UnlockSkin`, `Pet`,
      `Currency`, `Boost`, `Unlock`, `Portal` and `Generic` with a comment saying the caller would
      settle them, and the caller only checked whether the item was consumable. Roughly 893 of the
      1,942 activate uses parsed and then did nothing, which is the exact failure phase 6 existed
      to eliminate, reintroduced in phase 7
- [x] Storage for each: two dye slots, a worn skin, a backpack, pets, boosts and unlocked portals

### 12.2 The realm — **S**

- [x] `crates/sim/src/realm.rs` is written and tested and nothing calls it. No world populates
      itself, so a realm is an empty map

  Rewritten against `Oryx.cs` first: the version being wired had invented its rules. The real ones
  are per terrain, not global. A terrain's population is the squares it covers divided by the
  squares it gives each enemy, from a table Oryx keeps in code and not in the content. The
  population is held in a band, topped up below three quarters and thinned above one and a half,
  checked once a minute. The realm closes on a clock half an hour after it opens, not on a body
  count, with a minute's warning and then the castle.

- [x] Scenery is not an entity

  Found by populating the real realm map and getting nothing. The world made an entity of every
  object the map declared, and `world1.hmap` declares 245,916 trees. They filled the 65,536-entity
  arena exactly, so a realm could never spawn a single enemy, and every tree would have taken a
  place in every snapshot for the life of the world. `Wmap.Load` keeps static objects on the tile;
  we now do too, with the class consulted as well as the flag so the nexus fixtures a player has to
  name stay entities. Scenery goes to the client once with the ground, carrying its size, since the
  realm scales seventy thousand of its trees for variety.

- [x] Enemies carry the terrain they were placed on, and children inherit it

  Counting an enemy by where it is standing would let a chase empty one terrain and overfill the
  next, and would lose anything that bred. `Enemy.Terrain` does the same in the original.

- [x] A closed realm refuses arrivals, so nobody is let in to be sent straight back out

`cargo run -p hendra-sim --example realm_population` fills the real map from the real content:
2,716 enemies against a target of 2,712, every terrain at its number.
`cargo run -p hendra-sim --example map_entities` shows what every shipped map puts in a world.

- [x] The castle handoff. When a realm closes, the original quakes everybody to the castle

  Left open when 12.2 landed because it needed a world-to-session channel that did not exist. It
  does now: a world can move a body but not a connection, so `Order::GoTo` asks and the session
  makes the same move a portal makes. The session waits on it beside the client, because a player
  standing still sends nothing and a closing realm should still empty.

### 12.3 Setpieces — **S**

- [x] `World::stamp` is only called by tests, so `apply_setpiece` still reports unsupported. Those
      are the last 4 of 8,631 behaviour uses

  Bigger than it looked. `apply_setpiece` is one of two ways setpieces are used, and the smaller
  one: `Realm.Init` calls `SetPieces.ApplySetPieces`, which scatters fifteen kinds of structure
  across a realm before any enemy spawns. Nothing did that, so a realm had no temples, no castles,
  no graveyards, no lich, no cyclops god and none of the chests they hold.

- [x] All fifteen setpieces, ported from `wServer/realm/setpieces/`

  They are drawing programs rather than saved maps, which is the point: a grove picks its own radius
  and scatters cherry trees around its edge, a building draws four walls and then knocks holes in
  half of it. `crates/sim/src/setpiece.rs` keeps them as pure drawings, so each can be checked
  square by square without a world to check it in.

- [x] What a setpiece paints is scenery, not entities

  A castle drawn as entities cost 800 places in the world and 800 snapshot entries for 800 stones
  that never move; a whole realm cost 32,249. The original writes them onto the tile and calls
  `EnterWorld` only for bosses, chests and destructible walls. Now so do we: 639 entities for a
  whole realm. Painting changes the map as well as the collision bitmap, so a player joining later
  is told the world as it is rather than as it was drawn.

- [x] `apply_setpiece` draws where the entity stands

  The four names the shipped behaviours use name nothing, in the original too: `Type.GetType` finds
  no class and the behaviour throws where it stands. The runtime says which name was wanted instead.

`cargo run -p hendra-sim --example realm_population` builds a whole realm from the real map and
content: 84 buildings, 24 groves, 19 and 8 temples, 7 graveyards, 5 towers, 4 castles, 4 lich
temples, 3 lava fissures, the djinn, the ent and the crystal, then 2,720 enemies on top.

### 12.4 Trade — **M**

- [x] The store's trade is dupe-proof and tested, and there are no protocol messages for it, so no
      client can reach it

  Nine messages, from `Player.Trade.cs` and its handlers: four in and five out. A trade begins by
  both sides asking, which is what the original does and means one message rather than two does the
  work; a request that is not answered expires after twenty seconds.

- [x] `crates/server/src/trades.rs`, which is where two connections meet

  The world knows about bodies rather than connections, so this holds each player's sender and
  writes to the other side directly, the way the world writes snapshots. Who is trading with whom
  is kept apart from how to reach them, which lets the agreement be checked without a connection to
  check it over.

  The guards, each confirmed by breaking it: a worn slot cannot be offered, changing an offer
  unagrees both sides, an accept naming a stale offer does nothing, and a settled trade is cleared
  before the items move. A disconnection ends whatever was half-agreed rather than leaving it.

- [x] `Store::character_named`, so somebody can be asked to trade by typing their name

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
- [x] `Dye` and `UnlockSkin`, read and carried out. See 12.1
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
- [x] **The last five are blocked on something outside this server, and stay that way.**
      `registerDiscord` and `unregisterDiscord` need a Discord application and OAuth secrets;
      `getTextures` serves sprite sheets the client ships and the server has no copy of;
      `security/gameData` and `security/securityProtocols` served the old client-side anti-cheat,
      which this server replaced by not trusting the client at all, so building them would mean
      reintroducing the thing they existed to support. 35 of 40 is the finished figure

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
- [x] **The IP in git history: decided to leave it.** Removing it would rewrite every commit hash
      after the first occurrence and invalidate every existing clone. The working tree is clean,
      which is what a fresh checkout gets. Revisit only if this repository is made public, and do
      it then as a deliberate one-off rather than as part of other work
- [x] **Deleting `Server-Side/`: decided to keep it until the client is cut over.** That is this
      phase's own precondition and the cutover is out of scope by request. It is also the reference
      the cutover will be written against, and the behaviour and map converters still read from it.
      The server itself does not: `content/behaviours` and `content/maps` are committed, so nothing
      at runtime depends on the C# tree
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

## Phase 13 — What the protocol census found

Written because the answer to "is this complete" had been wrong three times, and each time the thing
that found the gap was counting something rather than reasoning about it. So the protocol got
counted: one row per handler in `wServer/networking/handlers/`, and what answers it here.

The first run said 82%. Nine packets had nothing behind them, and four were real mechanics.

- [x] The five remaining app endpoints

  Two link a Discord account and are administrators-only, as in the original. One serves the texture
  pack. `security/gameData` is an empty class in the original, so reproducing it faithfully means
  having nothing. `security/securityProtocols` is client attestation: the client sends SHA-256 of its
  own hardcoded rate of fire and cooldown and is let in if they match. That secures nothing, so the
  endpoint answers with the numbers the server itself enforces and says which side decides.

- [x] Escape, and Teleport

  Escape leaves for the nexus with no portal to step into. Teleport carries all seven of the
  original's refusals, and a grace period so the server's own teleport is not read as somebody
  moving too fast.

- [x] Shops, which nothing had

  Ten of them, from `MerchantLists.cs`, placed on the squares their region marks and dealt out
  around them. Two of the original's own item names differ from the content only in capitals, so the
  lookup does not insist on them.

- [x] Guild invite and remove, market commands, and the ignore and lock-out lists

  The store had guilds and a market and no way to reach either. The lists did not exist at all, and
  both now do something: somebody who has ignored you does not hear you, and somebody who has locked
  you out cannot be teleported to. Both answer as though the person simply is not there, because
  telling somebody they have been blocked is telling them to use another account.

- [x] Prestige, and the gift chest it delivers into

  Every fifteen hundred fame becomes one prestige and the character starts over, both in one
  transaction. Gifts had nowhere to go, so there is a gift chest now.

- [x] The five handlers that are left do nothing, and the reason is checked rather than claimed

  The forge, the crystal and the marks ask for object types the shipped content does not have, and
  the gamble and the unbox call methods the original does not define. `WITHOUT_CONTENT` lists the
  types, and a test re-checks them: if a later content drop adds any, it fails and says the handler
  is worth implementing.

`cargo run -p hendra-server --example handlers` counts the protocol. It carries its own tests.

## Phase 14 — Commands, which nothing had counted

Asked a fifth time whether the server was complete, and rather than answer from memory, counted a
surface nobody had counted: what a player can type. The original has 95 across
`realm/commands/`. This server had five.

- [x] `crates/server/src/commands.rs`, a table rather than a match

  A match with ninety-one arms is not something anybody can compare against another server. A table
  can be walked, printed and checked, and `cargo run -p hendra-server --example commands` prints it
  from the same file the server compiles rather than from a copy.

- [x] The 49 that are game mechanics

  Talking, going places, guilds, the market, the lists, and what the server knows about itself. Each
  goes through the machinery the protocol already uses: `/tp` is the world's teleport with all its
  refusals, `/trade` is the trade registry, `/ignore` is the list the whisper path reads. A command
  that reached past those would be a second, weaker door into the same room.

- [x] `/who`, `/online`, `/pos`, `/uptime`, `/lefttomax`, and guild chat that crosses worlds

  Each needed something the server did not expose: the world can now say who is in it and where
  somebody is standing, and the roster can say who is connected.

- [x] The 46 that are left, which are administrators' tools rather than mechanics

  All of them. Five say plainly that this server will not do them rather than doing nothing and
  looking as though they worked: acting as another account means holding two identities on one
  connection and every durable write here names its own, and linking a world at runtime means
  nothing when a world is reachable by the name its definition gives it.

## Phase 15 — The last two categories

- [x] The gift chest, which closed a hole opened three commits earlier

  Prestige purchases and gifts had storage and nowhere to open them from. The chest is a durable
  location like the vault rather than a bag, so taking a gift removes the row in one locked
  transaction; without that the same gift is handed out on every visit. One-way, refused at the
  store rather than in the session so it holds for every path into it.

- [x] Guild halls, which were one room per account loading one of four maps

  One room per guild now, as the original has them, and the level chooses the map, so the three
  `GuildHallN.hmap` files that were sitting unused are reachable. The upgrade merchant works, paid
  from the guild's fame by an officer.

- [x] The nexus shows portals to every running world, labelled with who is in them

- [x] Davy's keys are announced as they are found, and told to whoever arrives after

- [x] Access is enforced by the instance key rather than by a check

  The original checks on entry that the vault is yours and the hall is your guild's. Here the key
  carries the account or the guild and the only caller is the session acting for it, so there is no
  way to name somebody else's room. A check would be a second answer to a question already
  answered, and a second answer is somewhere the two can disagree.

`cargo run -p hendra-server --example worlds` counts the worlds with their own logic.
`cargo run -p hendra-server --example commands` counts the commands against the original's own list.

## Phase 16 — Loot, which nothing counted

Asked a sixth time whether the server was complete. Every counter read full, so rather than answer,
counted the one category that had never had a counter: the kinds of loot the content's tables use.

The original uses three: `ItemLoot` 858 times, `TierLoot` 767, and `Threshold` 204. This server
rolled two.

- [x] `threshold`, which was dropped twice over in silence

  The converter emitted a bare `threshold` with the share and the nested items both gone, and the
  compiler then dropped what was left because it matched no kind it knew. Two hundred and four of
  the content's soulbound drops went nowhere and nothing said so. The converter emits
  `threshold(share) { ... }` now, the grammar nests, and all 204 carry their children.

- [x] Loot that belongs to whoever earned it

  Everything under a threshold is rolled once per player who took that share of the enemy's health
  off it, into a bag only they can open. Enforced at the pickup rather than drawn differently, since
  a bag anybody could take from would make the threshold decide who the loot was rolled for and
  nothing at all about who ends up with it.

- [x] Enemies remember who hurt them

  Per enemy rather than globally, because it is a fact about that fight, and bounded at sixty-four
  because the list lives on every enemy and a realm holds thousands.

- [x] The share is measured against what the enemy started with

  Against what it has when it dies would be a share of nothing, which nobody can meet.

  The original compares its threshold against raw damage rather than a share, so `0.05 <= 3000` is
  true for anybody who landed a hit at all. That is a type confusion rather than an intention: the
  value is named a threshold, written as a fraction, and every table uses it as one. This reads it
  as the share it is spelled as, which is the mechanic the content describes.

`cargo run -p hendra-behavior --example gaps` counts loot kinds alongside behaviours and conditions,
and carries a test that walks a threshold from C# through the converter and the compiler.

## Phase 17 — Entity kinds, the last category without a counter

- [x] The player merchant, which nothing had

  A market listing could only be reached by typing `/market`. It now stands as a merchant in the
  marketplace, in the row its kind belongs to, refreshed on the same timer as the nexus portals. A
  listing bought from a stall goes through the market rather than through a shop's till, since a
  shop's stock is endless and a listing is one item somebody else owns until it is bought.

- [x] The guild hall portal, which was a second door into a different room

  Found by writing the census row and checking whether it was true. `/ghall` routed per guild and
  chose the map by level; stepping into the portal object went through the ordinary door and got one
  hall per account at map zero. One path now, used by both.

`cargo run -p hendra-server --example entities` counts the kinds of thing a world holds. Names differ
from the original's on purpose, so the rows say which is which rather than matching names and calling
every one a gap.

## Phase 18 — What the file-by-file audit found

Counted surfaces all read full, so the next audit went through `wServer` file by file rather than
category by category. It found thirteen things, one of them a mechanic that silently does nothing.

### 18.1 Death — **L**

- [x] A player whose health reaches zero simply vanishes. The character is never marked dead, no
      death message reaches the client, no gravestone, no announcement, no fame is worked out, and
      `desc.resurrects` is parsed and never read. Logging back in finds the character alive

  The world notes a death before it reaps the body, names it after whatever last took health off,
  and puts a gravestone where it fell: which stone and how long it stands come from how much of the
  character was finished, as they do in the original. The session answers the rest, in the
  original's order and with the same stops: nowhere personal and no nexus kills anybody, an amulet
  spends itself and sends them home alive, and only then is the character written down as dead.

  The durable half runs before anything is sent. A death message the player sees and a character the
  database still calls alive is a character they can log back into.

- [x] A death is kept, not just noted

  A `death` table, one row per character that has died, written in the same transaction that marks
  the character dead: neither half alone means anything. The class is remembered by identity and the
  row does not reference the character, so a death outlives the character it happened to and a
  graveyard does not empty itself when somebody tidies up.

  A character dies once. A second attempt finds it already dead and refuses, which is what stops two
  worlds both deciding they killed the same person from paying its fame twice. Proved with eight
  concurrent attempts against a real database: one recorded, one payment.

  The fame a character finished with goes to the account and to its guild, both inside the same
  transaction, so a guild joined or left between the death and the payment cannot take it to the
  wrong place.

### 18.2 Fame — **L**

- [x] `FameCounter` and the twenty bonuses in `common/FameStats.cs`. Nothing tracks shots, hits,
      dungeons, assists, tiles seen, teleports, abilities or potions, so no bonus can be awarded

  `crates/sim/src/fame.rs` holds all twenty. They compound: each is a share of the fame including
  the bonuses before it, which is what the original does by threading a running total through its
  loop, so the order they are declared in is part of the arithmetic rather than a matter of taste.

  Counted by the server from things it already decides: a shot where it is fired, a hit where it
  lands, a kill where the enemy dies. Nothing is taken from a client, because a client that reports
  its own accuracy reports whatever earns the most.

  The counters live on the character and are added to rather than set, so a session that ends
  without saving loses that session's counting and not every session's. A death reads them from the
  database rather than from the world, since a character's life is longer than one session.

  Still uncounted, and each needs something that does not exist yet: tiles seen (no fog of war),
  level-up assists (no party), and quests completed (no quest completion). Their bonuses are in the
  table and will start paying the moment the counters are fed.

### 18.3 Equipment sets — **M**

- [x] Seven sets in the content. Wearing a full one grants extra activates. The file is never parsed

  Read now, held by the catalog, and applied where the worn stat layer is worked out, so what a set
  gives goes away the moment a piece comes off. A set gives nothing for three of its four pieces,
  which is the whole shape of it and the thing the tests hold onto.

  Two details that would each have made this look implemented while doing nothing. A setpiece naming
  item type `0xFFFF` asks for an *empty* slot, which is how a three-piece set is spelled; read as an
  item it is a set nobody can wear. And a set's `IncrementStat` is a boost rather than the permanent
  rise the same effect means on a potion, because `ApplySetBonus` calls `IncrementBoost` for it.

  The second was caught by a test asserting that at least five of the shipped sets raise a stat. It
  said zero, because the effect being matched was the wrong one and nothing was ever applied.

### 18.4 Boost stacking — **S**

- [x] Stacked stat boosts add up flat here. The original halves each one after the largest, and
      takes only the highest of the non-stacking kind

  `stats::stacked` does both. Sorted largest first, each boost after the first counts for half of
  what the last did, so two rings of eight attack are worth twelve rather than sixteen and piling on
  a sixth is worth nothing at all. Non-stacking boosts are a separate pool of which only the largest
  counts, and the two pools add to each other.

  Two larger bugs turned up in the same place, both from the same line. A temporary boost's duration
  was discarded, so every one of them was permanent; and an aura's range was discarded, so an aura
  reached only whoever used it. Boosts are now held with their time, run down each tick, and
  restacked from the whole list when one lapses, because with halving what a boost contributes
  depends on which others are held and no bookkeeping at the edges gets that right.

### 18.5 Potion stacking — **S**

- [x] `ItemStacker`: potions stack in their own slots with a ceiling

  The store already had the stacks, dupe-proof, with the same ceiling of six the original's
  `init.xml` sets. Nothing called them, which is the pattern that keeps biting: built, tested and
  unreachable. Picking up a health or magic potion now goes to its stack rather than into the pack,
  drinking is addressed at slots 254 and 255 as the original addresses them, and the potion is taken
  durably before it heals, since one that heals and is still in the stack heals forever.

  The counts reach the client as their own message rather than as a container, because a stack is
  one kind of thing many times over and a container says what is in a slot rather than how much of
  it. The first attempt packed the count into the slot number's high byte, which nobody reading the
  protocol could have explained.

### 18.6 One session per account — **S**

- [x] The same account can log in twice. The original takes a lock and disconnects the other

  Claimed on arrival now: whatever else was playing on that account is ended first. Ended rather
  than the new login refused, because the common case is not somebody cheating but somebody whose
  connection dropped trying to get back in, and refusing them would hold them out until a timeout
  they cannot see. A takeover ends whatever trade the old session was in, for the same reason a
  disconnection does.

### 18.7 Soulbound on drop — **S**

- [x] Respected in a trade and not on a drop, so a soulbound item can be given away by dropping it

  The audit had the rule wrong. `InvDropHandler` does not refuse a soulbound drop: it drops it into
  a soul bag whose owner is the dropper. So dropping one is a way to move it and not a way to give
  it away, and the bag ownership that loot thresholds already needed is what carries it.

### 18.8 Quest choice — **S**

- [x] Scored by priority, level distance and range in the original, from a table of a hundred and
      twenty. Nearest-flagged-object here

  All hundred and twenty, extracted from the original rather than retyped, with its score:
  `(20 - |enemy level - your level|) * priority - distance / 100`. Three pulls against each other,
  so what the arrow points at is the most worthwhile thing near enough to be worth walking to rather
  than the nearest thing worth killing. The level range is a hard filter rather than part of the
  score, or a high enough priority would send a beginner to something that kills them.

  Ten of the hundred and twenty name enemies this content does not ship. They are kept and listed,
  each checked by hand for a near-miss on capitals, with a test in each direction: an eleventh
  missing name is a typo and fails, and one of the ten arriving in a content drop fails too.

### 18.9 Anti-cheat strikes — **M**

- [x] Five named offences are recorded in the original and nothing is recorded here, so there is no
      repeat-offender signal

  `crates/server/src/strikes.rs`. The world already refuses each thing on its own and none of that
  needs a record to be correct; what a record adds is the difference between one refusal and forty
  in ten seconds, which is the difference between a bad connection and a changed client. Twelve
  inside a ten-second window ends the connection, from the original.

  The account is left alone deliberately. Everything counted is a judgement made from timings over a
  network, and a network can produce all of it honestly, so the connection is cut and the reason
  logged for somebody to read. A server that banned on arithmetic would eventually ban somebody on
  a train.

### 18.10 Vault broadcast — **S**

- [x] A vault change reaches other sessions of the same account only on re-entry

  Narrower than the plan said, once one account plays one session: there are no other sessions of
  the same account to tell. What is left is the real case, and it was worse than the note claimed.
  A completed trade told both players it had happened and refreshed *neither* pack, so the side that
  did not settle it was looking at an inventory the database no longer agreed with. An
  administrator's gift landed in a chest its owner would not see until their next visit.

  A session can now be asked to re-read what it holds, and the paths that change somebody else's
  things ask. One slot: what a refresh reads is whatever is true when it reads it, so a second
  request waiting behind the first would read the same thing twice.

### 18.11 Position history — **M**

- [ ] `PositionTimeline`, which answers where a player was when a shot was fired

### 18.12 Loot boosts — **S**

- [x] `LTBoosted` and `LDBoosted`: loot-tier and loot-drop boosts, which the loot roll should apply

  The boosts were granted, stored with an expiry, and never read by anything that rolls loot. The
  drop boost now multiplies the chance on loot that belongs to whoever earned it, which is the roll
  that knows whose boost applies. A multiplier rather than a bonus, as the original has it: worth
  more on something that already drops often.

  Not done: the luck stat, which the original reads as boost index ten. This server models eight
  stats and has no tenth, so there is nothing to read. Faking it would be inventing a number.

### 18.13 Pets — **not wanted**

- [x] Pets are stored, chosen and never enter a world

  Declined. Asked for on 2026-08-14 and the answer was that this server does not want pets, so no
  pet ever enters a world. The storage the activate effects already write to is left where it is:
  it is harmless, and tearing it out would break the abilities that grant one.

### 18.14 A login queue — **S**

- [ ] `ConnectionQueue`, for when the server is full

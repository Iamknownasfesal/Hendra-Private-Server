# What the original has that this server does not

A read-only comparison of `Server-Side/` against `rust-server/`, made by reading both rather than by
trusting either side's own reports. Nothing here is implemented yet; this is the list and the
reasoning behind its order.

Kept out of `PLAN.md` deliberately. That file is the execution list and its rule is that nothing in
it stays unchecked; this is an assessment, and an assessment that forces its own implementation is
not one.

## How this was measured

Every claim below was checked against the C# source, not against our own censuses. That distinction
turned out to matter: one census had been over-reporting for the life of the project, and another
still reports a number frozen early in it.

---

## Part one: what is genuinely complete

These were checked by enumerating the original's surface and diffing it, not by reading our own
table of claims.

- **Behaviours and transitions.** The C# defines 99 behaviour and transition classes. Every one that
  the behaviour database actually instantiates maps to a primitive this runtime implements.
  Twenty-eight of the 99 are instantiated *nowhere* in `wServer` — dead code in the original.
  `OpenGate`'s only occurrence is inside a comment. `HpLessTransition`, which my first pass flagged
  as missing, is `hp_below` here and has 248 uses.

- **Realm setpieces.** `SetPieces.cs` places fifteen: Building, Castle, Crystal, Graveyard, Grove,
  KageKami, LavaFissure, LichyTemple, LuckyDjinn, LuckyEnt, Oasis, Pyre, TempleA, TempleB, Tower.
  We have exactly those fifteen. The other 23 setpiece files are the *event* pieces, which is Part
  two below.

- **Commands.** The C# registers 95 names. All 95 are in the census list, and the census resolves
  each one through the real dispatcher rather than through a table, so it proves what it claims.

- **Packet handlers.** All 51 are accounted for, including the five hit-claim packets, which are
  answered "decided by the world" on purpose.

- **HTTP endpoints.** All 37 routes the C# registers are covered. `/account/rp` is an alias of
  `resetPassword`, which is listed under that name.

- **Lag compensation.** `PositionTimeline` stores past positions so a client's *claimed* hit can be
  checked against where the target used to be. Our client never claims a hit — the world decides
  what a projectile strikes — so there is nothing to compensate for. Genuinely not applicable, and
  now confirmed by enumerating every client message we accept.

- **Procedural dungeons.** `DungeonTemplates.cs` is 38 lines of which the whole class is commented
  out. Not a gap.

---

## Part two: the real gaps

### 1. Realm events — **L**, and the largest thing missing

`Oryx.cs` runs an event system our realm has none of. On the death of any enemy the content marks
`Quest`, it picks an event at random and drops it into the world:

Skull Shrine, Pentaract, Grand Sphinx, Lord of the Lost Lands, Hermit God, Ghost Ship, The Magicial
lord of sky, LH Sentry. (Cube God, Dragon Head and the Shatters Defense System are in the table but
commented out, so eight are live.)

Each is a setpiece with a boss inside it, placed on a random passable square between Mountains and
MidForest terrain with no player nearby, drawn from its corner so the piece lands centred. An event
whose object has `PerRealmMax = 1` is struck off the list once used, so it cannot appear twice.

Alongside it, 21 enemies are "critical" and carry up to four kinds of taunt: a spawn message, a
"my {COUNT} Liches still stand" message, a "my final Lich" message, and a death message that can
name the player who landed the kill. Killing a critical enemy also forces every player's quest arrow
to be recalculated.

This is the endgame of a realm. Without it a realm is populated and never escalates, and the 23
unimplemented setpiece files are unimplemented precisely because they are the bodies of these
events.

Depends on: `apply_setpiece` (done), the quest arrow (done), `PerRealmMax` (needs reading), and a
broadcast that reaches a whole world (done, as `World::announce`).

### 2. An account may make unlimited characters — **S**

`Database.CreateCharacter` refuses once an account has `MaxCharSlot` living characters. Nothing here
refuses at all: `create_character` is a plain insert with no count, and no column holds a limit. The
`account/purchaseCharSlot` endpoint is answered by `Store::buy_item`, which sells an item and raises
no limit, because there is no limit to raise.

The visible effect is that character slots are a thing players can buy and already have infinitely
many of.

### 3. No IP bans — **S**

`BanIp`, `UnBanIp` and `IsIpBanned` have no counterpart. We ban accounts only. On a public server
that is the difference between a moderation tool that works once and one that works.

### 4. Chat kinds are encoded into the sender's name — **S**

The C# `ChatManager` sends typed messages: Say, Tell, Guild, GuildAnnounce, Announce, Oryx, Mob,
Invite, SendInfo, each with its own name and text colour. We send one `Chat { from, text }` and put
the kind in the sender string: `"X whispers"`, `"X [guild]"`, `"Server"`.

It works, because both ends are ours. It costs the client the ability to colour, filter or mute a
kind of message, and it means a player can choose a name that impersonates one.

### 5. Guest accounts — **not wanted, unless you say otherwise**

`CreateGuestAccount` and the `IsGuest` refusal on purchases. Worth a decision rather than an
implementation: a public server may not want anonymous accounts at all.

### 6. The guild treasury is a statistic — **checked, not a gap**

`AddToTreasury` accumulates merchant tax into one server-wide counter that nothing in `wServer` ever
reads. Implementing it would add a number nobody can see.

---

## Part three: what is wrong with how we measure

This part matters more than any single item above, because it is what let the largest defect in the
project survive to this week.

### 7. `convert.rs` reports a stale number — **S**

The converter prints "8501 of them (72%) are primitives the runtime already implements" against a
hardcoded list of fourteen names, frozen early in the project. It is wrong by about a quarter of the
content and it is the first thing anybody reads when they run the converter. The `gaps` census
compiles the real thing and is the number to trust.

### 8. Four censuses are tables of prose — **M**

`commands` resolves every original name through the real dispatcher, and `gaps` compiles the real
C#. Those two prove what they claim.

`handlers`, `entities`, `worlds` and `endpoints` list the right questions — I diffed all four
against the original and the coverage is right — but their answers are sentences a person wrote.
A row reading `ok Buy → decided by the world` is a claim, and nothing checks that the code it
describes exists or is reachable. That is exactly the shape of the behaviours defect: a table would
have said `ok Enemy → behaviour from the content` and been wrong for the life of the project.

### 9. Nothing exercises the assembled server — **M**

Every test builds its own fixture world. No test boots the server, loads real content and behaviours,
starts a real world and ticks it. The one defect that made every enemy in the game stand still lived
in precisely that seam for months with 880 passing tests either side of it.

---

## The order I would do these in

1. **The end-to-end smoke test (9).** It is the cheapest item here and it is the one that would have
   caught the worst defect found so far. Boot, load real content, start a realm, tick it, assert
   enemies have minds and something moves. Everything after this is safer for it.
2. **`convert.rs`'s stale number (7).** One line of misinformation that a reader meets first.
3. **Realm events (1).** The largest genuine feature gap, and the reason a realm currently has no
   endgame. Eight events and 21 taunt sets.
4. **The character limit (2).** Small, and it makes a purchasable thing mean something.
5. **IP bans (3).**
6. **Typed chat kinds (4).** Needs a wire field, so it wants doing alongside any other protocol
   change rather than on its own.
7. **Grounding the four prose censuses (8).** Worth doing after events, because events will add rows
   to them and the rework is cheaper once.
8. **Guest accounts (5).** Only if you want them.

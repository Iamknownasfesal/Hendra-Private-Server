# What the original checks, and what it takes on trust

Read from `realm/entities/player/Player.Verify.cs`, `realm/PositionTimeline.cs`.

**This file is entirely this project's work in the C# server.** The original took all of it on trust.
It is documented here because it is the design this server's own authority model was built from, and
because the constants were measured rather than chosen.

## What the original trusted, and what each was worth

| Claim | Original | Consequence |
| --- | --- | --- |
| "a bullet hit me" | believed | never sending one is godmode, complete and trivial |
| "my bullet hit that monster" | believed | widening your own hit test claims every monster on screen |
| "I am standing here" | believed | never be where a bullet is |
| "my shot started here, at this angle" | believed | bullets born anywhere; a 3-shot bow becomes 3 aimed guns |
| "I took ground damage" | believed | stand in lava forever |

Our server decides all five itself, which is why our client-facing surface has no hit claims at all.

## The constants, and why each is what it is

```
StrikeLimit          12          strikes inside one window before the connection is cut
StrikeWindowMs       10_000      the window, measured from the last strike
SilentHitsPerStrike  5           unreported hits applied before one strike is recorded
AcknowledgeGraceMs   600         how long a client has to own up to a hit
MostGraceMs          2_000       ceiling on that grace once latency is added
ShootOriginSlack     1.5 tiles   how far a shot may start from where the player said it was
MostShootDriftMs     1_000       how stale the trail may be before the shooter stops being anchored
ArcSlack             0.08 rad    how far a volley shot may sit off its own arc
HitClaimSlack        2.5 tiles   added to the hit box when judging a claim about a monster
HitClaimHistoryTicks 8           how far back the monster's own trail is searched
MoveSlack            1.5         multiplier on the speed a player's stats allow
MoveGraceTiles       1 tile      a flat allowance on top
MoveGraceMs          3_000       how long after arriving before movement is judged at all
```

`SweepBox = 0.2` and `MostSweepMs = 1000` live on the projectile and are covered on
[the projectile page](05-projectiles.md); both were set from measured disagreements with an honest
client rather than from taste.

## The four checks

### Being hit — deferred, not applied

A swept pass-through is **not applied immediately**. It is queued with a deadline of
`min(600 + latency, 2000)` ms, and applied only if the client has still not reported it by then. So
an honest client is never touched by the server's opinion, and damage cannot land twice because the
bullet's own hit set records the admission.

Every fifth silently-applied hit is one strike. Not every hit — the rate is what separates a bad
connection from a client that never reports.

### Shooting — origin and arc

The shooter is anchored to what the trail says, or to the newest sample plus whatever travelling its
own stats could have done since, capped at a second. The allowance is
`speed * 1.5 * elapsed / 1000 + 1.5` tiles — **tight when the trail is fresh and forgiving when it is
not**, which is the shape that avoids widening the slack for everybody.

For a multi-shot weapon, each shot of a volley must sit within `0.08` radians of the previous one
plus the weapon's own `ArcGap`. That is the check that stops a three-shot bow being aimed as three
guns.

### Hit claims — the flight, not the instant

```
slack = HitBox + 2.5
accept if the bullet's whole path came within slack of the monster
   or of where the monster was, up to 8 ticks ago
```

The **path** is checked rather than the position at the claimed moment, because the moment is the
client's word too and there is no sense weighing one claim against another — but the path follows
from the origin and angle, which are now checked and cannot be revised.

**A refused claim is not a strike.** The comment is explicit about why: refusing already takes the
whole prize, so striking bought nothing and cost a session every time the check was wrong — which it
had been, on an ordinary sword against ordinary monsters.

The bullet's age is logged with every refusal, because the likely cause is not cheating: **bullet ids
are a 256-entry ring**, so a hit arriving late for bullet 103 is matched against whatever bullet 103
has since become.

### Moving — clamped, never refused

Each sample is measured from **where the previous packet left the player**, not from the sample before
it, because a packet carries eleven samples and chaining them would hand out the slack eleven times.

```
allowed = speed * 1.5 * dt / 1000 + 1
```

A move beyond it is **clamped, not rejected**: rejecting would rubber-band anyone with a bad
connection, and the honest reading of a long jump — packets held and released together — is already
covered by measuring `dt` in the client's own clock, which does not stop during a lag spike.

Only the **endpoint** of a packet is worth a strike, so one impossible move reads as one strike rather
than eleven.

Movement is not judged at all for three seconds after arriving in a world, and `GrantMoveGrace`
clears the trail whenever something legitimately teleported the player.

## Ground damage

Ground damage cannot simply be moved to the server: the roll comes off a **shared random stream that
both sides step in lockstep**, so rolling it in both places puts every later shot's damage prediction
out. The server therefore waits well past the client's own half-second cadence and only then rolls it
itself — drawing exactly once either way.

## What the original *did* check: `TimeCop`

Read from `realm/entities/player/Player.AntiCheat.cs`. This part is original.

The one thing the original policed was **fire rate**, and it did it by comparing the client's clock
against the server's over a rolling window of 20 shots.

```
Push(clientTime, serverTime) keeps a ring of 20 deltas of each
TimeDiff() = sum(client deltas) / sum(server deltas)
```

Below `0.92` is `CLIENT_TOO_SLOW`, above `1.08` is `CLIENT_TOO_FAST`, and **while fewer than 20
samples have been taken it returns exactly `1`** — so the first twenty shots of a session are never
judged. That is the whole speed-hack window, and it re-opens on every reconnect.

`ValidatePlayerShoot` also checks, in order:

- the item being fired is the one in slot 0 (`ITEM_MISMATCH`),
- `time >= lastClientTime + 1 / attackFrequency / rateOfFire` (`COOLDOWN_STILL_ACTIVE`) — note the
  integer cast, so a weapon whose interval is under a millisecond gets `dt = 0`,
- a volley that stopped short of `NumProjectiles` before a new one began
  (`NUM_PROJECTILE_MISMATCH`).

The clock is only pushed on the **last shot of a volley**, so a three-shot weapon contributes one
sample per volley and the twenty-sample window covers sixty shots.

`IsNoClipping` is the other check: it asks whether the player is standing on an occupied tile. It only
logs — the caller decides what to do, and nothing about the move that got them there is examined. A
client that walks through a wall and off the other side is never on an occupied tile at the moment it
is sampled.

## What this server does differently

Ours is server-authoritative throughout, so most of this has no counterpart and needs none. Four
things are worth carrying:

- **The strike model** — 12 in a 10-second window, account untouched, log written for a human. We have
  this in `strikes.rs` for refused moves; the original counts four separate categories into one
  window.
- **Clamping rather than refusing** movement. We do this.
- **The bullet-id ring is a real hazard.** 256 ids per owner, and a late claim matched by id alone
  meets a different bullet. Our projectiles are generational handles, which removes the class of
  problem — worth keeping in mind as a reason not to "optimise" back to a byte.
- **A window that starts unjudged is a window that is never judged.** `TimeCop` returning `1` until it
  has 20 samples is the shape to avoid: any warm-up period in a check is an exploit with a reconnect
  in front of it. Ours should be conservative from the first sample rather than silent.

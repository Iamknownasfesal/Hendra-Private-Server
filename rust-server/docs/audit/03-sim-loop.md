# `hendra-sim`, the loop and the entities

Against [page 04](../mechanics/04-entities.md), [page 09](../mechanics/09-worlds.md),
[page 12](../mechanics/12-entity-kinds.md), [page 17](../mechanics/17-collision-index.md) and
[page 30](../mechanics/30-the-server-loop.md).

The arithmetic is right. Every stat formula matches the original to the constant:

| | Original | Ours |
| --- | --- | --- |
| Movement | `4 + 5.6 * (speed / 75)` | same |
| Slowed | flat `4`, overriding Speedy | same, and in the same order |
| Paralyzed | `0` | `rooted` |
| Attack multiplier | `0.5 + (att / 75) * 1.5` | same |
| Attack frequency | `0.0015 + (dex / 75) * 0.0065` | same |
| Health regen | `6 + vit * 0.12`, Sick zeroes the vitality only | same |
| Magic regen | `0.5 + wis * 0.06`, Quiet stops it | same |
| Tick rate | **6/s** as shipped (20 is only the code default) | 20/s |
| A late tick | skipped, never queued | `MissedTickBehavior::Skip` |

What differs is when things are run, and which things.

## There is only one clock, and the original has two

`FLLogicTicker` runs two loops:

- **Every `MsPT`**, `TickOneWorld` — entities, behaviours, projectiles.
- **Every 200 ms or more**, `World.Tick` — world timers, dungeon logic, and *every player's own tick*.

`MsPT` is `1000 / tps`, and the shipped `tps` is **6**, so that is 167 ms rather than the 50 ms an
earlier draft of this page assumed — 20 is only the default in `ConfigModels.cs`. The two clocks are
therefore 167 ms and 200 ms, which is nearly the same clock. We run the fast one at 50 ms, so this
server simulates at **more than three times the original's granularity**, and the gap between the
two loops that this page treats as a scheduling difference is much smaller than it looked.

The slow one is guarded: `if (_worldTask == null || _worldTask.IsCompleted)`. A slow tick still
running when the next is due is not queued, and the elapsed time it would have covered is folded into
the next one that does run.

Everything here runs on the 50 ms tick. For rates that scale by elapsed time — regeneration, effect
timers — that is equivalent and smoother. It is not equivalent for anything the content was tuned
against at 167 ms, and it is not equivalent for `WorldTimer`, which
[page 30](../mechanics/30-the-server-loop.md) records as firing at `> period` and discarding the
overshoot: at 200 ms granularity a 1,500 ms timer fires at 1,600, and here it fires at 1,500.

That is in our favour and still a difference. Whether to keep it is a decision, not an oversight —
but it is currently an undocumented one.

## Every enemy thinks, everywhere, all the time

`World.TickLogic` does not tick every enemy:

```csharp
foreach (var i in EnemiesCollision.GetActiveChunks(PlayersCollision))
    i.Tick(time);
```

`GetActiveChunks` walks the chunks holding players and returns everything within `ACTIVE_RADIUS = 3`
chunks of one, where `CHUNK_SIZE = 16`. An enemy more than about fifty tiles from the nearest player
does not run its behaviour at all. Decoys are ticked separately and unconditionally, which is the
only exception.

`World::think` filtered on `mind.is_some() && !dead && !paused` and nothing else, so every enemy in
the world thought twenty times a second. **Fixed** — the same chunk rule now gates it, and a live
comparison confirmed the original's: 500 enemies in a world with no player in it cost the C# server
`enemies 0.1%` of its loop, and moving a player in took it to `96%`.

Two consequences, and the second is the one that matters:

- **Cost.** A realm holds thousands of enemies and a handful of players. We run every one of them.
- **State advances while nobody is watching.** `TimedTransition`s fire, spawners spawn, `Decay`
  counts down, a boss walks its phases. In the original an unobserved enemy is frozen mid-state and
  resumes when a player comes back. Here it does not.

A player who walks past a dungeon room and returns finds it in a different state in the two servers.
Anything built on "the fight starts when you arrive" — which is most of what
[page 44](../mechanics/44-how-a-dungeon-is-wired.md) describes — depends on this.

## `MoveTo2` should walk through walls

Of the sixteen behaviours that move their host, fifteen call `ValidateAndMove`, which stops at
terrain. `World::step` (`crates/sim/src/world.rs:923`) does the same, sliding along a wall rather
than stopping dead, and that is correct for all fifteen.

`MoveTo2` calls `Entity.Move` directly, with no terrain check, and so does nothing else that is still
in use — `Ported` and `TeleporttoTarget` have no uses at all
([page 43](../mechanics/43-the-behaviour-scripts.md)).

So `MoveTo2`'s 29 uses are meant to pass through walls, and here they are stopped by them. Sixteen of
those are Janus's keys, which move six tiles toward the boss *through the wall of the room*. Blocked,
they stack against it.

This one is worth deciding rather than fixing on sight: the original's behaviour is a consequence of
which method it happened to call, and Janus works either way. It is recorded so that whoever finds
keys piled against a wall knows why.

## The shared random stream does not exist here, and that is a cutover decision

[Page 36](../mechanics/36-the-shared-random-stream.md) describes a Park-Miller MINSTD generator the
client and server step in lockstep, seeded from `MapInfo.Seed`, so the client can predict damage
locally. Every extra or missing draw desynchronises everything after it.

Nothing in this server implements it, and for this server's protocol nothing should: there is no
`Damage` message at all. Health arrives in the snapshot, the server is authoritative, and a client
has nothing to predict. `Shot` carries position, angle, speed and lifetime, and no damage.

But the client still has one. `godot-client/src/Core/MinstdRandom.cs` is live, `GameSession.cs:296`
reseeds it from `mapInfo.Seed`, and its own documentation says the two rules are "reseed on every
MapInfo" and "draw in exactly the same order as the server". That client speaks the legacy RC4 packet
protocol — `Rc4.cs`, `PacketId.cs`, `ProtocolKeys.cs` — which is the C# server's, not ours.

So this is not a defect in either half. It is the shape of a decision the client cutover forces:
**either the client's local damage prediction is removed, or this server grows a per-client MINSTD
generator and draws from it in exactly the right places.** The first is much cheaper and fits the
protocol we have. It should be written down before the cutover rather than discovered during it.

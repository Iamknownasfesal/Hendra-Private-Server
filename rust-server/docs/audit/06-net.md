# `hendra-net`, and what a client is told

Against [page 16](../mechanics/16-sight.md), [page 19](../mechanics/19-what-a-client-is-told.md),
[page 37](../mechanics/37-the-wire.md) and [page 40](../mechanics/40-packets.md).

Pages 37 and 40 describe a wire this server does not speak and should not: a 5-byte header, RC4 in
both directions, RSA inside it for credentials, and 105 packet types. Ours is QUIC with TLS 1.3,
snapshots against a per-client baseline, and about forty messages. Comparing them field by field
would be comparing two answers to different questions.

What does compare is the *mechanics carried over the wire* — which entities a player is allowed to
know about. That is [page 19](../mechanics/19-what-a-client-is-told.md), and it is where the
divergence is.

## Sight is blocked everywhere here, and off by default there

The sight radius matches: `Player.Update.cs:64` has `Radius = 20`, and `SIGHT_RADIUS` is `20.0`.

The occlusion does not. `World::look_around` (`crates/sim/src/world.rs:4524`) walks a ray for every
candidate and drops anything a wall stands in front of, in every world, always. The comment beside it
argues the case well — "in a game where being seen means being shot" — and it is the right default
for a new server. It is not what the original does.

`Sight.GetSightCircle(Owner.Blocking)` picks one of **four** algorithms per world:

| `blocking` | Algorithm | Worlds in this content |
| --- | --- | --- |
| 0 | `CalcUnblockedSight` — the whole circle, walls ignored | Marketplace, **and every world built in code** |
| 1 | `CalcBlockedRoomSight` | 25 |
| 2 | `CalcBlockedLineOfSight` | **none** |
| 3 | `CalcRegionBlockSight` | 3 |

Two things follow from that table.

**The mode we implement is the one nothing uses.** A ray walk from the viewer to the target is
`CalcBlockedLineOfSight`, mode 2, and no `.jw` in the content selects it. The 25 worlds that block
sight use room-based occlusion, which is a different shape: you see the room you are in, not a cone
of clear line.

**The default is off.** `World.cs:147`, in the base constructor:

```csharp
Blocking = 0; // toggles sight block (0 disables sight block)
```

Only a world loaded from a `.jw` proto overrides it. The Realm, the Nexus, the Vault, guild halls and
everything else constructed in code keep the default, so in the original **a player in the realm sees
every enemy within twenty tiles regardless of what is between them.** Here they do not.

That matters most in the realm, where the whole loop is spotting something worth walking to. It also
interacts with [the quest arrow](04-sim-combat.md): the arrow points at an enemy the player may now
have no way to see.

This is not a bug to fix on sight — a per-world sight mode is a feature, and choosing to block sight
everywhere is defensible. It is recorded because it is currently an accident rather than a decision:
nothing in the tree reads a world's blocking mode, so no world can ask for anything else.

## What the snapshot does right

- Invisible entities are filtered server-side rather than by the client, with the reason given: a
  client told about something it should not see can be made to reveal it. The original does the same.
- A projectile costs one message for its whole life rather than a snapshot entry per tick, because
  its flight is a function of its start and its age. That is the same reasoning behind
  `Projectile.GetPosition` on [page 05](../mechanics/05-projectiles.md), arrived at independently.
- Terrain goes out in run-length-encoded strips, which is the only way a four-million-square map fits.

## The one protocol fact worth carrying forward

There is no `Damage` message. Health arrives in the snapshot and the server is authoritative, which
is why [the shared random stream](03-sim-loop.md) is absent and should stay absent — but the client
still has a MINSTD generator wired to a legacy `MapInfo.Seed`, and the cutover has to settle which of
the two designs it is keeping.

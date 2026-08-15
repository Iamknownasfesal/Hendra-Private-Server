# Projectiles and how a hit is decided

Read from `realm/entities/Projectile.cs`.

**A caution before anything else.** This file is not purely the original. Large parts of it — the
server-side sweep, the hit box constants, the deferred-hit accounting — were written by this project
into the C# server, and their doc comments say so. When matching behaviour, the parts to treat as
"the original" are the hit box, `Blocked`, and `ForceHit` — and the flight path, with one exception
noted under [Flight](#flight). The sweep is a design this project chose and can choose differently in
Rust.

## The hit box is a square, and it is half a tile

```
HitBox = 0.5     // half-width, in tiles
```

The comment is explicit that this is the client's `GameObject.radius_`, that in this fork it is half
a tile for **everything** regardless of the object's own data, and that the test is **per axis** —
a square, not a circle. Anything on the server that has to agree with the client must test the same
way:

```
hit  <=>  |dx| <= 0.5 and |dy| <= 0.5
```

Our simulation tests a radius: `grid.within(x, y, HIT_RADIUS)` with `HIT_RADIUS = 0.5`. That is the
circle **inscribed** in the original's square, so we miss every corner — `π/4` of the area, meaning
**about 21% of the hits the original would call**. A bullet passing 0.4 tiles off on both axes is a
hit there and a miss here.

The fix is a square test per axis, which is also cheaper than a distance.

## Who decides a hit, in the original

Nothing on the server ever decided that a *player* was hit. Damage to a player arrived only when the
client volunteered a `PlayerHit`. A client that never sends one takes no damage from anything that
shoots — the comment names this plainly as what godmode is.

That is why this project added a sweep. This server is server-authoritative instead, which is a
larger and better change; the point for parity is that **the original's numbers for damage to
players come from the client**, so any behaviour tuned against the original was tuned against that.

## Flight

`GetPosition(elapsed)` — covered in [the behaviours page](02-behaviours.md) under curved shots. The
distance travelled is `elapsed * Speed / 10000`, which is why the content's speed unit is tiles per
ten seconds.

**The wavy coefficient is the one part of the flight path that is this project's, not the shipped
game's.** The 2020 source reads `Angle + (Math.PI * 64) * Math.Sin(...)` — about 201 radians of
amplitude, some thirty full turns, which is not a wave at all. The C# here reads `Math.PI / 64`, a
wobble of roughly ±3°, and that is what the AS3 client draws and therefore what a player actually
dodges. Ours implements the divided form at `crates/sim/src/projectile.rs:213`, with a test at
`:826` holding the shot under 4° off its line across its whole lifetime. A one-character difference,
and it decides whether a bullet went through somebody.

## Blocked

What stops a bullet, mirroring the client:

```
off the map                          -> blocked
tile's ground is NoWalk              -> blocked
tile has no object                   -> clear
object is an Enemy                   -> clear      (a target does not stop a shot aimed at it)
object is EnemyOccupySquare          -> blocked
object is OccupySquare and the shot does not pass cover -> blocked
otherwise                            -> clear
```

Note the asymmetry: **the server's own projectiles have never collided with terrain at all** in the
original — the comment says so. Only this added sweep walks the terrain. So in the original, an
enemy bullet passes through walls server-side and breaks on them client-side, and the client's view
is the one that counted, because the client is what reported hits.

For a server-authoritative rewrite the right reading is the client's: bullets break on walls.

## Multi-hit and the spent flag

```
ForceHit(entity):
  if not MultiHit and already used and the target is not a player: ignore
  add to the hit set; if it was not already there, apply the hit
  mark used
```

Two details:
- The hit set means **one bullet can never hit the same entity twice**, even a multi-hit one.
- The "already used" check exempts players, so a spent bullet can still be reported as hitting a
  player. That is a consequence of the client reporting player hits.

## Projectile identity

From `Entity.CreateProjectile`: ids are a **byte per owner**, wrapping at 256, and creating a
projectile into an occupied slot **destroys the bullet already there**. An enemy firing more than
256 bullets inside one bullet's lifetime erases its own earliest shots.

## What this server does differently

- **Circle versus square hit box.** Ours uses a radius; the original and the client use a half-tile
  square tested per axis.
- **We decide player hits ourselves**, which the original could not. This is a deliberate improvement
  and the reason our anti-cheat surface is small; it also means enemy damage output against players
  is *higher* here than in an original server played by an honest client, because nothing is lost to
  unreported hits.
- **Our bullets stop on terrain**; the original's server-side ones did not. Matching the client is
  the right call, and it is worth knowing it is not what the original server did.
- We should confirm we never let one bullet hit the same entity twice, and that our projectile ids
  cannot collide within a lifetime.

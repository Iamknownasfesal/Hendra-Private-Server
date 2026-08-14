# What to fix, and in what order

Everything found by reading the 103 Rust files against the 44 mechanics pages, ordered by how many
players notice and how often.

The ordering is deliberate. A defect that touches every hit a player takes outranks one that touches
a dungeon, however wrong the dungeon is. Counts are from the content this server loads.

## First: things a player feels in the first hour

**1. The stat numbering.** [01](01-content.md)

Twenty-one of the game's twenty-four potions raise max HP. A Potion of Attack raises max HP; a Potion
of Mana raises defence. 619 of 690 worn item bonuses are dropped or land on the wrong stat.

The fix is one eight-row table applied at parse time in both `desc.rs` and `activate.rs`, so nothing
downstream ever sees a content number. Do not translate at the point of use — two call sites each
translating differently is why this survived.

**2. A player's defence is never read.** [04](04-sim-combat.md)

`resolve_hits` takes defence from the class descriptor, which is the level-one base and usually zero.
`Stats::defence()` exists and is called from nowhere. Armour, defence rings and levelling all do
nothing.

One line, and it compounds with the first: a player's defence bonuses are mostly dropped at load,
the survivors raise the wrong stat, and whatever survives is not read when they are hit.

**3. `Shoot` drops its acquire range.** [02](02-behaviour.md)

4,486 uses, every one of them positional. Enemies acquire at the 20-tile sense radius instead of
their own, which is often four to eight, so they snipe across the screen and nothing has a safe
distance.

**4. `Shoot` drops its cooldown offset.** [02](02-behaviour.md)

2,663 uses. A staggered pattern becomes one simultaneous volley, once, and then silence. This is what
a boss's pattern *is*.

## Second: things that change how a fight reads

**5. Everything that orbits, orbits the player.** [02](02-behaviour.md)

137 of 167 orbits name an entity to circle and we drop the name. The ring the player is meant to move
around becomes a ring that follows them.

**6. Area damage ignores invulnerability.** [01](01-content.md)

`explode` uses a second copy of the damage formula with a 0.15 floor instead of 0.25 and no
conditions at all. An invulnerable boss takes full damage from a spell. Delete the free function and
route through `Rules`.

**7. Spawned minions are worth full experience.** [04](04-sim-combat.md)

The original abandons the award for anything `Spawned` and defaults `givesNoXp` to true across 364
spawners. Here every minion is worth `max_hp / 10`, which is a standing farm next to any spawner.

**8. Wisdom does nothing for abilities.** [04](04-sim-combat.md)

`useWisMod` is parsed and read nowhere. 38 live abilities ignore it — every heal nova, every stat
aura. Wisdom currently affects only MP regeneration.

**9. Enemies think with nobody in the room.** [03](03-sim-loop.md)

The original ticks only enemies within three chunks of a player. Here every enemy in the world
thinks, so bosses walk their phases unobserved and a room walked past and returned to is in a
different state. Also the largest single cost in the tick.

## Third: worth doing, felt over a session

**10. No world-wide loot table.** [05](05-sim-worlds.md) — one line on `World.cs:27` gives every
enemy in the game a 3% tier-1 potion. It is the baseline potion economy and it is absent.

**11. A thrown object lands 700 ms early.** [02](02-behaviour.md) — the telegraph is a hardcoded
1,500 ms in the original and an unmatched argument name here, so all 389 uses take the 800 ms
default.

**12. Ground damage.** [01](01-content.md) — averaged instead of rolled, every tick instead of every
500 ms, on enemies as well as players, ignoring `Damaging`, `ProtectFromGroundDamage`, `Paused` and
`Invincible`.

**13. `Spawn` drops `initialSpawn`.** [02](02-behaviour.md) — 225 uses decide how many appear on
entering a state.

**14. Sight is blocked in every world.** [06](06-net.md) — the original picks one of four modes per
world and **defaults to none**, so the realm and every code-built world show everything within twenty
tiles. We ray-walk everywhere, which is the one mode no world selects. Decide this rather than fix
it: a per-world mode is the feature, and right now no world can ask for anything.

**15. `GenericActivate` does nothing.** [01](01-content.md) — 26 items say "nothing happens". It is a
fully specified condition-effect area, not an unknown id, and every argument is already parsed.

**16. The rank ladder is three rungs.** [07](07-server.md) — eight in the original. Handing out items
is currently as trusted as stamping a setpiece into a live world.

## Fourth: small, cheap, do them alongside something else

- **Projectile damage can roll its maximum.** [01](01-content.md) — remove the `+ 1.0`; both of the
  original's rolls are half-open.
- **Bag reach is two tiles, not one.** [04](04-sim-combat.md)
- **`Orbit` variance, `Follow` duration and cooldown, `Grenade` fixed angle, `SetAltTexture`'s
  animating form.** [02](02-behaviour.md) — 67, 53, 57, 20 and 5 uses.
- **Required drops.** [05](05-sim-worlds.md) — `numRequired` is passed three times in the whole
  content. Three guaranteed drops are merely likely.
- **Ocean Trench oxygen.** [01](01-content.md) — a whole dungeon's pressure, and only that dungeon.
- **The `fame_from_experience` comment.** [04](04-sim-combat.md) — the code is right and the comment
  above it describes a halving that does not happen. Correct the comment, not the arithmetic.

## Decisions rather than fixes

These are differences to settle deliberately, not defects to close.

- **`MoveTo2` walking through walls.** [03](03-sim-loop.md) — the original's behaviour is a
  consequence of which method it happened to call. Janus works either way.
- **No slow tick.** [03](03-sim-loop.md) — we are more precise than the 200 ms clock. Fine, and
  currently undocumented.
- **The shared random stream.** [03](03-sim-loop.md) — absent here and correctly so, because this
  protocol sends no damage. The client still has a MINSTD generator wired to a legacy `MapInfo.Seed`,
  and the cutover has to keep one design or the other.
- **Page 26's verification surface.** [07](07-server.md) — moot, not missing. This client cannot
  claim a hit, a shot origin or ground damage. Do not implement tolerances for inputs that do not
  exist.

## The structural fix, which outlasts the list

Every defect in the first two sections was invisible to the test suite *and* to a census that
reported 100%.

- `BEHAVIOURS: 0 of 9636 uses unimplemented (100.00% covered)` — counts constructor names. `Shoot`
  resolves to `Primitive::Shoot`, so it counts, and `Primitive::Shoot` has five fields where the C#
  has twelve parameters.
- The activate census counts activate names the same way, which is how `GenericActivate` reads as
  covered while doing nothing.
- The behaviour census counted commented-out code until this pass, which is how five constructs
  looked used while having no live use at all.

**A census that counts names will always report success.** The check worth building compares each C#
constructor's parameter list against what `compile.rs` consumes and fails the build on anything
neither read nor named in an explicit ignore list. That check would have caught items 3, 4, 5, 11 and
13 the day they were written, and it is the only item on this page that stops the next one.

The same discipline applies to the endpoint census, which says 40 of 40 and
[has not been checked](09-app-auth-transport.md).

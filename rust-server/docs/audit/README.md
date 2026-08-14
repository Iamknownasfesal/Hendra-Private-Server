# Reading our own server against the pages

The 44 pages in [`../mechanics/`](../mechanics/) were written by reading all 547 C# files. They record
what the original does. This directory records what **this** server does, measured against them, file
by file.

The distinction matters because of how the earlier findings were made. Every claim in the mechanics
pages about the Rust side came from looking up the one function already named — which finds a
divergence when you already suspect one and finds nothing when you do not. Three defects have been
found by reading the two implementations side by side, and all three were invisible to the test suite
and to the censuses:

- `Order` restarting the state of everything it ordered, once a second.
- Content ids matched by exact case where the original ignores case.
- An argument census that counts constructor *names* and so reports 100% while half of `Shoot`'s
  parameters are dropped.

That is the failure mode this pass exists to find: **built, tested, and wrong in a way nothing
measures.**

## Method

Read the Rust file. Read the C# it corresponds to. Where they disagree, establish which is right by
reading the original rather than by reasoning about it, and measure how much content the difference
touches. A divergence that affects three items and one that affects every potion in the game are not
the same finding, and the count is what tells them apart.

Where a census already exists, distrust it until its denominator is checked. Both of the ones this
project relies on were over-reporting: the behaviour census counted commented-out code, and the
argument census counts names.

## Pages

| Page | Crate | Files |
| --- | --- | --- |
| [01-content.md](01-content.md) | `hendra-content` | 13 |
| [02-behaviour.md](02-behaviour.md) | `hendra-behavior` | 9 |
| [03-sim-loop.md](03-sim-loop.md) | `hendra-sim`, the loop | the tick, entities, movement |
| [04-sim-combat.md](04-sim-combat.md) | `hendra-sim`, combat | damage, experience, abilities |
| [05-sim-worlds.md](05-sim-worlds.md) | `hendra-sim`, worlds | loot, the realm, setpieces |
| [06-net.md](06-net.md) | `hendra-net` | visibility, the wire |
| [07-server.md](07-server.md) | `hendra-server` | commands, verification, sessions |
| [08-store.md](08-store.md) | `hendra-store` | persistence, the economy |
| [09-app-auth-transport.md](09-app-auth-transport.md) | `app`, `auth`, `transport` | endpoints, the wire |
| **[10-what-to-fix.md](10-what-to-fix.md)** | — | **the ordered refactor list** |

## Findings, worst first

**All but one are now fixed** — marked ✅ below; the exception waits on the client cutover. The ordered list with the reasoning is [page 10](10-what-to-fix.md).

The ordered list with the reasoning is [page 10](10-what-to-fix.md).

| What | Reach | Where |
| --- | --- | --- |
| ✅ Every stat potion but Life raises the wrong stat | 24 potions, 690 item bonuses | [01](01-content.md) |
| ✅ A player's defence is never read, so armour does nothing | every hit on every player | [04](04-sim-combat.md) |
| ✅ Spawned minions are worth full experience | 364 spawners | [04](04-sim-combat.md) |
| ✅ Wisdom does nothing for abilities | 38 abilities | [04](04-sim-combat.md) |
| ✅ Area damage ignores invulnerability and uses the wrong floor | every grenade and spell | [01](01-content.md) |
| ✅ Ground damage is averaged, continuous, and burns enemies | every hazard tile | [01](01-content.md) |
| ✅ `GenericActivate` does nothing | 26 items | [01](01-content.md) |
| ✅ No world-wide loot table, so no baseline potion drops | every enemy in the game | [05](05-sim-worlds.md) |
| Ocean Trench has no oxygen *(waits on the client)* | one dungeon, entirely | [01](01-content.md) |
| ✅ Sight is blocked in every world; the original defaults to off | the realm and every code-built world | [06](06-net.md) |
| ✅ Everything that orbits, orbits the player | 137 of 167 orbits | [02](02-behaviour.md) |
| ✅ `Shoot` drops its acquire range and its stagger | 4,486 and 2,663 uses | [02](02-behaviour.md) |
| ✅ Projectile damage rolls can hit the maximum | every projectile | [01](01-content.md) |
| ✅ A thrown object lands 700ms before it should | 389 uses | [02](02-behaviour.md) |
| ✅ The staff rank ladder is three rungs where the original has eight | 54 ranked commands | [07](07-server.md) |
| ✅ Enemies think with nobody in the room | every enemy, every world | [03](03-sim-loop.md) |
| No slow tick, where the original has two clocks *(a decision, see page 03)* | world timers, dungeon logic | [03](03-sim-loop.md) |

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

## Findings so far, worst first

| What | Reach | Where |
| --- | --- | --- |
| Every stat potion but Life raises the wrong stat | 24 potions, 690 item bonuses | [01](01-content.md) |
| Area damage ignores invulnerability and uses the wrong floor | every grenade and spell | [01](01-content.md) |
| Ground damage is averaged, continuous, and burns enemies | every hazard tile | [01](01-content.md) |
| `GenericActivate` does nothing | 26 items | [01](01-content.md) |
| Ocean Trench has no oxygen | one dungeon, entirely | [01](01-content.md) |
| Projectile damage rolls can hit the maximum | every projectile | [01](01-content.md) |

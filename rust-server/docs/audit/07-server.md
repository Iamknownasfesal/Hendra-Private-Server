# `hendra-server`, sessions, commands and verification

Against [page 22](../mechanics/22-trading.md), [page 26](../mechanics/26-verification.md),
[page 29](../mechanics/29-connecting.md), [page 31](../mechanics/31-commands.md) and
[page 38](../mechanics/38-handlers.md).

Commands are complete: the census answers **95 of the 95 the original has**, across 94 of ours, with
aliases. The strike counter matches `Player.Verify` exactly — twelve strikes in a ten-second window
before the connection is cut, and nothing durable happening to the account.

## Most of the verification surface is moot here, and that is worth saying

[Page 26](../mechanics/26-verification.md) lists a dozen slack constants: `ShootOriginSlack 1.5`,
`ArcSlack 0.08`, `HitClaimSlack 2.5`, `HitClaimHistoryTicks 8`, `SilentHitsPerStrike 5`, and so on.
Only the movement ones have an equivalent here, and reading the protocol explains why rather than
indicting it.

The C# checks those things because its client supplies them. A `PlayerShoot` carries an origin and an
angle; an `EnemyHit` and a `GroundDamage` are *claims* the server has to find plausible. Every slack
constant on that page is the tolerance on believing a client.

This server's client cannot make any of those claims. `ClientMessage::Shoot` carries an **angle and
nothing else** — the origin is wherever the server already thinks the player is. Hits are resolved by
`Projectiles::resolve_hits` from the server's own projectile positions, and there is no hit message
for a client to send. There is no ground-damage message either.

So `ShootOriginSlack`, `ArcSlack`, `HitClaimSlack`, `HitClaimHistoryTicks` and `SilentHitsPerStrike`
have nothing to check, and implementing them would be implementing tolerances for inputs that do not
exist. Rate of fire *is* checked, because it is the one thing a client can still overstate by asking
too often.

Movement is the exception, and it is checked: a claim is advisory, `World::resolve_move` decides
where the player actually is, and a refusal feeds the strike counter. That is the same shape as the
original's.

**This is the one place in the audit where a gap against the pages is not a gap.** It is recorded so
nobody works down page 26 implementing checks against a client that cannot lie that way.

## The rank ladder is collapsed from eight rungs to three

`Command`'s `permLevel` in the original takes eight distinct values, and the 54 ranked commands are
spread across all of them:

| `permLevel` | Commands |
| --- | --- |
| 0 | 2 |
| 8 | 1 |
| 10 | 4 |
| 40 | 2 |
| 80 | 15 |
| 90 | 18 |
| 95 | 1 |
| 100 | 11 |

Ours has three: `Needs::Nobody`, `Needs::Moderator`, `Needs::Administrator`, resolved through
`Admin::may_mute` (≥10) and `Admin::may_ban` (≥20). The 94 commands split 43 / 7 / 44.

Everything above level 10 collapses into one tier. `/gimme` at 40, `/clearspawn` at 80, `/lootspawn`
at 90, `/grank` at 95 and `/setpiece` at 100 are all simply "Administrator" — so an account trusted
to hand out items is equally trusted to stamp a setpiece into a live world, and the eleven
most-privileged commands in the game sit behind the same check as the fifteen least.

The original's ladder is not decoration: 80 is "can moderate a world", 90 is "can create content in
one", 100 is "can change the map". Those are different amounts of damage.

The fix is a rank as a number rather than an enum with two thresholds, and the per-command levels are
already written down on [page 31](../mechanics/31-commands.md).

## Bugs in the original we correctly do not have

[Page 38](../mechanics/38-handlers.md) records several defects in the C# handlers. Checked, and none
of them are reproduced here, which is right:

- `Rank >= 0` as a guard in `InvSwapHandler` and `InvDropHandler` — always true, so the soulbound
  check it was meant to gate never runs.
- Dungeon completion credited on *entering a realm* rather than on finishing a dungeon.
- `/pause`'s body being unreachable behind `if (owner != null) return false`.

The one handler rule we do diverge on is bag reach, which is [on the combat page](04-sim-combat.md):
one tile in the original, two here.

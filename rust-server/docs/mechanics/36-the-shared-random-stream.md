# The shared random stream

Read from `wServer/wRandom.cs`, `realm/ConnectManager.cs`, `realm/StatsManager.cs`,
`realm/entities/player/Player.Ground.cs`.

There are **two** sources of randomness in the original, and they are not interchangeable.

- `System.Random`, used by everything in `logic/behaviors/`. Server-only, nobody predicts it.
- **`wRandom`**, one per client, seeded at connect and **stepped in lockstep by the client**. Only
  three things draw from it, and every one of them is a number the client also has to know.

## The generator

A Lehmer generator — the "minimal standard", multiplier 16807, modulus 2^31 - 1 — written as a
16-bit split so it works in 32-bit arithmetic without overflow:

```csharp
uint lb = 16807 * (_seed & 0xFFFF);
uint hb = 16807 * (uint)((int)_seed >> 16);
lb = lb + ((hb & 32767) << 16);
lb = lb + (uint)((int)hb >> 15);
if (lb > 2147483647)
    lb = lb - 2147483647;
return _seed = lb;
```

This has to be reproduced **exactly**, arithmetic and all, because the client runs the same code on
the same seed and expects the same sequence. A mathematically equivalent implementation is not
sufficient: the two shifts are **arithmetic** on a signed reinterpretation of an unsigned value, and
the reduction is one conditional subtraction rather than a modulo, so results differ from a textbook
Park-Miller at the boundary.

Two properties follow from the code:

- **Seed 0 is a fixed point.** `Gen()` returns 0 forever.
- The output range is `[0, 2147483647]` inclusive, so **`NextDouble()` can return exactly 1.0**
  (`Gen() / 2147483647.0`).

### Seeding

```csharp
var seed = (uint)((long)Environment.TickCount * conInfo.GUID.GetHashCode()) % uint.MaxValue;
```

Per connection, so it re-seeds on every world change. The seed is sent to the client in `MapInfo`,
which is what makes both sides agree.

## The three draws

| Draw | What it decides |
| --- | --- |
| `StatsManager.GetAttackDamage` | the base roll of every player weapon and ability shot |
| `Player.Ground` | ground damage from a damaging tile |
| `Player.DropNextRandom()` | nothing — it exists to step the stream |

`DropNextRandom` is the tell. Because both sides step the same stream, **any draw one side makes and
the other does not puts every later draw out of phase**, and every subsequent damage number
disagrees. So when the server takes a number the client did not, it must call `DropNextRandom` on the
client's behalf, and when the client takes one the server did not, the server must burn one.

That is also why ground damage cannot simply be moved server-side, as
[the verification page](26-verification.md) records: rolling it in both places draws twice.

### Damage

```csharp
NextIntRange(min, max) * GetAttackMult(isAbility)
```

`NextIntRange` is `min == max ? min : min + Gen() % (max - min)`, so the range is **half-open**: the
maximum damage a weapon lists is never rolled. The multiplier is applied afterwards and the result
truncated to `int`.

```
GetAttackMult(ability)  = 1
GetAttackMult(weapon)   = MinAttackMult + (attack / 75) * (MaxAttackMult - MinAttackMult)
                          x1.5 if Damaging
                          = MinAttackMult flat if Weak
```

Abilities do not scale with attack at all, and `Damaging` multiplies after the attack scaling rather
than before.

## `NextNormal` is broken and unused

```csharp
var j = Gen() / 2147483647;      // integer division on uint
var k = Gen() / 2147483647;
var l = Math.Sqrt(-2 * Math.Log(j)) * Math.Cos(2 * k * Math.PI);
```

`j` and `k` are integer divisions, so both are 0 for all but one possible output. `Math.Log(0)` is
negative infinity, and the method returns infinity. Nothing in the tree calls it.

## What this server does differently

We are server-authoritative and the client is not yet cut over, so the lockstep requirement is
inherited whether we like it or not. Three things to hold:

- **Reproduce `Gen()` bit for bit**, including the signed shifts and the conditional subtraction. A
  correct Park-Miller is the wrong answer.
- **`NextIntRange` is half-open.** A weapon's listed maximum damage never occurs. Using an inclusive
  range makes every weapon in the game slightly stronger than the original.
- **Every extra draw is a desync.** If we take a number for something the client does not know about,
  it has to come from a different generator, not this one.

Behaviour randomness has no such constraint and should stay on our own generator.

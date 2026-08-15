# The server loop

Read from `realm/FLLogicTicker.cs`, `realm/NetworkTicker.cs`, `realm/WorldTimer.cs`,
`realm/TickPhases.cs`, `realm/LogicTicker.cs`, `realm/SpatialStorage.cs`.

Two long-running tasks, started once by `RealmManager.Run()`. Everything else in the server is called
from one of them.

## The logic loop

```
MsPT = 1000 / tps
```

Each pass:

1. Drain the five pending-action queues, in priority order.
2. `ConMan.Tick` — the connection queue.
3. `Monitor.Tick` — portal names.
4. `InterServer.Tick` — the cross-server bus.
5. `TickWorlds1` — every world's `TickLogic`, and every 200 ms every world's `Tick`.
6. Flush every connected player's outgoing packets.

Then it sleeps for whatever is left of `MsPT` and starts again.

### The tick delta is history, not the future

```csharp
t.TickDelta = loopTime / MsPT;
loopTime += (int)(watch.ElapsedMilliseconds - t.TotalElapsedMs) - t.ElaspedMsDelta;
```

`loopTime` accumulates the amount by which passes have **overrun**, and `TickDelta` is how many whole
ticks of debt that is. So a pass that took 320 ms on a 200 ms budget makes the *next* pass report
`TickDelta = 1` and simulate 200 ms of catching up. The debt is reduced by exactly what was
simulated (`ElaspedMsDelta`), so it neither compounds nor is forgiven.

A `TickDelta > 3` logs `LAGGED!` with the measured TPS.

### Five priorities, and what they are for

```
Emergent  Destruction  Normal  Creation
```

Four names, but the array is **five** queues and `PendingPriority` has four members, so index 4 is
allocated and never used. Drained in declaration order, so destruction runs before creation: an
entity removed and an entity added in the same tick do not overlap.

The whole point is that anything mutating a world from outside the loop queues an action instead. A
callback that throws is logged and the drain continues — one bad action does not stop the tick.

### Worlds tick side by side

`TickWorlds1` runs `world.TickLogic` for every distinct world in parallel, falling back to a direct
call when there is only one (handing a single item to the thread pool costs more than doing it
inline). Worlds share almost nothing: each owns its entities, its collision map and its timers.

**Inside** a world is deliberately serial. Behaviours spawn, move and damage through structures with
no locking at all.

A world that throws is caught per world, so a dungeon whose behaviour faults does not stop the realm.

This is one of the places the C# server has already been changed by this project; the original ticked
worlds one after another.

### Two rates

`TickLogic` runs every pass. `Tick` runs at most every 200 ms, and only when the previous `Tick` task
has finished — it is fired onto the thread pool and skipped entirely if it is still running. So the
slow half of the world tick **drops work under load** rather than falling behind, and the fast half
never does.

`_worldTime.TickDelta` accumulates across passes so the slow tick still sees the true elapsed time
when it does run.

Note that `t.TickDelta` and `t.ElaspedMsDelta` are **overwritten** on the `RealmTime` struct before
starting the slow tick. `RealmTime` is a struct passed by value, so this only affects the copy the
slow tick sees, which is what is wanted, but it is written as though it were mutating shared state.

## The network loop

A `BlockingCollection` of `(client, client id, packet id, bytes)`, consumed by one thread. Packets
are parsed and dispatched **there**, not on the socket thread and not in the logic loop.

Two staleness checks before anything is parsed:

```csharp
if (client.State == Disconnected || client.Id != pending.Item2) continue;
```

The client id is captured at enqueue time, so a packet queued by a client that then disconnected and
reconnected into the same object is dropped. Without that, a slot reused between enqueue and dispatch
would have one session acting on another's packets.

A parse failure logs and sends the player a "Network Read Error" dialog rather than disconnecting.

**Shutdown requires `Terminating` first**, then enqueues a dummy `(null, 0, 0, null)` to wake the
blocking enumerator. The dummy is never dereferenced because the loop breaks on `Terminating` before
touching it — which is why the ordering is enforced with a thrown exception.

Note `Pendings` is `static`, so a second `NetworkTicker` in one process would share the queue. There
is only ever one.

## `WorldTimer`

The only scheduling primitive worlds have.

```csharp
_remain -= time.ElaspedMsDelta;
if (_remain >= 0) return false;
```

Strictly less than zero, so a timer of exactly 200 ms on a 200 ms tick fires on the **second** tick,
not the first. Off-by-one-tick by construction, and consistent everywhere.

The overshoot is **discarded**, not carried: `Reset()` restores the full period rather than
`_remain += _total`. A repeating timer therefore drifts by up to one tick each time it fires. For a
timer used to schedule events, that is a slow accumulating lag; for a timer used for a cooldown, it is
what you want.

Two constructor overloads: an `Action` timer always reports fired, a `Func<bool>` timer reports
whatever the callback returns. Callers use the return value to decide whether to remove the timer, so
a `Func` timer can ask to be kept.

## `TickPhases`

This project's addition, not original. A stopwatch scope per named phase, summed and reported as a
percentage of the wall-clock window — **and only while the loop is lagging**, because a server keeping
up does not need telling what it spent its spare time on.

The comment records two mistakes worth not repeating: averaging per entry rather than per window had
a phase that runs once per world looking six times cheaper than it was, and dividing milliseconds by
seconds had it reporting 99,000%.

## Dead code

Two files in this directory are referenced by nothing:

- **`LogicTicker.cs`** — the predecessor of `FLLogicTicker`, with a different and worse debt
  calculation (it flushes players *before* ticking worlds, so every packet is one tick stale) and a
  `TickWorlds2` fixed-step variant that was never wired up. Nothing constructs it.
- **`SpatialStorage.cs`** — a 16-tile hash grid keyed `(x/16 << 16) | (y/16)`. Nothing constructs it
  either; the live spatial index is `Collision.cs`. It also has two real bugs, which is evidence it
  was never used: `Remove` indexes `store[hash]` without checking, and `Move` computes the source
  bucket from `entity.X/Y` — correct only if the caller has not already moved the entity.

Neither should be ported.

## What this server does differently

- We tick worlds in parallel and each world serially, which matches the C# **as it stands now**. The
  parallelism is this project's own change to it; the shipped 2020 server ticked worlds one after
  another, as the section above records.
- **The 200 ms slow tick that skips rather than queues** is worth having: it is the difference between
  a loaded server running slow and a loaded server falling permanently behind.
- **Packet dispatch on its own thread with a captured client id** is the guard against a reused
  connection acting on a stale packet. Ours uses generational handles, which covers the same ground.
- `WorldTimer` firing at `> period` rather than `>= period`, and discarding overshoot, is a real
  behavioural detail. Anything timed against it in the content is calibrated to that.
- The pending-action priorities matter in one specific way: **destruction is drained before creation**.

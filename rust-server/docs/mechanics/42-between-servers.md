# Between servers

Read from `common/ISManager.cs`, `InterServerChannel.cs`, `ISDataTypes.cs`, `ConfigModels.cs`,
`NReader.cs`, `NWriter.cs`, `TimedLock.cs`, `DbStatus.cs`, `Interfaces.cs`, `PrivateMessages.cs`,
`ChangePassword.cs`, `Ranks.cs`, and `wServer/discord/SendWebHook.cs`.

This completes `common/`.

## The bus is Redis pub/sub carrying JSON

```csharp
Publish<T>(channel, val, target = null)  ->  {InstId, TargetInst, Content} as JSON
AddHandler<T>(channel, handler)          ->  Subscribe, deserialise, drop if TargetInst != mine
```

Three channels — `Network`, `Control`, `Chat` — named by their enum's `ToString()`. Every server
subscribes to all three and filters by `TargetInst` on receipt, so a targeted message still reaches
every process and is discarded by all but one.

`T` is constrained to `struct`, which is what makes a missing field deserialise to zero rather than
null.

**A publisher receives its own messages.** Nothing filters on `InstId == InstanceId`; the handlers
that need to (chat, mostly) check it themselves when deciding whether an object id means anything.

## Server discovery

```
PingPeriod     2000 ms
ServerTimeout 30000 ms
```

On construction a server publishes `Join`. Anyone who sees a `Join` from an unknown instance
**replies with its own `Join`**, which is how a newly started server learns about the ones already
running without a registry.

Every 2 seconds each server publishes `Ping` carrying its whole `ServerInfo` — name, address, port,
player count, queue length, and **the full player list**. So the character-select screen's server list
is at most 2 seconds stale, and the cost is that every player's name and world crosses the bus twice a
second.

A server not heard from for 30 seconds is dropped and a `ServerQuit` event raised locally. `Dispose`
publishes `Quit`, so a clean shutdown is immediate and a crash takes 30 seconds.

`Tick` runs on a `System.Timers.Timer` at the ping period and then checks `_lastPing < PingPeriod`
before doing anything — the timer and the guard measure the same interval, so the guard only ever
fires on clock jitter.

## Configuration

One JSON file per process, deserialised into `ServerConfig`. Every field has a default, so an empty
file starts a working localhost server.

```
dbInfo          127.0.0.1:6379, no auth, index 0
serverInfo      type World, "Localhost", port 2051, maxPlayers 100, minRank 0, adminOnly false
serverSettings  tps 20, mode Single, key "B1A5ED", maxConnections 256,
                maxPlayers 100, maxPlayersWithPriority 120, enableMarket true
```

**`tps` is 20 only as a code default.** `ConfigModels.cs:60` declares `public int tps { get; set; }
= 20`, and that is the number an earlier draft of this page reported. The shipped
`wServer/wServer.json` overrides it to **6**, and git shows it has said 6 since the file was added.

So the running server's `MsPT` is **167 ms**, not 50. That changes the reading of
[the loop page](30-the-server-loop.md): the "fast" tick at 167 ms and the world tick at 200 ms are
nearly the same clock, rather than one running four times as often as the other. Measured live —
booting this server with 500 enemies and one player has it reporting 96% of wall time in the enemy
phase at that rate.

That measurement is of the C# as it stands, which is not the C# as shipped: the percentages come from
this project's `TickPhases`, the 500 enemies from its `HENDRA_STRESS` hook, the worlds are ticked in
parallel, and every enemy bullet is swept against players — none of which the 2020 server does. The
`tps 6` above is the shipped value and is unaffected.

`key` is the RC4 client key from [page 37](37-the-wire.md), and its default is in this file.

Four server modes decide which worlds are created: `Single` (nexus hub plus a realm), `Nexus`,
`Realm`, `Marketplace`.

`ServerInfo.playerList` is a `PlayerList` backed by a `ConcurrentDictionary<PlayerInfo, int>` keyed by
**reference**, so removing a `PlayerInfo` requires the identical object — which is why
`RealmManager.Disconnect` keeps the instance it got from `TryRemove`.

## Byte order is big-endian, everywhere

`NReader`/`NWriter` override every multi-byte read and write with `IPAddress.NetworkToHostOrder`, and
reverse the bytes by hand for `float` and `double`. Strings are length-prefixed:

```
WriteUTF    int16 length + UTF-8 bytes;  a null string writes a length of 0
Write32UTF  int32 length + UTF-8 bytes
```

`ReadUTF` on a null-written string returns `""`, not null — so the two do not round-trip exactly.

`ReadNullTerminatedString` casts each byte to `char` rather than decoding UTF-8, so it is Latin-1
only; it is used exclusively for the Flash policy handshake.

## `TimedLock`

Every `lock` in the codebase that matters goes through this: `Monitor.TryEnter` with a **1-second**
timeout, throwing `LockTimeoutException` if it cannot be taken.

The comment records the original was 10 seconds; the live value is 1. So a contended lock does not
deadlock the server, it throws — and the throw surfaces wherever that call was made, which for a
behaviour is the per-world catch in the tick loop.

## Private messages

Stored as one JSON blob per account, so **every send rewrites the whole mailbox**. `PrepareForSend`
resolves sender and recipient names at read time rather than storing them, and sorts newest first.
Deletion matches on `ReceiveTime`, which means two messages received in the same second cannot be told
apart.

`NeedsFix()` is `OwnerAccountId == 0` — a migration marker for mailboxes written before the field
existed.

## `SendWebHook`

Posts an embed to a Discord webhook on bans, unbans and deaths, with a static `HttpClient` and a
per-call `SetSuccessWebHook` that writes to a `static object` shared across every call — so two
concurrent posts race and one sends the other's body.

**The webhook id and token are hard-coded in the source.** They are live credentials committed to this
repository; anyone with the file can post to that channel. Nothing in the Rust server should carry
them over, and the existing ones should be rotated.

## What this server does differently

We are a single process, so most of this has no counterpart.

- **Big-endian on the wire** is inherited by the client we still ship; our own protocol does not need
  to be.
- **A 1-second lock timeout that throws** is a real design choice worth naming: it converts a deadlock
  into a caught exception and a lost tick, which is recoverable. Rust's ownership removes the class of
  problem for us.
- **A publisher receiving its own broadcast** is a shape to avoid if we ever add a bus.
- **Do not commit credentials.** The Discord webhook in `SendWebHook.cs` is a live secret in the tree.

# `hendra-app`, `hendra-auth`, `hendra-transport`, `hendra-characters`

Against [page 41](../mechanics/41-the-account-server.md) and
[page 42](../mechanics/42-between-servers.md).

## The account server

The endpoint census reports **40 of 40**, and unlike the behaviour and activate censuses it is not
obviously counting names: several rows say what an endpoint resolves to rather than claiming a route,
and one is honest about resolving to nothing —

```
ok   security/gameData    nothing: the original is an empty class
```

That row is right. Page 41 records `securityProtocols.cs` as registered in neither dictionary, and
`gameData` as an empty class.

**Caution rather than a finding:** every other census in this project turned out to over-report once
its denominator was checked. The behaviour census counted commented-out code; the argument census
counts constructor names. This one has not been checked the same way, and "40 of 40" should be
treated as unverified until somebody compares each route's *behaviour* against the original's rather
than its existence. Page 41's own list is 37 POST routes, which is a different number from 40 and may
be a different way of counting the same thing — or may not.

The one defect page 41 names is correctly absent: `FameList.FromDb` looks up its cache, assigns the
hit to a local, discards it, and returns an object whose `ToXml` is empty. Our leaderboard returns
its rows.

## Transport

`hendra-transport` is QUIC with TLS 1.3. Page 37's wire — a 5-byte header outside an RC4-encrypted
body, with RSA inside the RC4 for the `Hello` credentials — has no counterpart and needs none.

Two of page 37's mechanics do have counterparts and both are present in spirit:

- Outgoing traffic paced to the tick rather than sent as produced.
- A mismatched build being told to update rather than shown a failure dialog — ours refuses with a
  reason rather than dropping the connection silently.

## Between servers

Page 42 describes a Redis pub/sub bus with `PingPeriod 2000` and `ServerTimeout 30000`, a `tps: 20`
setting that yields the 50 ms fast tick, and a `TimedLock` with a one-second timeout that throws on
expiry.

The tick rate matches ([page 03](03-sim-loop.md)). The bus does not exist here because this server is
one process with worlds as tasks rather than several processes coordinating over Redis, which is why
`TimedLock` has no counterpart either — there is no cross-process lock to time out.

That is a genuine architectural simplification rather than a gap, and it removes a class of failure
the original documents: a `TimedLock` that throws after a second takes the world down with it.

## What is not covered

`hendra-characters` and `hendra-godot` are this project's own, with nothing on the other side to
compare them to. They were read; there is nothing to report against the pages.

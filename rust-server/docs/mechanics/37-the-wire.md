# The wire

Read from `networking/Client.cs`, `Client.KeepAlive.cs`, `RC4.cs`, `PolicyServer.cs`,
`IPacketHandler.cs`, and `networking/server/` (`Server`, `CommHandler`, `BufferManager`,
`ClientPool`, `SocketAsyncEventArgsPool`).

Not mechanics, but the shape of it explains several things the mechanics pages assume, and the
protocol is what the client this project still ships expects.

## The frame

```
0..3   int32, big-endian: total length including these five bytes
4      byte: packet id
5..    body, RC4 encrypted
```

`PrefixLength = 5`. A length under 5 or over `BufferSize` (0x20000, 128 KiB) is discarded and the
read state reset — **silently, and without disconnecting**, so a desynced stream just drops packets
until it happens to resynchronise on a plausible length.

One special value: a length of **1014001516** is the ASCII of `<pol` and triggers the cross-domain
policy response. That is a Flash-era handshake living inside the length field.

## Encryption is RC4 with two fixed keys

```
ServerKey  61 2a 80 6c ac 78 11 4b a5 01 3c b5 31
ClientKey  from config (serverSettings.key)
```

Two independent RC4 streams, one per direction, **never re-keyed within a connection** — and reset to
the same starting state on `Client.Reset()`, which happens when a pooled client object is reused.

RC4 with a fixed 13-byte key is not encryption in any meaningful sense; it is obfuscation the client
also knows. It matters only because a stream cipher **cannot tolerate a lost or truncated byte**: one
byte out and everything after it is garbage. That is the failure mode the `CommHandler` comment
describes.

## The send path, and the bug that was fixed in it

Three priority queues (`High`, `Normal`, `Low`), drained in order into a 1 MiB per-client buffer, then
written to the socket in 128 KiB chunks.

```csharp
s.BytesSent += e.BytesTransferred;
s.BytesAvailable -= e.BytesTransferred;
```

The comment records what it used to be: subtracting the running total from the remaining count, which
went **negative on the third chunk** of a large packet. A non-positive count reads as "done", so
anything over two buffers' worth was truncated mid-flight, the buffer reused for later packets, and
the client left decrypting unrelated bytes as the tail of the packet it had been promised. Exactly
the stream-cipher failure above.

After a queue drains, the next send is delayed by one tick (`Logic.MsPT`), so **outgoing data is
paced to the tick rate** rather than sent as fast as the socket will take it.

`FlushPending` re-enqueues a packet that would not fit and returns, so a packet is never split across
two flushes.

`IsLagging()` is "is a `NewTick` still sitting in a queue" — the client has not kept up with the tick
it was last sent. The two places that acted on it (dropping `Low` priority packets) are **commented
out**, so nothing currently uses it.

## Pooling

Every client object, its two `SocketAsyncEventArgs`, and its slice of one large pinned buffer are
allocated **once at startup** for `maxConnections + 1` clients, and recycled. A disconnect resets and
returns the object to the pool.

This is why `Client.Id` matters: a pooled object is reused by a different session, and
`NetworkTicker` captures the id at enqueue time so a packet queued by the previous occupant is
dropped. `Reset()` sets `Id = 0` explicitly, with a comment saying so.

`_maxConnectionsEnforcer` is a semaphore taken before each accept, so the listener blocks rather than
accepting a connection it has no client object for.

## Handler registration

Same reflection pattern as commands: every non-abstract `IPacketHandler` in the assembly is
constructed and added to a dictionary by its `ID`. **Two handlers claiming the same id throw at
startup.** An unknown id is ignored without a word.

A handler that throws logs and **disconnects with no `Failure` packet**, which the comment calls out:
to the player it is an unexplained closed connection, and it used to not even appear in the log.

## Keep-alive

The queue's keep-alive, not the in-world one ([that is on the player](23-weapon-damage-and-the-rest.md)):

```
PingPeriod   3000 ms
DcThresold  15000 ms
```

`QueuePing` carries a serial (the ping's own timestamp) plus the client's position and the queue
length, and only a `QueuePong` with the matching serial refreshes the deadline. So the queue screen's
position counter and the liveness check are the same packet.

The first call initialises `_pingTime` to `now - PingPeriod`, so the first ping goes out immediately
rather than three seconds in.

## Disconnect saves

```
if (Character == null || Player == null || Player.Owner is Test)
    release the lock and stop
Player.SaveToCharacter()
refresh lastSeen unless hidden or being overridden
SaveCharacter(..., lockAcc: true)
release the lock
```

**A character in a `Test` world is never saved**, which is what makes the map editor safe to
experiment in. The account lock is released last, and `SaveCharacter` is conditioned on still holding
it — so a save that lost the lock does not overwrite whatever took it.

`SendFailure` and `SendFailureDialog` both `await Task.Delay(1000)` before disconnecting, to give the
packet time to leave. Both are `async void`, so nothing can wait for them.

The failure dialog is JSON carrying a `build` field: **if it does not match the client's build, the
client shows an "update your client" dialog instead of the message.** Any error sent to a
mismatched client is silently replaced.

## What this server does differently

QUIC with TLS 1.3 and length-delimited frames, so nearly all of this has no counterpart. Three things
are worth keeping in view:

- **A stream cipher makes a truncation bug into a total corruption bug.** We do not have that failure
  mode, which is one fewer class of "the client is showing nonsense" to chase.
- **Outgoing data paced to the tick** rather than sent eagerly. We batch per tick too, and should.
- **Handler dispatch keyed by a captured session id**, so a reused connection cannot act on a previous
  session's packet. Ours gets this from generational handles.

And the thing not to copy: **discarding a malformed frame and continuing.** A frame we cannot parse
means we no longer know where we are in the stream, and the honest response is to close the
connection.

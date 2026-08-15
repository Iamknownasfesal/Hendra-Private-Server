# The packets

**All 105 read**: `Packet.cs`, `PacketIds.cs`, `IncomingMessage`, `OutgoingMessage`, 53 incoming and
49 outgoing definitions.

## Registration

`Packet`'s static constructor reflects over the assembly and registers every non-abstract `Packet`
that is **not** an `OutgoingMessage`, keyed by `ID`. So:

- **Only incoming packets are dispatchable.** An outgoing id arriving from a client resolves to
  nothing and the packet is dropped.
- `Packets.Add` means **two incoming packets claiming one id throw at startup**.

Every packet also carries a `CreateInstance()` that returns a fresh one, because the registry holds a
prototype rather than a type.

## Framing and encryption

```csharp
Write(client, buff, offset):
    serialise the body to a MemoryStream
    if body + 5 does not fit in the remaining buffer, return 0
    copy the body to offset+5
    Crypt(client, buff, offset+5, bodyLength)      // RC4, direction-specific
    write (bodyLength + 5) as int32 big-endian at offset
    write (byte)ID at offset+4
```

Returning **0 when it does not fit** is what `CommHandler.FlushPending` reads as "put this packet
back and send what we have".

`IncomingMessage.Crypt` uses `client.ReceiveKey`; `OutgoingMessage.Crypt` uses `client.SendKey`. The
header — length and id — is **outside** the encrypted region, which is what makes framing possible at
all.

## The ids

90 values, `byte`-sized, running from 0 to 173 with large gaps. They are the legacy client's numbering
and are not ordered by anything: `USEITEM` is 1, `USEPORTAL` is 6, `HELLO` is 9, `FAILURE` is 0. No
two share a value.

The blocks added by later work are contiguous and at the top: 110-112 for the connection queue,
151/152 for key info, 165-170 for this fork's UI actions, and **171-173 for the vault**. The last
three are this project's own, not the fork's: they address the vault by chest and slot because the
chests stopped being entities in a world when they became rows in one panel, and `InvSwap` has
nowhere to put a slot that belongs to nothing visible. See [page 14](14-world-subclasses.md).

## Notable wire shapes

### `Hello` — RSA inside RC4

```
BuildVersion  UTF
GameId        int32
GUID          UTF, RSA-encrypted
Password      UTF, RSA-encrypted
Secret        UTF, RSA-encrypted
KeyTime       int32
Key           int16 length + bytes
MapJSON       int32 length + UTF
```

Three fields are RSA-encrypted **on top of** the RC4 stream. That is the only asymmetric crypto in
the protocol, and it exists because the RC4 keys are fixed and shipped in the client, so credentials
would otherwise be readable by anyone with the binary.

`MapJSON` uses a 32-bit length prefix — it carries a whole test map.

### `Move` — the position trail

```
ObjectId int32, TickId int32, Time int32, NewPosition, then int16 count of TimedPosition
```

`TimedPosition` is `(int32 clientTime, float x, float y)`. This trail is what
[the verification page](26-verification.md) is built on, and the original parsed and discarded it.

### `Damage` — and a shift that wraps

```csharp
for (byte i = 1; i < 255; i++)
    if ((Effects & (ConditionEffects)(1 << i)) != 0)
        eff.Add(i);
```

`ConditionEffects` is a `ulong` with 51 flags, but `1 << i` is an **`int`**, and C# masks an int shift
count to 5 bits. Three consequences:

- **The loop starts at 1**, so `Dead` (bit 0) is never reported.
- `i = 31` produces `int.MinValue`, which widens to `0xFFFFFFFF80000000` — so it matches if **any** of
  bits 31 through 63 is set, and reports the target as `SlowedImmune` whenever any boost or immunity
  is active.
- `i = 32..50` wrap to bits 0..18, so a target that is `Quiet` is also reported as `DazedImmune`.

Everything above bit 30 is therefore wrong on every damage packet. The `Read` side is fine — it shifts
by a value read from the wire, which never exceeds 50 — so this only affects what the client is told.

### `EnemyShoot` — one packet for a whole volley

```
BulletId, OwnerId, BulletType, StartingPos, Angle, Damage, NumShots, AngleInc
```

`NumShots` and `AngleInc` mean a five-shot enemy volley is one packet and five consecutive bullet ids,
which is why bullet ids advance in blocks and why the 256-entry ring wraps as fast as it does.

`ServerPlayerShoot` has no `NumShots`: a server-authored player shot is always one bullet, and it
carries `Damage` where `AllyShoot` (purely cosmetic, sent to bystanders) does not.

### `ObjectStats` — the delta format

```
Id int32, Position, int16 count, then per stat:
    byte statType
    UTF if the stat is Guild or Name, otherwise int32
```

Two stats are strings and everything else is a 32-bit integer. The writer accepts `int`, `string`,
`bool` (as 1/0) and `ushort` (widened), and **throws on anything else** — which is the check that
catches a stat added with the wrong type.

`Update` carries tiles, new objects and dropped object ids; `NewTick` carries a tick id, a tick time
and the stat deltas. Both are `int16`-counted, so **65,535 is the hard limit on entities in one
update**.

### `Death` writes its fields in a different order than it lists them

```csharp
public int ZombieId; public int ZombieType; public bool IsZombie;
...
wtr.Write(ZombieType);
wtr.Write(ZombieId);
```

Declared id-then-type, written type-then-id, and `IsZombie` is never written at all. Read and write
do agree with each other, so it works; the field order is just misleading.

### `Failure` — five kinds

```
4  ClientUpdateNeeded
5  MessageWithDisconnect
6  MessageWithImmediateReconnect
7  NoMessageDisconnect
8  JsonDialogDisconnect
```

`JsonDialogDisconnect` carries a JSON body with a `build` field, and **a mismatched build makes the
client show an update prompt instead of the message** — see [the wire page](37-the-wire.md).

## Packets that cannot be written

Three incoming packets throw from `Write`:

```csharp
throw new InvalidOperationException("Nope, no write client packetzzzz :D");
```

`LaunchRaid`, `MarketCommand` and `UnboxRequest`. Four more — `AlertNotice`, `MarkRequest`,
`QoLAction`, `UpdateAck` — have an **empty** `Write`, so they serialise to nothing rather than
failing. Since only the client sends them, neither matters in practice; but it means those four
cannot be round-tripped in a test.

## What this server does differently

Our wire is a different protocol entirely, so this page is a reference for the client we still ship
rather than a specification to match. Four things carry over:

- **The header must be outside the encrypted region.** Ours is, by construction.
- **A whole volley in one message.** We send per-projectile; batching a volley is a real saving on a
  boss with a 20-shot pattern and costs nothing in fidelity.
- **`int16` counts everywhere** put a 65,535 ceiling on a single update. Ours should not inherit an
  arbitrary one.
- **Do not build a bitmask loop out of `1 << i` when the mask is 64 bits wide.** The C# bug above is
  exactly the shape that survives review because the read side looks correct.

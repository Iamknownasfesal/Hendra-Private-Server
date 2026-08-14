# Commands

Read from `realm/commands/Command.cs`, `UnrankedCommands.cs`, `RankedCommands.cs`.

## Registration is by reflection, and it is fragile

`CommandManager` walks **every type in the assembly**, takes anything assignable to `Command`, and
constructs it with either `()` or `(RealmManager)`. The name and the alias both go into one
case-insensitive dictionary.

Two consequences:

- **A duplicate name or alias throws at startup**, because `Dictionary.Add` is used rather than the
  indexer. The server does not start.
- A nested command class is picked up the same as a top-level one. `SpawnCommand` contains a nested
  `SpawnBCommand` (`lootspawn` / `ls`, rank 90) which is a near-copy of its parent with a different
  rank check and no `Invisible` effect on what it spawns. It is registered, and reads as an accident.

`Execute` splits on the **first space**: everything before is the name, everything after is one
argument string. There is no argument parsing in the framework — each command parses its own tail,
and most do it with a regex or an `IndexOf(' ')`.

Every command is wrapped in a `try`. A throwing command tells the player "Error when executing the
command" and logs; it never takes the server down.

## Permission is a single integer

```
0    ordinary player
10   donor
20   donor (per /rank's help text)
40   ...
70   former staff
80   GM
90   dev
100  owner
```

`HasPermission` is `account.Rank >= PermissionLevel`, checked before `Process`. `ListCommand: false`
hides a command from `/commands` without changing who may run it.

`/commands` (not `/help`, which the client intercepts) lists the distinct commands the caller may run,
sorted by name. Aliases are hidden because the list is `Distinct()` on the command objects.

## The unranked set

Movement and worlds: `/nexus`, `/realm`, `/vault`, `/marketplace`, `/tutorial`, `/ghall`, `/gland`
(hard-coded to realm tile 1512, 1048), `/tp <name>`, `/pos`, `/world`, `/who`, `/online`, `/uptime`.

Chat: `/tell` (`/t`), `/g` (`/guild`), `/l`, `/ignore`, `/unignore`, `/lock`, `/unlock`.

Guild: `/join`, `/invite` (`/ginvite`, needs guild rank 20), `/gkick`, `/gwho` (`/mates`).

Market: `/market <slot> <price>`, `/marketall` (`/mall`), `/mymarket`, `/rmarket <id>`, `/oops`.

Other: `/trade`, `/pause`, `/spectate`, `/lefttomax`, `/ps`, `/currentsong`, `/time` ("Time for you to
get a watch!!"), `/removeOverride`.

### Details worth keeping

- **`/tp` bypasses the client's cooldown UI**, and the file says so: typing `/teleport` works while
  the graphical teleport is still on cooldown, and the graphical one then fails and resets its own
  timer. A real exploit, documented in a doc comment and left in.
- **`/pause` can never work.** It checks `if (owner != null) { "Can't pause in arena."; return
  false; }` — and `owner` is `player.Owner`, which is never null for a player in a world. So pausing
  is unreachable; only *unpausing* works, and only for a pause applied some other way.
- `/pause` also refuses within 8 tiles of an enemy, and is refused entirely while spectating.
- **`/market` adds 3 to the slot number** so that "slot 1" means inventory index 4 — the first
  non-equipment slot. `/marketall` starts its loop at index 4 for the same reason.
- **`/marketall` checks `Soulbound` and `/market` does not.** The same protection, present in one
  path and missing in the other.
- `/l` is the only chat command that goes through `CompareAndCheckSpam`, and one of two places that
  call `Owner.ChatReceived` — the other is `PlayerTextHandler`, so **enemies hear both ordinary say
  and local**, and neither whispers nor guild chat.
- `/spectate` on yourself clears the target and then schedules a 3-second timer to un-pause, guarded
  by re-checking that no new target was picked in the meantime.
- `/ignore`, `/unignore`, `/lock`, `/unlock` and `/gkick` all refuse outright in a `Test` world.

## The ranked set

| Rank | Commands |
| --- | --- |
| 8 | `/unlink` |
| 10 | `/size`, `/reskin`, `/glow`, `/donorshop` |
| 40 | `/gimme` (`/give`), `/max` |
| 80 | `/kick`, `/announce`, `/summon`, `/clearspawn`, `/cleargraves`, `/mute`, `/unmute`, `/ban`, `/banip`, `/unban`, `/clearinv`, `/visit`, `/hide`, `/rename`, `/unname` |
| 90 | `/spawn`, `/lootspawn`, `/eff`, `/killAll`, `/getQuest`, `/oryxSay`, `/summonall`, `/reboot`, `/rank`, `/music`, `/closerealm`, `/quake`, `/link`, `/gift`, `/setfame`, `/setgold`, `/setprestige` |
| 95 | `/grank` |
| 100 | `/setpiece`, `/debug`, `/killPlayer`, `/tq`, `/override`, `/warg`, `/compactLOH`, `/setstar` |

`/unlink` at rank 8 against `/link` at rank 90 is clearly a typo for 80, and both then re-check
`player.Rank < 80` inside, so the effect is only that a rank-8 account gets "Forbidden" instead of "No
permission".

`/level20` (`/l20`) and `/Set` are registered at **permission level 0** — anyone can max a character
to level 20 and hand themselves a full class kit. That is either deliberate for this server or a
mistake; either way it is not the original game's behaviour and should not be carried over silently.

### Spawning

`/spawn [count] <name>` resolves the name exactly, and failing that takes the **partial match with the
highest `MaxHP`** — so `/spawn dragon` gets the biggest thing whose id contains "dragon". Spawning is
deferred by a 3-second `WorldTimer`, capped at 500 per command, and broadcast as a red notification
plus a `#name` chat line before it happens.

Spawned entities get `Spawned = true`, which is what `/clearspawn` and `/killAll` key off, and
`/spawn` also applies a **permanent `Invisible`** to enemies it creates. `/lootspawn` does neither of
those two things, which is the actual difference between them.

The JSON form takes `{notif, spawns:[{name, hp, size, count, x[], y[], target}]}` with clamps: hp must
exceed the descriptor's, size 25–500, count 2–500, coordinates inside the map. A coordinate that fails
the clamp becomes **0**, not the player's position, because the array was zero-initialised — so an
out-of-range x puts the mob at the map edge.

`/clearspawn` and `/killAll` both loop up to **5 times** until the count stops changing, because
killing something can spawn something else. `/killAll` sets `Spawned = true` on everything it kills,
which suppresses the loot those kills would otherwise drop, and explicitly skips "Tradabad Nexus
Crier".

### Moderation

- **Mutes are by IP, not account**, stored as a Redis key with a TTL, and lifted by the key-expiry
  subscription in `DbEvents`. Muting mutes **every connected client on that IP** that is not an admin,
  and `/unmute` works by re-muting for 1 second so the same expiry path does the unmuting.
- `/mute` and `/banip` both refuse if the target's IP equals the caller's. `/mute` refuses admins
  outright; `/ban` refuses anyone of **equal or higher rank**.
- `/rank` **disconnects the target first**, and demoting an admin below 80 wipes the account.
- `/rename` and `/unname` take a global name lock with `while ((lockToken = db.AcquireLock(key)) ==
  null);` — a **bare spin with no backoff and no timeout**. Two renames at once busy-wait a core.
- `/hide` applies `Hidden` **and `Invincible`** together, and sets the flag on the server's own player
  list so `/who` and `/online` skip the account.

### Things that will throw

Several ranked commands index `SingleOrDefault(...)` and then dereference without a null check:
`/setfame`, `/setgold`, `/setprestige` and `/setstar` all do `target.Account...` on a player who may
be offline. The `try` in `Command.Execute` catches it, so the operator sees "Error when executing the
command" rather than a stack trace — which is why these have survived.

`/setfame` writes `TotalFame` and `Fame` on the *client's* account object but calls `FlushAsync` on
the *database's* copy before setting them, so **nothing is persisted**. Same shape in `/setgold` and
`/setprestige`.

`/currentsong` opens the mp3 off disk with TagLib on the calling thread, inside the logic tick.

## What this server does differently

Our commands live in `chat.rs` and are dispatched by name, and the `commands` census proves what
exists. The things worth carrying:

- **A rank integer, compared with `>=`, checked before the body runs**, and a hidden-from-listing flag
  that is not a permission.
- **Deferred spawning with a per-command cap** so a mistyped count cannot stall a tick.
- **The re-run-until-stable loop** in mass kills, bounded at 5 passes.
- **Mutes by IP with a TTL and an expiry callback**, rather than a field that has to be polled.

The things not to carry: `/level20` and `/Set` at rank 0, the `/tp` cooldown bypass, `/pause`'s
unreachable body, `/market` skipping the soulbound check that `/marketall` performs, and the
unbounded spin locks in `/rename`.

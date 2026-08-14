# Chat

Read from `realm/ChatManager.cs`, `realm/entities/player/Player.Chat.cs`.

Six channels, three of which cross servers.

| Channel | Reaches | Crosses servers |
| --- | --- | --- |
| Say | everyone in the world | no |
| Local | the world, within sight radius | no |
| Tell | one account, wherever it is | yes |
| Guild | one guild id | yes |
| Announce | everybody | yes (unless `local`) |
| Info | one account | yes |

Enemy speech (`Mob`) and Oryx (`Oryx`) are the same packet with a `#`-prefixed name and `NumStars = -1`,
which is how the client knows not to draw a player plaque.

## Emote gating happens first, and by string surgery

Five emotes are account-unlockable:

```
:whitebag:  :bluebag:  :cyanbag:  :rip:  :pbag:
```

Any of those in the text that the account does not own is **deleted from the message** with
`text.Replace(word, "")` — not refused, not escaped. If the message is then whitespace, nothing is
sent at all.

Two consequences worth noting. `Replace` has no count, so every occurrence goes. And the split is on
spaces while the replace is on the whole string, so `":rip:x"` survives the `Where` filter (it does not
end in `:`) but any bare `:rip:` elsewhere in the same message is removed.

This gate is applied in `Say`, `Local`, `Tell` and `Guild` — **not** in `Announce`, `Mob`, `Oryx` or
`SendInfo`.

## The filter list does not block, it isolates

```csharp
var filtered = manager.Resources.FilterList.Any(r => r.IsMatch(tp.Txt));
if (filtered)
    broadcast to players whose Account.IP == src.Account.IP
else
    broadcast to everyone
```

A message matching the profanity list is still **accepted, echoed to the sender, and logged** — it is
just delivered only to clients sharing the sender's IP. The sender sees their own message go through
and has no signal that nobody else did. The same shape is applied to `Tell`: the recipient only gets a
filtered whisper if they are on the sender's IP.

The log line carries a `*filtered*` marker so a human can see what happened.

## Colour and stars

```
Say     NameColor = Glow, or 0x123456   TextColor = 0xFFFFFF if glowing, else 0x123456
Local   both 0xAD85FF
```

`0x123456` is the client's sentinel for "use the default", so a player with no glow gets ordinary
colours and a player with one gets their glow on the name and white text. Local chat is a fixed
lavender regardless of glow.

Admins get an `@` prepended to the name, in addition to the `Admin` flag on the packet.

`BubbleTime = 5` for everything a player or enemy says; Oryx uses `0`, so his lines do not draw a
speech bubble over anything.

## Local range

```csharp
p.DistSqr(src) < Player.RadiusSqr
```

The same radius that decides what a player can see. Local chat is therefore exactly co-extensive with
sight, which is why it needs no separate tuning.

## Ignore lists are enforced on receipt, per channel

Every channel except `Announce` and `GuildAnnounce` filters on
`!p.Account.IgnoreList.Contains(src.AccountId)`. Ignoring is by **account id**, so it survives
character death and name changes.

Note `Announce` deliberately does not respect it — a server-wide announcement reaches everyone.

## Whispers and the checks before them

`Tell` resolves the target name to an account id, then requires:

- the id resolves (`ResolveId != 0`),
- **an account lock exists** — that is, the account is currently logged in somewhere,
- the account is not `Hidden`, unless the sender is an admin.

All three fail the same way, with a bare `false`, so the sender is told "player not online" for a
name that does not exist, a player who is offline, and a hidden admin alike. That is the right
direction: it does not leak which of the three it was.

The published message carries `SrcIP` so that the receiving server can apply the filter-list isolation
rule, and `ObjId` is replaced with `-1` when the message arrives on a different instance than it was
sent from — an object id only means something on its own server.

## Guild chat

Two forms. `Guild` is a player speaking, and carries their object id, stars and admin flag.
`GuildAnnounce` is the server speaking about someone (joins, promotions) and carries
`-1 / -1 / 0 / ""`, so it renders as a plain system line.

`GuildAnnounce` respects `acc.Hidden`: a hidden account's join is delivered only to admins.

A login announces "*name* has joined the game" to the guild **only if the account has not been seen
for 1800 seconds** (30 minutes). Reconnecting after a crash does not spam the guild.

## What this server does differently

- We have no guild system and no cross-server bus, so `Tell`, `Guild` and cross-instance `Announce`
  have no counterpart yet.
- **The filter list isolating rather than blocking** is a design worth keeping: the sender gets no
  feedback loop to tune against, which is the whole point.
- **Emote gating by string deletion** is the wrong shape for us — deleting a substring from arbitrary
  user text is how you get surprises. Refusing the message, or replacing the token with the literal
  text, both behave better and are indistinguishable to an honest player.
- Local chat must use the same radius as sight. If those two ever diverge, players hear things they
  cannot see the source of.
- The 30-minute quiet window on join announcements is worth copying whenever we add one.

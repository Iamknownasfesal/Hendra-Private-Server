# The account server

**All 48 files of `server/` read.** A separate process from the world server, sharing only Redis and
the resource folder.

## Shape

An `Anna` HTTP server bound to one port. Handlers are a **hand-written dictionary**, not reflection:
one `Get` entry and **37 `Post` entries**. Everything the launcher and the character-select screen
needs is a POST with a form-encoded body, and every response is XML.

```
POST body -> HttpUtility.ParseQueryString -> handler.HandleRequest(context, query)
```

The body is read with a **4,096-byte cap** on the POST path (the helper's own default is 50,000, and
the caller overrides it down). Anything longer is silently truncated mid-field.

Static files come from the resource folder's `web/` directory and are registered as `Get` entries at
startup, with `/` aliased to `index.html`. `InitWebFiles` throws if `/` is already registered, which
is the only guard against double initialisation.

`/account/rp` is the **only** route registered as both GET and POST, because it is a link in an email.

## Every response is deflated

`RequestHandler.Write` compresses by default and sets `Content-Encoding: deflate`. Three content types
exist: `text/plain`, `application/xml` and `image/png`. Four handlers precompute and cache their body
in a static at `InitHandler` time — `app/init`, `app/globalNews`, `weekQuest/getQuests`,
`inGameNews/getNews`, `dailyLogin/fetchCalendar` — so those files are read once at startup and never
again.

An unhandled exception anywhere responds `<Error>Internal server error</Error>` with status 500.

## Authentication is a credential pair on every request

```csharp
Database.Verify(query["guid"], query["password"], out acc)
```

There is **no session and no token**. Every request carries the email and password, and every handler
re-verifies. Combined with SHA-1 hashing ([page 34](34-persistence.md)), that is the whole auth model.

`char/list` is the exception that also **creates a guest account** when the GUID is unknown, which is
how a first-time player reaches the character screen.

Two handlers gate on `acc.RankManager` rather than admin: `account/rank` and the Discord
register/unregister pair.

`char/fame` requires **no credentials at all** — it takes an account id and character id and returns
the death record. That is deliberate, because it is how a death link is shared.

## What each route does

| Route | Notes |
| --- | --- |
| `char/list` | account, characters, class availability, max levels, news, and the server list |
| `char/delete` | under the account lock |
| `char/fame` | public; refuses "Character not dead" |
| `char/purchaseClassUnlock` | reads `Unlock.Cost` from the class descriptor |
| `account/register` | requires a valid email; converts a guest account in place |
| `account/verify` | returns the whole account XML |
| `account/setName` | 3-15 letters, **1,000 gold** for a second name |
| `account/purchaseCharSlot` | price and currency both from settings |
| `account/purchaseSkin` | checks unlock level, cost, `Restricted`, `UnlockSpecial` |
| `account/changePassword` | logs IP and account to a dedicated `PassLog` |
| `account/forgotPassword` | writes a GUID token to the account |
| `account/rp` | consumes the token, generates a new password, returns an HTML page |
| `account/verifyage` | a boolean |
| `account/rank`, `registerDiscord`, `unregisterDiscord` | rank-manager only |
| `account/sendVerifyEmail` | `<Error>Nope.</Error>` |
| `guild/getBoard`, `setBoard`, `listMembers` | board needs guild rank 20 |
| `privateMessage/send`, `list`, `delete` | publishes a refresh over the inter-server bus |
| `fame/list` | see below |
| `credits/getoffers` | a hard-coded placeholder XML |
| `credits/add` | `<Error>Nope</Error>` — the body is commented out |
| `friends/getList`, `getRequests` | empty XML, marked TODO |
| `app/init`, `globalNews`, `getServerXmls`, `getTextures`, `getLanguageStrings` | static or precomputed |
| `picture/get` | serves a cached texture or 404s; the fetch-from-upstream path is commented out |
| `weekQuest/getQuests`, `inGameNews/getNews`, `dailyLogin/fetchCalendar` | serve a file verbatim |

## Password reset generates the password server-side

```csharp
var password = CreatePassword(new Random().Next(8, 12));
```

8 to 11 characters from `[a-zA-Z0-9]`, drawn from `new Random()` — clock-seeded, so **two resets in
the same tick produce the same password**. The new password is then shown in an HTML page rather than
emailed; `forgotPassword` builds a reset link and **never sends it** (the SendGrid import is there,
the send is not), so the flow only works if an operator reads the log.

`resetPassword` dereferences `acc.PassResetToken` without a null check, so a request naming a
non-existent account id throws into the 500 handler.

## The leaderboard is not implemented

```csharp
public static FameList FromDb(Database db, string timeSpan, DbChar character)
{
    if (StoredLists.ContainsKey(timeSpan))
    {
        var fl = StoredLists[timeSpan];      // assigned and never used
    }
    var fameList = new FameList { _timeSpan = timeSpan };
    StoredLists[timeSpan] = fameList;
    return fameList;
}
```

`ToXml` emits `<FameList timespan="..."/>` with **no entries**. `FameListEntry` is fully written and
never constructed. The cache lookup assigns to a local inside an `if` and discards it. So the fame
leaderboard in this fork returns an empty list, always, and the `legends` Redis key
[the wipe path clears](34-persistence.md) is never written.

## Class availability has three states

```
"available"     a class with no unlock requirement, not yet played
"unavailable"   a Restricted class
"unrestricted"  a class with an entry in classStats — played, or bought
```

Availability is computed **once, statically, at startup** from the object descriptors, and only the
per-account `unrestricted` overlay is per request.

## `securityProtocols.cs`

161 lines that read twenty-odd query parameters named `LOESOFT_HASH`, `protocolToken`, `protocolID`,
`rateoffireValueData`, `arcgapValueData` and so on, hash them, and compare against hard-coded
constants — an attempt at client-integrity attestation from a different fork.

**It is not registered in either handler dictionary.** No route reaches it. `security/gameData.cs` is
an empty class. Both are dead.

## What this server does differently

We have no account server; login is part of the QUIC handshake. If one is built:

- **Do not send credentials on every request.** A token with an expiry is the minimum, and it removes
  the reason the password has to be recoverable in the first place.
- **`char/fame` being public is a real design decision**, not an oversight — a death page needs to be
  shareable. Keep it, but make it read a death record rather than a live character.
- **Precomputing and caching the static XML at startup** is worth copying; those five files are served
  from memory.
- **The fame list is not implemented at all.** If we build one, we are not matching anything.
- Do not generate passwords from a clock-seeded `Random`.

//! The app-engine endpoints, in the shapes the game's own clients ask for.
//!
//! The original splits the server in two: a world server on a socket, and an HTTP "app engine"
//! (`Server-Side/server/`) that owns accounts, character lists and everything a player reads before
//! they are playing. Both clients that exist speak to that second half directly -- the AS3 client
//! at `GetCharListTask.as:57` and `WebLoginTask.as:20`, and ours from the login screen, the account
//! panel and the death screen -- and they speak it in a particular dialect: a form-encoded POST
//! with `guid` and `password`, answered with an XML document, or with `<Error>why</Error>` when it
//! will not.
//!
//! So these are served rather than the clients being moved onto the JSON API beside them. The XML
//! shapes here are the specification's, element for element; a client written against the original
//! is a client that works against this. The JSON routes remain what the game socket's own sign-in
//! uses, and both end up in the same store through the same [`crate::sign_in`].
//!
//! # Names and addresses
//!
//! An account is registered with an email address and is *not* yet named: `Database.Register`
//! (`common/Database.cs:352-372`) writes a name off a fixed list of forty-five and leaves
//! `NameChosen` false, and the player picks a real one afterwards through `/account/setName`. The
//! two are different fields there. Here they are one column plus the address, and "has not chosen a
//! name yet" is read back off the name itself: the reserved list is exactly the set of names
//! `setName` refuses (`account/setName.cs:15`), so a name from it is one nobody can have chosen.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Form;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use hendra_auth::hash_password;
use hendra_store::{Account, StoreError};

use crate::App;
use crate::appfiles::{CHARACTER_SLOT_COST, CHARACTER_SLOT_CURRENCY};

/// How many characters an account may have alive at once.
///
/// `<MaxCharSlot>2</MaxCharSlot>` in the original's `XmlDatas/data/init.xml:38`, which is what
/// `Database.Register` copies onto a new account.
const CHARACTER_SLOTS: i32 = 2;

/// The names an unnamed account is given, and which no account may choose.
///
/// `Database.GuestNames` (`common/Database.cs:62-75`), used verbatim. They are reserved rather than
/// decorative: `setName` refuses them, which is what lets a name from this list mean "this account
/// has not picked one yet" instead of being a name somebody happens to have.
const GUEST_NAMES: [&str; 45] = [
    "Darq", "Deyst", "Drac", "Drol", "Eango", "Eashy", "Eati", "Eendi", "Ehoni", "Gharr", "Iatho",
    "Iawa", "Idrae", "Iri", "Issz", "Itani", "Laen", "Lauk", "Lorz", "Oalei", "Odaru", "Oeti",
    "Orothi", "Oshyu", "Queq", "Radph", "Rayr", "Ril", "Rilr", "Risrr", "Saylt", "Scheev", "Sek",
    "Serl", "Seus", "Tal", "Tiar", "Uoro", "Urake", "Utanu", "Vorck", "Vorv", "Yangu", "Yimi",
    "Zhiar",
];

/// Whether a name is one of the reserved ones, and so not a name anybody chose.
///
/// Trailing digits are ignored, because a reserved name already taken is handed out with a number
/// after it. A chosen name can never collide: `setName` takes letters only.
pub(crate) fn is_guest_name(name: &str) -> bool {
    let stem = name.trim_end_matches(|character: char| character.is_ascii_digit());
    GUEST_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
}

/// An XML document, served as one.
///
/// The original compresses these and labels them `deflate`; ours are sent plain. The clients read
/// either -- both inflate only if inflating works and fall back to the bytes as they arrived.
pub struct Xml(pub String);

impl IntoResponse for Xml {
    fn into_response(self) -> Response {
        (
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/xml; charset=utf-8"),
            )],
            self.0,
        )
            .into_response()
    }
}

/// The refusal shape every one of these endpoints shares.
///
/// Answered with 200 rather than a status code, as the original does. The clients read the body to
/// decide: a status code with an unparseable body is what "Root element is missing" was.
fn error(why: &str) -> Xml {
    Xml(format!("<Error>{}</Error>", escape(why)))
}

/// Escapes text for an XML text node or attribute.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Reads a field, trimmed, or the empty string.
fn field<'a>(form: &'a HashMap<String, String>, key: &str) -> &'a str {
    form.get(key).map(|value| value.trim()).unwrap_or("")
}

/// Checks the credentials a form carries.
///
/// The refusal wording is the original's: `LoginStatus.GetInfo()` answers "Bad Login" for a wrong
/// password and for an account that does not exist alike, which is deliberate -- telling them apart
/// turns the endpoint into a way to enumerate accounts.
async fn verified(app: &App, form: &HashMap<String, String>) -> Result<Account, Xml> {
    let guid = field(form, "guid");
    let password = field(form, "password");

    if guid.is_empty() {
        return Err(error("Bad Login"));
    }

    crate::sign_in(app, guid, password).await.map_err(|(status, refusal)| {
        match status {
            StatusCode::TOO_MANY_REQUESTS => error(&refusal.0.error),
            StatusCode::FORBIDDEN => error(&refusal.0.error),
            _ => error("Bad Login"),
        }
    })
}

/// `POST /account/verify` -- the account, if the password is right.
///
/// `account/verify.cs`. Carries nothing the character list does not; the login screen asks it first
/// because it is the endpoint that refuses a stranger, where `/char/list` in the original answers a
/// guest instead.
pub async fn verify(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    match verified(&app, &form).await {
        Ok(account) => {
            let unlocks = hendra_characters::Unlocks::load(&app.store, account.id)
                .await
                .ok();
            let graveyard = app.store.graveyard(account.id, GRAVESTONES_AS_NEWS).await;

            Xml(account_xml(
                &app,
                &account,
                unlocks.as_ref(),
                graveyard.as_deref().unwrap_or(&[]),
            )
            .await)
        }
        Err(refusal) => refusal,
    }
}

/// `POST /account/register` -- makes an account.
///
/// `account/register.cs`. The address becomes the account's sign-in name; the name it is *known* by
/// is chosen afterwards, so one is reserved from the guest list here and `/account/setName`
/// replaces it. No confirmation link stands between registering and playing.
pub async fn register(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let address = field(&form, "newGUID");
    let password = field(&form, "newPassword");

    // `Utils.IsValidEmail` guards the original's, and the client checks the same thing before it
    // asks, so this is the third fence rather than the first.
    if !is_valid_email(address) {
        return error("Invalid email");
    }

    if password.is_empty() {
        return error("Invalid password");
    }

    let hash = {
        let _permit = app.hashing.acquire().await;
        match hash_password(password) {
            Ok(hash) => hash,
            Err(err) => return error(&err.to_string()),
        }
    };

    // Taken before anything is written, so a second registration on the same address fails here
    // rather than leaving a half-made account behind.
    if app.store.account_by_email(address).await.is_ok() {
        return error("Duplicate Email");
    }

    let Some(account) = reserve_name(&app, address).await else {
        return error("try again shortly");
    };

    if let Err(err) = app.store.set_email(account.id, address).await {
        tracing::error!(%err, account = account.id, "could not record an address");
        return error("Duplicate Email");
    }

    if let Err(err) = app.store.set_password(account.id, &hash).await {
        tracing::error!(%err, account = account.id, "could not store a password");
        return error("try again shortly");
    }

    // A registered account is of age. `Database.Register` (`common/Database.cs:356`) writes
    // `AgeVerified = true` on every account it makes, and the AS3 client refuses to enter the game
    // when the character list says otherwise (`EnterGameCommand.as:43`) -- an account that cannot
    // play is not a registration.
    if let Err(err) = app.store.set_age_verified(account.id, true).await {
        tracing::warn!(%err, account = account.id, "could not record age verification");
    }

    tracing::info!(account = account.id, "registered");
    Xml("<Success />".to_string())
}

/// Makes an account under a reserved name that is free.
///
/// The original picks by hashing the address and does not care about collisions, because its names
/// are not unique. Ours are, so a taken one is tried again with a number after it -- still a name
/// `setName` would refuse, which is what keeps it readable as "unnamed".
async fn reserve_name(app: &App, address: &str) -> Option<Account> {
    let first = (address.bytes().map(u32::from).sum::<u32>() as usize) % GUEST_NAMES.len();

    for attempt in 0..GUEST_NAMES.len() {
        let reserved = GUEST_NAMES[(first + attempt) % GUEST_NAMES.len()];

        for suffix in 0..32 {
            let name = if suffix == 0 {
                reserved.to_string()
            } else {
                format!("{reserved}{suffix}")
            };

            match app.store.create_account(&name).await {
                Ok(account) => return Some(account),
                Err(StoreError::NameTaken) => continue,
                Err(err) => {
                    tracing::error!(%err, "could not create an account");
                    return None;
                }
            }
        }
    }

    None
}

/// `POST /account/setName` -- claims the name an account is known by.
///
/// `account/setName.cs`, including its rules: three to fifteen letters, nothing already taken, and
/// none of the reserved names. The first name is free and every one after it costs a thousand
/// credits, which is checked before the rename and charged in the same transaction as it -- the
/// original charges first and then renames in a loop that never gives up (`setName.cs:38-41`), so a
/// name taken in between costs the credits and spins.
pub async fn set_name(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let name = field(&form, "name").to_string();

    if name.chars().count() < 3
        || name.chars().count() > 15
        || !name.chars().all(|character| character.is_alphabetic())
        || is_guest_name(&name)
    {
        return error("Invalid name");
    }

    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal,
    };

    // `Account.NameChosen`, which a reserved name stands for here: an account wearing one has never
    // picked a name, so this is its first and free.
    let price = if is_guest_name(&account.name) {
        0
    } else {
        RENAME_CREDITS
    };

    match app
        .store
        .rename_account_for_credits(account.id, &name, price)
        .await
    {
        Ok(()) => Xml("<Success />".to_string()),
        Err(StoreError::NameTaken) => error("Duplicated name"),
        Err(StoreError::Refused(why)) => error(why),
        Err(err) => {
            tracing::error!(%err, account = account.id, "could not rename an account");
            error("try again shortly")
        }
    }
}

/// What a second name costs, in credits.
///
/// `account/setName.cs:38` and `:40`, where the check and the charge are the same number.
pub const RENAME_CREDITS: i32 = 1000;

/// `POST /char/list` -- the characters, the account and the server list.
///
/// `char/list.cs`. The name undersells it: this one document is everything the character-selection
/// screen draws, which is why the client asks for nothing else between signing in and playing.
///
/// One difference from the original, deliberately: a guid it has never seen is refused rather than
/// having a guest account built for it on the spot (`char/list.cs:38-39`). A guest that cannot then
/// log into the world server is a player stranded one screen further on than they started.
pub async fn char_list(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal,
    };

    let characters = app.store.characters(account.id).await.unwrap_or_default();

    // What this account has unlocked, loaded once. Three blocks below ask about it -- which classes
    // are offered, how far each has been taken, and how many stars that adds up to -- and asking
    // three times is how the picker and the level beside it end up disagreeing.
    let unlocks = hendra_characters::Unlocks::load(&app.store, account.id)
        .await
        .ok();

    // The original's per-account character counter, which is a stored field there and a count here.
    // It never goes down, so it is the number made rather than the number alive.
    let next_char_id = app
        .store
        .characters_made_before(account.id, i64::MAX)
        .await
        .unwrap_or(characters.len() as i64);

    let mut document = format!(
        "<Chars nextCharId=\"{next_char_id}\" maxNumChars=\"{CHARACTER_SLOTS}\">"
    );

    for summary in &characters {
        let Ok(character) = app.store.character(summary.id).await else {
            continue;
        };
        let tally = app.store.tally(summary.id).await.ok();
        document.push_str(&character_xml(&app, &character, tally.as_ref()));
    }

    let graveyard = app.store.graveyard(account.id, GRAVESTONES_AS_NEWS).await;
    let graveyard = graveyard.as_deref().unwrap_or(&[]);

    document.push_str(&account_xml(&app, &account, unlocks.as_ref(), graveyard).await);
    document.push_str(&class_availability_xml(&app, unlocks.as_ref()));
    document.push_str(&news_xml(&app, graveyard).await);

    document.push_str("<Servers>");
    for server in &app.servers {
        document.push_str(&format!(
            "<Server><Name>{}</Name><DNS>{}</DNS><Port>{}</Port><Lat>0</Lat><Long>0</Long>\
             <Usage>0</Usage><AdminOnly>false</AdminOnly></Server>",
            escape(&server.name),
            escape(&server.host),
            server.port
        ));
    }
    document.push_str("</Servers>");

    document.push_str(&item_costs_xml(&app));
    document.push_str(&max_class_level_xml(&app, unlocks.as_ref()));
    document.push_str("</Chars>");

    Xml(document)
}

/// How many gravestones the character list carries as news.
///
/// `CharList.GetItems` (`server/XmlModels.cs:600`) takes ten, which is what the AS3 client's
/// graveyard draws. Nothing shows an eleventh, so nothing reads one.
const GRAVESTONES_AS_NEWS: i64 = 10;

/// `<ClassAvailabilityList>`, which `ClassAvailability.ToXml` (`server/XmlModels.cs:502-513`)
/// defines.
///
/// Three words are possible. The original writes `unrestricted` for a class the account has an
/// entry for in its class-stats table -- meaning played or bought -- `unavailable` for one the
/// content marks `<Restricted/>`, and `available` for the rest. The AS3 client acts on all three:
/// an `unavailable` class is not drawn at all, an `available` one is drawn with its unlock
/// requirement, and only `unrestricted` is playable (`NewCharacterScreen.as:78-80`).
///
/// The account's side of that question is answered here by the same [`hendra_characters::Unlocks`]
/// the class picker and character creation use, so a class offered by this document is exactly a
/// class `create` will accept.
fn class_availability_xml(app: &App, unlocks: Option<&hendra_characters::Unlocks>) -> String {
    let mut element = String::from("<ClassAvailabilityList>");

    for class in app.catalog.classes() {
        let desc = app.catalog.object(class.object_type);
        let id = desc.map(|desc| desc.id.as_str()).unwrap_or_default();

        let availability = match unlocks {
            Some(unlocks) if unlocks.locked(&app.catalog, class).is_none() => "unrestricted",
            _ => "available",
        };

        element.push_str(&format!(
            "<ClassAvailability id=\"{}\">{availability}</ClassAvailability>",
            escape(id)
        ));
    }

    element.push_str("</ClassAvailabilityList>");
    element
}

/// `<MaxClassLevelList>`, which `MaxClassLevelList.ToXml` (`server/XmlModels.cs:562-575`) defines.
///
/// One self-closing element per class, in content order, carrying the best level that class has
/// ever reached on this account. The AS3 client gates which skins it shows as unlocked on these
/// numbers (`CharacterClass.as:46-48`), which is why it is a list of every class rather than only
/// the ones with progress.
fn max_class_level_xml(app: &App, unlocks: Option<&hendra_characters::Unlocks>) -> String {
    let mut element = String::from("<MaxClassLevelList>");

    for class in app.catalog.classes() {
        let level = unlocks
            .map(|unlocks| unlocks.best(&app.catalog, class.object_type).0)
            .unwrap_or(0);

        element.push_str(&format!(
            "<MaxClassLevel maxLevel=\"{level}\" classType=\"{}\" />",
            class.object_type.0
        ));
    }

    element.push_str("</MaxClassLevelList>");
    element
}

/// `<ItemCosts>`, which `ItemCosts` (`server/XmlModels.cs:516-540`) builds once at boot.
///
/// Every skin in the content with its price, whether it is rented rather than kept, and whether it
/// may be bought at all. The AS3 wardrobe hides a skin whose `purchasable` is zero
/// (`ParseCharListXmlCommand.as:39-58`); the shipped files restrict none, so all 191 are offered.
fn item_costs_xml(app: &App) -> String {
    let mut element = String::from("<ItemCosts>");

    for skin in app.catalog.skins() {
        element.push_str(&format!(
            "<ItemCost type=\"{}\" expires=\"{}\" purchasable=\"{}\">{}</ItemCost>",
            skin.object_type.0,
            i32::from(skin.expires),
            i32::from(!skin.restricted),
            skin.cost
        ));
    }

    element.push_str("</ItemCosts>");
    element
}

/// `<News>`, which `CharList.GetItems` (`server/XmlModels.cs:596-614`) fills.
///
/// Two sources merged and sorted newest first: whatever the server has posted, and the account's
/// own recent deaths rendered as gravestones. The second is the half a client acts on -- the AS3
/// graveyard draws only the items whose link names a character (`Graveyard.as:22-27`) -- so a death
/// is written in the exact words the original uses, since the title is the caption on the stone.
async fn news_xml(app: &App, graveyard: &[hendra_store::Departed]) -> String {
    let posted = app
        .store
        .news(false, GRAVESTONES_AS_NEWS)
        .await
        .unwrap_or_default();

    if posted.is_empty() && graveyard.is_empty() {
        return String::from("<News />");
    }

    let mut element = String::from("<News>");

    // Posted news first, since the read returns it newest first and nothing here is older than a
    // gravestone that has already been read off a descending query. It carries no icon or link:
    // the columns the original reads those from have no counterpart here, and an item with neither
    // is one the graveyard skips rather than one it draws wrongly. Its date is likewise left at
    // zero rather than guessed at.
    for news in posted {
        element.push_str(&format!(
            "<Item><Icon></Icon><Title>{}</Title><TagLine>{}</TagLine><Link></Link>\
             <Date>0</Date></Item>",
            escape(&news.title),
            escape(&news.body),
        ));
    }

    for death in graveyard {
        let class = app
            .catalog
            .type_of_uuid(death.class)
            .and_then(|object_type| app.catalog.object(object_type))
            .map(|desc| desc.id.clone())
            .unwrap_or_default();

        element.push_str(&format!(
            "<Item><Icon>fame</Icon><Title>Your {} died at level {}</Title>\
             <TagLine>You earned {} glorious Fame</TagLine><Link>fame:{}</Link>\
             <Date>{}</Date></Item>",
            escape(&class),
            death.level,
            death.final_fame,
            death.character_id,
            death.at.timestamp(),
        ));
    }

    element.push_str("</News>");
    element
}

/// The `<Account>` element, which `Account.ToXml` (`server/XmlModels.cs:332-360`) defines.
///
/// The element order is that method's, verbatim, because the AS3 client walks the children by name
/// but a handful of the flags are *presence* flags -- written empty when true and left out when
/// false -- and a reader that expects one at a position it never reaches gets a default instead of
/// a value.
async fn account_xml(
    app: &App,
    account: &Account,
    unlocks: Option<&hendra_characters::Unlocks>,
    graveyard: &[hendra_store::Departed],
) -> String {
    let named = !is_guest_name(&account.name);

    let verified_email = app
        .store
        .email_of(account.id)
        .await
        .ok()
        .flatten()
        .is_some_and(|(_, confirmed)| confirmed);

    let guild = match app.store.guild_of(account.id).await {
        Ok(Some((id, rank))) => app
            .store
            .guild(id)
            .await
            .ok()
            .map(|guild| (id, guild.name, rank as i32)),
        _ => None,
    };

    let age_verified = app.store.age_verified(account.id).await.unwrap_or(false);
    let last_seen = app.store.last_seen(account.id).await.unwrap_or(0);

    let mut element = format!(
        "<Account><AccountId>{}</AccountId><Name>{}</Name>",
        account.id,
        escape(&account.name)
    );

    if named {
        element.push_str("<NameChosen></NameChosen>");
    }
    if account.admin_rank > 0 {
        element.push_str("<Admin></Admin>");
    }

    // How far the account is trusted, not how far it has played. `Account.Rank` is `acc.Rank`
    // (`server/XmlModels.cs:308`), which `DbAccount.Rank` (`common/DbModels.cs:521-524`) resolves
    // to the higher of the staff rank and the Discord rank -- a staff ladder, nothing to do with
    // fame. The AS3 client reads it straight into `Player.rank` beside `Player.isAdmin`
    // (`SavedCharactersList.as:106-108`) and draws the two together on a name; it counts stars for
    // itself out of `<ClassStats>` (`:150-155`), which is why no star count is sent.
    element.push_str(&format!(
        "<Rank>{}</Rank><LastSeen>{last_seen}</LastSeen>",
        account.admin_rank
    ));

    if verified_email {
        element.push_str("<VerifiedEmail></VerifiedEmail>");
    }

    element.push_str(&format!(
        "<IsAgeVerified>{}</IsAgeVerified>",
        i32::from(age_verified)
    ));

    // Present until the account has buried somebody, because what it gates is the once-only
    // ancestor bonus. The original keeps a flag and clears it on the first death; the graveyard
    // already records that same fact, so it is read rather than kept twice.
    if graveyard.is_empty() {
        element.push_str("<isFirstDeath></isFirstDeath>");
    }

    // What another slot costs and in which coin. `<CharacterSlotCurrency>1</...>` is fame, which is
    // the balance `purchase_char_slot` charges against, so the price the client quotes is the price
    // the purchase enforces.
    element.push_str(&format!(
        "<Credits>{}</Credits><NextCharSlotPrice>{CHARACTER_SLOT_COST}</NextCharSlotPrice>\
         <CharSlotCurrency>{CHARACTER_SLOT_CURRENCY}</CharSlotCurrency>\
         <MenuMusic></MenuMusic><DeadMusic></DeadMusic>",
        account.credits
    ));

    element.push_str(&vault_xml(app, account).await);
    element.push_str(&stats_xml(app, account, unlocks));

    match guild {
        Some((id, name, rank)) => element.push_str(&format!(
            "<Guild id=\"{id}\"><Name>{}</Name><Rank>{rank}</Rank></Guild>",
            escape(&name)
        )),
        None => element.push_str("<Guild id=\"0\"><Name /><Rank>0</Rank></Guild>"),
    }

    element.push_str("</Account>");
    element
}

/// `<Vault>`, which `Vault.ToXml` (`server/XmlModels.cs:258-264`) defines.
///
/// One `<Chest>` per chest, each eight comma-separated item types with an empty slot written as -1,
/// and `<Vault />` when there are none.
///
/// One chest fewer than the account owns, which is the original's own arithmetic:
/// `Enumerable.Range(0, acc.VaultCount - 1)` (`server/XmlModels.cs:252`) passes a *count* where it
/// reads as an end, so the last chest is never mentioned. Its own world server disagrees with it and
/// stands up all `VaultCount` of them (`wServer/realm/worlds/logic/Vault.cs:96`). Kept because the
/// reference is the specification and this is what it answers — measured against the pristine app
/// server, an account with `vaultCount` 1 gets `<Vault />` and one with 3 gets two `<Chest>`
/// elements.
async fn vault_xml(app: &App, account: &Account) -> String {
    let chests = account.vault_chests.max(0).saturating_sub(1);
    if chests == 0 {
        return String::from("<Vault />");
    }

    let contents = app.store.vault(account.id).await.unwrap_or_default();

    let mut element = String::from("<Vault>");
    for chest in 0..chests {
        let slots: Vec<String> = (0..VAULT_CHEST_SLOTS)
            .map(|slot| {
                contents
                    .iter()
                    .find(|(held, _)| *held == chest * VAULT_CHEST_SLOTS + slot)
                    .and_then(|(_, item)| app.catalog.type_of_uuid(*item))
                    .map(|found| found.0 as i32)
                    .unwrap_or(-1)
                    .to_string()
            })
            .collect();

        element.push_str(&format!("<Chest>{}</Chest>", slots.join(", ")));
    }
    element.push_str("</Vault>");
    element
}

/// How many items one vault chest holds.
const VAULT_CHEST_SLOTS: i16 = 8;

/// `<Stats>`, which `Stats.ToXml` (`server/XmlModels.cs:227-237`) defines.
///
/// A `<ClassStats>` for every class the account may play, then the three fame totals. The original
/// writes one entry per row in the class-stats table, and a row appears there exactly when a class
/// is unlocked -- so "has an entry" and "may be played" are the same question, and it is asked here
/// of the same [`hendra_characters::Unlocks`] the class picker uses.
fn stats_xml(app: &App, account: &Account, unlocks: Option<&hendra_characters::Unlocks>) -> String {
    let mut element = String::from("<Stats>");
    let mut best_char_fame = 0;

    if let Some(unlocks) = unlocks {
        for class in app.catalog.classes() {
            let (best_level, best_fame) = unlocks.best(&app.catalog, class.object_type);

            // A class the account has never played and may not play yet has no entry, which is how
            // the original's class-stats table behaves. Progress on its own is enough: a class can
            // hold a record and then be locked again behind a prerequisite, and dropping the entry
            // would lose the record rather than the lock.
            if unlocks.locked(&app.catalog, class).is_some()
                && (best_level, best_fame) == (0, 0)
            {
                continue;
            }
            best_char_fame = best_char_fame.max(best_fame);

            element.push_str(&format!(
                "<ClassStats objectType=\"0x{:04x}\"><BestLevel>{best_level}</BestLevel>\
                 <BestFame>{best_fame}</BestFame></ClassStats>",
                class.object_type.0
            ));
        }
    }

    // `TotalFame` is the fame the account has ever earned and `Fame` what it still has left to
    // spend, which is the pair `Database.UpdateFame` keeps: a credit raises both, a debit only the
    // second.
    element.push_str(&format!(
        "<BestCharFame>{best_char_fame}</BestCharFame><TotalFame>{}</TotalFame><Fame>{}</Fame>",
        account.total_fame, account.fame
    ));

    element.push_str("</Stats>");
    element
}

/// The tally a character has run up, in the blob `<PCStats>` carries.
///
/// `FameStats.Write` (`common/FameStats.cs:78-110`) writes one five-byte record per counter: the
/// counter's number, then its value in network order. Both clients read exactly that -- ours at
/// `Account/CharacterStats.cs:78-84`, which is what fills the in-game character panel's rows.
///
/// The numbers are the original's, so a counter nothing records is written as zero rather than
/// omitted: leaving it out would slide every later row onto the wrong label.
fn fame_stats_blob(tally: Option<&hendra_store::TallyRow>) -> String {
    use base64::Engine;

    let count = |read: fn(&hendra_store::TallyRow) -> i32| -> i32 {
        tally.map(read).unwrap_or(0)
    };

    let counters: [(u8, i32); 21] = [
        (0, count(|tally| tally.shots)),
        (1, count(|tally| tally.shots_that_hit)),
        (2, count(|tally| tally.abilities_used)),
        (3, count(|tally| tally.tiles_seen)),
        (4, count(|tally| tally.teleports)),
        (5, count(|tally| tally.potions_drunk)),
        (6, count(|tally| tally.monster_kills)),
        // Assists are the original's own counters and nothing here records one, so they are the
        // zeroes `char/fame` already reports rather than a number invented to fill the slot.
        (7, 0),
        (8, count(|tally| tally.god_kills)),
        (9, 0),
        (10, count(|tally| tally.cube_kills)),
        (11, count(|tally| tally.oryx_kills)),
        (12, count(|tally| tally.quests_completed)),
        (13, 0),
        (14, 0),
        (15, 0),
        (16, 0),
        (17, 0),
        (18, 0),
        (19, count(|tally| tally.level_up_assists)),
        (20, 0),
    ];

    let mut bytes = Vec::with_capacity(counters.len() * 5);
    for (id, value) in counters {
        bytes.push(id);
        bytes.extend_from_slice(&value.to_be_bytes());
    }

    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The `<Char>` element, which `Character.ToXml` (`server/XmlModels.cs:423-455`) defines.
fn character_xml(
    app: &App,
    character: &hendra_store::Character,
    tally: Option<&hendra_store::TallyRow>,
) -> String {
    let object_type = app
        .catalog
        .type_of_uuid(character.class)
        .map(|found| found.0)
        .unwrap_or(0);

    let class = app.catalog.class(hendra_content::ObjectType(object_type));

    // A stat the row has no record of falls back to what the class starts with, which is what the
    // column deliberately does not guess at for itself.
    let stat = |index: usize| -> i32 {
        character
            .stats
            .get(index)
            .copied()
            .flatten()
            .unwrap_or_else(|| class.map(|class| class.stats[index].starting).unwrap_or(0))
    };

    // Equipped slots only, in slot order, with an empty slot written as -1. The client splits on
    // commas and draws them in order, so the holes have to be there.
    let equipment: Vec<String> = (0..hendra_characters::EQUIPPED_SLOTS)
        .map(|slot| {
            character
                .inventory
                .iter()
                .find(|(held, _)| *held == slot)
                .and_then(|(_, item)| app.catalog.type_of_uuid(*item))
                .map(|found| found.0 as i32)
                .unwrap_or(-1)
                .to_string()
        })
        .collect();

    format!(
        "<Char id=\"{}\"><ObjectType>{object_type}</ObjectType><Level>{}</Level><Exp>{}</Exp>\
         <CurrentFame>{}</CurrentFame><Equipment>{}</Equipment>\
         <MaxHitPoints>{}</MaxHitPoints><HitPoints>{}</HitPoints>\
         <MaxMagicPoints>{}</MaxMagicPoints><MagicPoints>{}</MagicPoints>\
         <Attack>{}</Attack><Defense>{}</Defense><Speed>{}</Speed><Dexterity>{}</Dexterity>\
         <HpRegen>{}</HpRegen><MpRegen>{}</MpRegen>\
         <Tex1>{}</Tex1><Tex2>{}</Tex2><Texture>{}</Texture><PCStats>{}</PCStats>\
         <HealthStackCount>{}</HealthStackCount><MagicStackCount>{}</MagicStackCount>\
         <Dead>{}</Dead><HasBackpack>{}</HasBackpack></Char>",
        character.id,
        character.level,
        character.experience,
        character.fame,
        equipment.join(","),
        character.max_hp,
        character.hp,
        character.max_mp,
        character.mp,
        stat(2),
        stat(3),
        stat(4),
        stat(5),
        stat(6),
        stat(7),
        character.dye_cloth,
        character.dye_accessory,
        character.skin,
        fame_stats_blob(tally),
        character.health_potions,
        character.magic_potions,
        !character.alive,
        i32::from(character.has_backpack),
    )
}

/// `POST /char/fame` -- what a dead character did with its life.
///
/// `char/fame.cs`. Asked by the death screen, which learns the two ids from the Death packet that
/// put it on screen. A living character has no tally to give, and the original says so in the same
/// words.
pub async fn char_fame(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let Ok(account_id) = field(&form, "accountId").parse::<i64>() else {
        return error("Invalid character");
    };
    let Ok(character_id) = field(&form, "charId").parse::<i64>() else {
        return error("Invalid character");
    };

    let Ok(character) = app.store.character(character_id).await else {
        return error("Invalid character");
    };

    if character.account_id != account_id {
        return error("Invalid character");
    }

    if character.alive {
        return error("Character not dead");
    }

    // The graveyard is read most-recent-first and this is asked the moment somebody dies, so the
    // death being looked for is at the front of it.
    let Some(death) = app
        .store
        .graveyard(account_id, 100)
        .await
        .ok()
        .and_then(|deaths| {
            deaths
                .into_iter()
                .find(|death| death.character_id == character_id)
        })
    else {
        return error("Character not dead");
    };

    let tally = app.store.tally(character_id).await.ok();
    let count = |read: fn(&hendra_store::TallyRow) -> i32| -> i32 {
        tally.as_ref().map(read).unwrap_or(0)
    };

    let name = app
        .store
        .account(account_id)
        .await
        .map(|account| account.name)
        .unwrap_or_default();

    let mut document = String::from("<Fame>");

    // The character element the list returns, with the account name grafted on, which is where the
    // death screen reads whose character it was.
    let mut element = character_xml(&app, &character, tally.as_ref());
    element = element.replace(
        "</Char>",
        &format!("<Account><Name>{}</Name></Account></Char>", escape(&name)),
    );
    document.push_str(&element);

    document.push_str(&format!(
        "<BaseFame>{}</BaseFame><TotalFame>{}</TotalFame>",
        character.fame, death.final_fame
    ));

    document.push_str(&format!(
        "<Shots>{}</Shots><ShotsThatDamage>{}</ShotsThatDamage>\
         <SpecialAbilityUses>{}</SpecialAbilityUses><TilesUncovered>{}</TilesUncovered>\
         <Teleports>{}</Teleports><PotionsDrunk>{}</PotionsDrunk>\
         <MonsterKills>{}</MonsterKills><MonsterAssists>0</MonsterAssists>\
         <GodKills>{}</GodKills><GodAssists>0</GodAssists>\
         <CubeKills>{}</CubeKills><OryxKills>{}</OryxKills>\
         <QuestsCompleted>{}</QuestsCompleted><LevelUpAssists>{}</LevelUpAssists>\
         <MinutesActive>0</MinutesActive>",
        count(|tally| tally.shots),
        count(|tally| tally.shots_that_hit),
        count(|tally| tally.abilities_used),
        count(|tally| tally.tiles_seen),
        count(|tally| tally.teleports),
        count(|tally| tally.potions_drunk),
        count(|tally| tally.monster_kills),
        count(|tally| tally.god_kills),
        count(|tally| tally.cube_kills),
        count(|tally| tally.oryx_kills),
        count(|tally| tally.quests_completed),
        count(|tally| tally.level_up_assists),
    ));

    // What each bonus was called, why it was paid and what it paid, in the order they were awarded.
    // Read off the death rather than worked out again: these are what the character was actually
    // paid, and the death screen subtracts them from the total to say how much of it was earned by
    // living. The original serves the same three parts (`XmlModels.cs:729-734`), and the client
    // adds them onto the base fame to arrive at the total (`TotalFame.as:16-30`).
    for bonus in &death.bonuses {
        document.push_str(&format!(
            "<Bonus id=\"{}\" desc=\"{}\">{}</Bonus>",
            escape(&bonus.name),
            escape(hendra_sim::fame::describe(&bonus.name).unwrap_or_default()),
            bonus.fame
        ));
    }

    document.push_str(&format!(
        "<CreatedOn>{}</CreatedOn><KilledBy>{}</KilledBy></Fame>",
        death.at.timestamp(),
        escape(&death.killed_by)
    ));

    Xml(document)
}

/// `POST /guild/listMembers` -- who is in the asking account's guild.
///
/// `guild/listMembers.cs`. Answers the guild the caller is in rather than one named in the request,
/// which is all the original does with it too.
pub async fn guild_members(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal,
    };

    let Ok(Some((id, _))) = app.store.guild_of(account.id).await else {
        return error("Not in guild");
    };

    let Ok(guild) = app.store.guild(id).await else {
        return error("Not in guild");
    };

    let mut document = format!(
        "<Guild id=\"{}\"><Name>{}</Name><Level>{}</Level><Fame>{}</Fame><Board>{}</Board><Members>",
        guild.id,
        escape(&guild.name),
        guild.level,
        guild.fame,
        escape(&guild.board)
    );

    for member in app.store.guild_members(id).await.unwrap_or_default() {
        document.push_str(&format!(
            "<Member><Name>{}</Name><Rank>{}</Rank><Fame>0</Fame><LastSeen>0</LastSeen></Member>",
            escape(&member.name),
            member.rank as i32
        ));
    }

    document.push_str("</Members></Guild>");
    Xml(document)
}

/// A body served with a content type of its own.
///
/// The XML endpoints all answer alike and [`Xml`] covers them; these are the handfuls that do not.
/// `RequestHandler.Write` labels a body `text/plain` whatever is in it, `WriteXml` labels it
/// `application/xml` and `WriteImg` `image/png` (`RequestHandler.cs:26-70`), and the difference is
/// visible to a client that branches on the header.
pub struct Blob(pub Vec<u8>, pub &'static str);

impl IntoResponse for Blob {
    fn into_response(self) -> Response {
        ([(header::CONTENT_TYPE, HeaderValue::from_static(self.1))], self.0).into_response()
    }
}

/// The one refusal that is not a body but a status.
///
/// `Program.OnError` answers 500 with `<Error>Internal server error</Error>` for anything a handler
/// throws (`Program.cs:120-131`), and several handlers throw for input a client can send: a
/// `fame/list` with no timespan, a `char/delete` with an unparseable id. Those are answered here
/// the same way rather than being tidied into a 200, because a client that retries on 500 and gives
/// up on a 200 tells the two apart.
fn threw() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        "<Error>Internal server error</Error>",
    )
        .into_response()
}

/// Reads a number written either plainly or as `0x...`.
///
/// `Utils.FromString` (`common/Utils.cs`), which the class and skin endpoints put their `classType`
/// and `skinType` through. The client sends object types in hex.
fn from_string(text: &str) -> Option<u32> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(digits) => u32::from_str_radix(digits, 16).ok(),
        None => text.parse::<u32>().ok(),
    }
}

/// Reads the class number a form asked for, exactly as far as the original reads it.
///
/// `(ushort)Utils.FromString(query["classType"])` (`purchaseClassUnlock.cs:15`) narrows in two ways
/// this has to keep. `Int32.Parse` takes only a lowercase `0x` as a hexadecimal marker and reads
/// everything else as decimal, and anything it cannot read at all is swallowed and counts as zero,
/// which is no class; the `ushort` cast then keeps only the low sixteen bits, so `0x1030e` and
/// `-64754` both name the wizard. The reference answers `<Success />` to both.
fn class_type(text: &str) -> u16 {
    let text = text.trim();
    let read = match text.strip_prefix("0x") {
        Some(digits) => u32::from_str_radix(digits, 16).ok().map(|value| value as i32),
        None => text.parse::<i32>().ok(),
    };

    read.unwrap_or(0) as u16
}

/// Whether an address is one `Utils.IsValidEmail` would accept.
///
/// Kept as loose as the original's, which only asks .NET whether it parses as a mailbox. The point
/// is to catch a username typed where an address belongs, not to decide what a valid address is.
fn is_valid_email(address: &str) -> bool {
    let address = address.trim();
    address.contains('@') && address.contains('.') && !address.contains(' ') && address.len() <= 254
}

// -- app -----------------------------------------------------------------------------------------

/// `POST /app/init` -- the settings a client reads before anything else.
///
/// `app/init.cs`. One cached document, served to everyone.
pub async fn app_init(State(app): State<Arc<App>>) -> Blob {
    Blob(app.files.init.clone(), "application/xml")
}

/// `POST /app/getServerXmls` -- the game xmls a client does not already ship.
///
/// `app/getServerXmls.cs`, which writes `GameData.ZippedXmls`: a count and that many length-prefixed
/// documents. The original builds it before it has loaded anything into that list
/// (`common/resources/XmlData.cs:149-160` compresses, then loads), so the reference answers a count
/// of zero. Ours answers the same, and for the same reason it is harmless: the content a client
/// needs is the content it shipped with.
pub async fn app_server_xmls(State(app): State<Arc<App>>) -> Blob {
    Blob(app.files.server_xmls.clone(), "text/plain")
}

/// `POST /app/globalNews` -- the title-screen news.
///
/// `app/globalNews.cs`, which serves `data/news.txt` unchanged.
pub async fn app_global_news(State(app): State<Arc<App>>) -> Blob {
    Blob(app.files.global_news.clone(), "text/plain")
}

/// `POST /app/getTextures` -- every remote texture in one response.
///
/// `app/getTextures.cs`. The artwork for objects the client does not ship art for; ours asks for
/// this on startup (`godot-client/src/Assets/RemoteTextures.cs:43`) and draws a placeholder for
/// each object it cannot get.
pub async fn app_textures(State(app): State<Arc<App>>) -> Blob {
    Blob(app.files.textures.clone(), "text/plain")
}

/// `POST /app/getLanguageStrings` -- the translation table for a language.
///
/// `app/getLanguageStrings.cs`, which indexes `Resources.Languages` by the `languageType` field.
/// The body is a JSON array of `[key, value, language]` triples, which is what both clients parse
/// (`godot-client/src/Text/LineBuilder.cs:36-52`).
///
/// A file shipped in the content directory wins, so a deployment can serve the original's own
/// `data/languages/en.txt` verbatim; otherwise the table is built from the strings the database
/// holds, which is where everything else in this server reads them from.
pub async fn app_language_strings(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Blob {
    let language = match field(&form, "languageType") {
        "" => "en",
        named => named,
    };

    if let Some(shipped) = app.files.languages.get(language) {
        return Blob(shipped.clone().into_bytes(), "text/plain");
    }

    let rows = app.store.strings(language).await.unwrap_or_default();
    let triples: Vec<[&str; 3]> = rows
        .iter()
        .map(|(key, value)| [key.as_str(), value.as_str(), language])
        .collect();

    Blob(
        serde_json::to_vec(&triples).unwrap_or_else(|_| b"[]".to_vec()),
        "text/plain",
    )
}

/// `POST /inGameNews/getNews` -- the news panel inside the game.
///
/// `inGameNews/getNews.cs`, which serves `data/inGameNews.txt`. Empty in the reference, and empty
/// here when nothing is shipped.
pub async fn in_game_news(State(app): State<Arc<App>>) -> Blob {
    Blob(app.files.in_game_news.clone(), "text/plain")
}

/// `POST /dailyLogin/fetchCalendar` -- the login-reward calendar.
///
/// `dailyLogin/fetchCalendar.cs`, which serves `data/loginRewards.xml`. Static there: the file
/// carries a fixed `serverTime` and day count and nothing reads the account.
pub async fn daily_calendar(State(app): State<Arc<App>>) -> Blob {
    Blob(app.files.calendar.clone(), "application/xml")
}

/// `POST /weekQuest/getQuests` -- the week's quests.
///
/// `weekQuest/getQuests.cs`, which serves `data/quests.xml` unchanged.
pub async fn week_quests(State(app): State<Arc<App>>) -> Blob {
    Blob(app.files.quests.clone(), "application/xml")
}

// -- credits and friends -------------------------------------------------------------------------

/// `POST /credits/getoffers` -- what gold is for sale.
///
/// `credits/getoffers.cs`, which answers one hard-coded offer and does not look at anything. Served
/// verbatim: the client parses the shape whether or not the numbers mean anything, and the shape is
/// all this ever carried.
pub async fn credits_offers() -> Xml {
    Xml(OFFERS.to_string())
}

/// The one offer `credits/getoffers.cs` has ever answered with, character for character.
const OFFERS: &str = "<Offers><Tok>WUT</Tok><Exp>STH</Exp><Offer><Id>0</Id><Price>0</Price>\
                      <RealmGold>1000</RealmGold><CheckoutJWT>1000</CheckoutJWT><Data>YO</Data>\
                      <Currency>HKD</Currency></Offer></Offers>";

/// `POST /credits/add` -- buying gold, which nothing here sells.
///
/// `credits/add.cs`, whose whole body is commented out above a refusal. Kept refusing: a server
/// that took payment details would be a server that took payment details.
pub async fn credits_add() -> Xml {
    Xml("<Error>Nope</Error>".to_string())
}

/// `POST /account/sendVerifyEmail` -- which never sends one.
///
/// `account/sendVerifyEmail.cs`, whose entire body is this refusal. It is also the reason nothing
/// in the login path waits on a confirmation: the endpoint that would send the link answers "Nope."
/// in the specification itself.
pub async fn send_verify_email() -> Xml {
    Xml("<Error>Nope.</Error>".to_string())
}

/// `POST /friends/getList` -- who an account knows.
///
/// `friends/getList.cs`, which is a `// TODO` above an empty list. Answered empty here too: the
/// friends an account has are read over the game socket, and inventing contents for an element no
/// client has ever seen filled would be inventing a shape rather than copying one.
pub async fn friends_list() -> Xml {
    Xml("<Friends></Friends>".to_string())
}

/// `POST /friends/getRequests` -- who is waiting for an answer.
///
/// `friends/getRequests.cs`, the same stub as its sibling and answered the same way.
pub async fn friend_requests() -> Xml {
    Xml("<Requests></Requests>".to_string())
}

/// `POST /fame/list` -- the fame leaderboard.
///
/// `fame/list.cs`. `FameList.FromDb` (`server/XmlModels.cs:794-812`) reads no rows: it builds an
/// empty list, caches it under the timespan and returns it, so the document is one self-closing
/// element carrying only the timespan asked for. Kept, because a leaderboard invented here would
/// not be the one the original serves.
///
/// A request with no timespan throws there -- `timeSpan.ToLower()` on nothing -- and is answered
/// with the 500 the router turns that into.
pub async fn fame_list(Form(form): Form<HashMap<String, String>>) -> Response {
    let Some(timespan) = form.get("timespan") else {
        return threw();
    };

    Xml(format!(
        "<FameList timespan=\"{}\" />",
        escape(&timespan.to_lowercase())
    ))
    .into_response()
}

/// `POST /picture/get` -- one texture by name.
///
/// `picture/get.cs`. The id arrives either bare or as `file:name`, in which case the name after the
/// colon is the one meant; anything with more colons than that is refused. A name the server does
/// not hold is a 404 with no body, which is what the reference answers and what the client treats
/// as "use the sprite I shipped with".
pub async fn picture_get(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let Some(id) = form.get("id") else {
        return Xml("<Error>Invalid input</Error>".to_string()).into_response();
    };

    let parts: Vec<&str> = id.split(':').collect();
    let name = match parts.as_slice() {
        [only] => *only,
        [_, second] => *second,
        _ => return Xml("<Error>Invalid input</Error>".to_string()).into_response(),
    };

    match app.files.texture_index.get(name) {
        Some(bytes) => Blob(bytes.clone(), "image/png").into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// -- characters ----------------------------------------------------------------------------------

/// `POST /char/delete` -- removes a character.
///
/// `char/delete.cs`. Answers `<Success />` whether or not there was a character with that id to
/// remove, which is the original's behaviour and not an oversight worth correcting: the caller
/// asked for it to be gone, and it is.
pub async fn char_delete(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    // Credentials before input, in that order: `int.Parse(query["charId"])` sits inside the branch
    // the login check passes into (`char/delete.cs:12-20`), so a wrong password is refused as one
    // rather than throwing on whatever was sent with it.
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal.into_response(),
    };

    let Ok(character_id) = field(&form, "charId").parse::<i64>() else {
        return threw();
    };

    // The original takes the account lock and refuses a character that is being played from another
    // process (`char/delete.cs:14-23`). Ours holds the same lock for the same reason.
    let Ok(Some(lock)) = app.store.acquire_lock(account.id).await else {
        return Xml("<Error>Account in Use</Error>".to_string()).into_response();
    };

    let deleted = app.store.delete_character(account.id, character_id).await;
    let _ = app.store.release_lock(&lock).await;

    if let Err(err) = deleted {
        tracing::warn!(%err, account = account.id, character_id, "could not delete a character");
    }

    Xml("<Success />".to_string()).into_response()
}

/// `POST /char/purchaseClassUnlock` -- pays to skip a class's levelling requirement.
///
/// `char/purchaseClassUnlock.cs`. Three different answers for three different inputs, and the
/// client tells them apart: a number that is no class at all throws there --
/// `Program.Resources.GameData.Classes[cType]` on a key that is not in the dictionary -- while a
/// class the content puts no price on is refused in words, and one the account cannot afford is
/// refused in different words again.
///
/// The refusal is on a price that is absent, not on one that is nothing. The wizard's
/// `<UnlockCost>` reads zero, so it is granted for nothing rather than turned away, and the
/// levelling requirement this pays to skip is the only thing that ever locked it
/// (`purchaseClassUnlock.cs:18-23`).
pub async fn purchase_class_unlock(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal.into_response(),
    };

    let asked = hendra_content::ObjectType(class_type(field(&form, "classType")));

    let class = app
        .catalog
        .classes()
        .iter()
        .find(|class| class.object_type == asked);

    let Some(class) = class else {
        return threw();
    };

    let Some(cost) = class.unlock.cost else {
        return error("Bad input to character unlock").into_response();
    };

    if account.credits < cost as i32 {
        return error("Not enough gold").into_response();
    }

    let Some(identity) = app.catalog.object(class.object_type).map(|desc| desc.uuid) else {
        return threw();
    };

    match app.store.buy_class(account.id, identity, cost as i32).await {
        Ok(()) => Xml("<Success />".to_string()).into_response(),

        // Already unlocked. The original charges again and answers success, because its unlock is
        // an idempotent write it does not look at (`purchaseClassUnlock.cs:31-33`); the answer is
        // kept and the second charge is not, since taking gold for a class the account already has
        // is the part of that nobody would ask for.
        Err(StoreError::Refused("you already have that")) => {
            Xml("<Success />".to_string()).into_response()
        }

        Err(StoreError::Refused(_)) => error("Not enough gold").into_response(),
        Err(err) => {
            tracing::error!(%err, account = account.id, "could not unlock a class");
            error("Bad input to character unlock").into_response()
        }
    }
}

// -- account -------------------------------------------------------------------------------------

/// `POST /account/changePassword` -- replaces a password, given the old one.
///
/// `account/changePassword.cs`.
pub async fn change_password(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal,
    };

    let fresh = field(&form, "newPassword");
    if fresh.is_empty() {
        return error("Invalid password");
    }

    let hash = {
        let _permit = app.hashing.acquire().await;
        match hash_password(fresh) {
            Ok(hash) => hash,
            Err(err) => return error(&err.to_string()),
        }
    };

    match app.store.set_password(account.id, &hash).await {
        Ok(()) => {
            tracing::info!(account = account.id, "password changed");
            Xml("<Success />".to_string())
        }
        Err(err) => {
            tracing::error!(%err, account = account.id, "could not change a password");
            error("try again shortly")
        }
    }
}

/// `POST /account/verifyage` -- records whether somebody said they are old enough.
///
/// `account/verifyage.cs`. Anything other than `1` is taken as "no", and a request that omits the
/// field throws there, so it throws here.
pub async fn verify_age(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal.into_response(),
    };

    // Read after the login check, where the original reads it: the field is dereferenced inside the
    // `status == LoginStatus.OK` branch (`account/verifyage.cs:13-19`).
    let Some(answer) = form.get("isAgeVerified") else {
        return threw();
    };

    if let Err(err) = app.store.set_age_verified(account.id, answer == "1").await {
        tracing::error!(%err, account = account.id, "could not record an age check");
    }

    Xml("<Success />".to_string()).into_response()
}

/// `POST /account/purchaseSkin` -- buys a skin with gold.
///
/// `account/purchaseSkin.cs`. Every reason to refuse -- not levelled far enough, too expensive,
/// restricted, unlockable only by other means -- answers with the same sentence there, which is
/// deliberate: it tells a client nothing about which skins exist.
/// A `skinType` that is not a skin throws there -- `GameData.Skins[skinType]` on a number that is
/// not in the dictionary -- so it is answered with the 500 that becomes, rather than being sold.
pub async fn purchase_skin(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal.into_response(),
    };

    let asked = from_string(field(&form, "skinType"))
        .and_then(|number| u16::try_from(number).ok())
        .map(hendra_content::ObjectType);

    let Some(skin) = asked.and_then(|object_type| app.catalog.skin(object_type)) else {
        return threw();
    };

    // How far the account has got with the class this skin dresses, which is what the content says
    // it takes to wear. `ReadClassStats(acc)[skinDesc.PlayerClassType].BestLevel` there.
    let progress = app.store.class_progress(account.id).await.unwrap_or_default();
    let best_level = app
        .catalog
        .object(skin.class)
        .and_then(|desc| progress.get(&desc.uuid))
        .map(|(level, _)| *level as i32)
        .unwrap_or(0);

    if skin.unlock_level > best_level {
        return error("Failed to purchase skin").into_response();
    }

    // The price comes from the content rather than from the request, or a client would name its
    // own.
    let price = app
        .catalog
        .object(skin.object_type)
        .and_then(|desc| desc.item.as_ref())
        .map(|item| item.fame_bonus.max(0))
        .unwrap_or(0);

    if price > account.credits {
        return error("Failed to purchase skin").into_response();
    }

    let Some(identity) = app.catalog.object(skin.object_type).map(|desc| desc.uuid) else {
        return threw();
    };

    match app.store.buy_skin(account.id, identity, price).await {
        Ok(()) => Xml("<Success />".to_string()).into_response(),
        Err(_) => error("Failed to purchase skin").into_response(),
    }
}

/// `POST /account/purchaseCharSlot` -- buys another character slot.
///
/// `account/purchaseCharSlot.cs`, whose price and currency come from the settings: 550 fame, since
/// `<CharacterSlotCurrency>1</CharacterSlotCurrency>` is fame rather than gold.
///
/// The funds check is the original's. What follows it is not, and deliberately: the slot count here
/// is a fixed number rather than a column, so there is nothing to increment, and the original's own
/// answer for a purchase whose write did not happen is `<Error>Internal Server Error</Error>`
/// (`account/purchaseCharSlot.cs:44-48`). That is what an affordable request gets, with nothing
/// taken from the account -- refusing is a far better wrong answer than charging for a slot that
/// will not appear.
pub async fn purchase_char_slot(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal,
    };

    if account.fame < CHARACTER_SLOT_COST {
        return error("Insufficient funds");
    }

    error("Internal Server Error")
}

/// `POST /account/rank` -- sets the rank a Discord id carries.
///
/// `account/rank.cs`. Only an account trusted to manage ranks may ask, and a role the server does
/// not recognise is not a refusal but a success that says so. Nothing here ships a role table, so
/// every role is unrecognised -- which is exactly what the original answers with an empty
/// `roles.json`, the file it reads that list from.
pub async fn set_rank(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal,
    };

    if account.admin_rank <= 0 {
        return error("Account not allowed to manage ranks.");
    }

    Xml("<Success>Role not found. Default to 0</Success>".to_string())
}

/// `POST /account/registerDiscord` -- links an account to a Discord id.
///
/// `account/registerDiscord.cs`. An account already linked is unlinked first, so one account never
/// answers to two ids.
pub async fn register_discord(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal,
    };

    if account.admin_rank <= 0 {
        return error("No permission");
    }

    let Ok(target) = app.store.account_by_name(field(&form, "ign")).await else {
        return error("Account does not exist");
    };

    let discord_id = field(&form, "dId");
    if discord_id.is_empty() {
        return error("Invalid discord id");
    }

    if let Some(existing) = target.discord_id.as_deref() {
        let _ = app.store.unregister_discord(target.id, existing).await;
    }

    match app.store.register_discord(target.id, discord_id).await {
        Ok(()) => Xml("<Success/>".to_string()),
        Err(err) => {
            tracing::error!(%err, account = target.id, "could not link a discord id");
            error("Invalid discord id")
        }
    }
}

/// `POST /account/unregisterDiscord` -- unlinks one.
///
/// `account/unregisterDiscord.cs`. An id that was not the one linked is refused rather than
/// quietly succeeding, so a caller finds out it unlinked nothing.
pub async fn unregister_discord(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal,
    };

    if account.admin_rank <= 0 {
        return error("No permission");
    }

    let Ok(target) = app.store.account_by_name(field(&form, "ign")).await else {
        return error("Account does not exist");
    };

    let discord_id = field(&form, "dId");
    if discord_id.is_empty() {
        return error("Invalid discord id");
    }

    if target.discord_id.as_deref() != Some(discord_id) {
        return error("Account not linked to discord id");
    }

    match app.store.unregister_discord(target.id, discord_id).await {
        Ok(()) => Xml("<Success/>".to_string()),
        Err(_) => error("Account not linked to discord id"),
    }
}

/// `POST /account/forgotPassword` -- starts a password reset.
///
/// `account/forgotPassword.cs`. An address that is not an address and an address nobody registered
/// get the same answer, which is the point: otherwise this is a way to ask whether somebody has an
/// account here.
///
/// The original mints a token, builds the link and never sends it -- the mail call is gone from the
/// body, leaving `resetLink` unused. Ours mints the same token and hands the link to whatever
/// delivery is configured, which on a laptop is the log.
pub async fn forgot_password(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let address = field(&form, "guid");

    if !is_valid_email(address) {
        return error("Email not recognized");
    }

    let Ok(account_id) = app.store.account_by_email(address).await else {
        return error("Email not recognized");
    };

    match app
        .store
        .issue_email_token(account_id, hendra_store::email::Purpose::Reset)
        .await
    {
        Ok(token) => {
            tracing::info!(account = account_id, "password reset requested");
            let link = format!("/account/rp?b={}&a={account_id}", token.0);
            let _ = app.mail.send(address, "Reset your password", &link);
        }
        Err(err) => tracing::error!(%err, account = account_id, "could not issue a reset token"),
    }

    Xml("<Success />".to_string())
}

/// The page `account/rp` answers with when the token is not the one that was issued.
///
/// `data/changePassword/resetError.html`, served byte for byte.
const RESET_ERROR_PAGE: &str = r#"<html lang='en'>
    <head>
        <meta http-equiv='Content-Type' contet='text/html; charset=utf-8'>
        <title>Error Verifying Email Address</title>
        <style>
            body { margin: 0; padding: 0; overflow: hidden;
            background-color: #000; color: #fff; font-family: verdana }
            :link { color: #ccf; }
            :visited { color: #fcf; }
        </style>
    </head>
    <body>
    <center>
        <br/>
        <b><font color='red'>Unable to Reset Password</font></b>
    </center>
    </body>
</html>"#;

/// The page it answers with when the reset went through.
///
/// `data/changePassword/reset.html`, with `{PASSWORD}` standing where the new one goes.
const RESET_PAGE: &str = r#"<html lang='en'>
    <head>
        <meta http-equiv='Content-Type' content='text/html; charset=utf-8'>
        <title>Email Address Verified</title>
        <style>
            body { margin: 0; padding: 0; overflow: hidden;
            background-color: #000; color: #fff; font-family: verdana }
            :link { color: #ccf; }
            :visited { color: #fcf; }
        </style>
    </head>
    <body>
    <center>
        <br/>
        <p>Your new password is: {PASSWORD}</p>
        <p>It has also been emailed to you.</p>
        <p>Passwords are CaSe-SeNsItIvE!</p>
        <p>You can change your password using the <b>'account'</b> link on the title screen.</p>
    </center>
    </body>
</html>"#;

/// The alphabet a generated password is drawn from.
///
/// `resetPassword.CreatePassword` (`account/resetPassword.cs:14-23`), letters and digits with no
/// punctuation, because the page tells the player to type it back.
const PASSWORD_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";

/// `GET`/`POST /account/rp` -- finishes a password reset, in a browser.
///
/// `account/resetPassword.cs`, the one endpoint here that answers HTML: it is reached from the link
/// in a reset mail rather than from the game. The server picks the new password and shows it, which
/// is why the page says to change it afterwards.
///
/// Registered on both verbs, as the original registers it: `GET` for the link and `POST` for the
/// form (`RequestHandler.cs:96-108`).
pub async fn reset_password_link(
    State(app): State<Arc<App>>,
    axum::extract::Query(fields): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    reset_password(&app, fields).await
}

/// The same, reached by posting the token rather than following a link.
pub async fn reset_password_form(
    State(app): State<Arc<App>>,
    Form(fields): Form<HashMap<String, String>>,
) -> Response {
    reset_password(&app, fields).await
}

async fn reset_password(app: &App, fields: HashMap<String, String>) -> Response {
    let page = |body: String| {
        (
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            )],
            body,
        )
            .into_response()
    };

    let token = field(&fields, "b");
    if token.is_empty() {
        return page(RESET_ERROR_PAGE.to_string());
    }

    let Ok(account_id) = app
        .store
        .spend_email_token(token, hendra_store::email::Purpose::Reset)
        .await
    else {
        return page(RESET_ERROR_PAGE.to_string());
    };

    let password = new_password();
    let hash = {
        let _permit = app.hashing.acquire().await;
        match hash_password(&password) {
            Ok(hash) => hash,
            Err(_) => return page(RESET_ERROR_PAGE.to_string()),
        }
    };

    if app.store.set_password(account_id, &hash).await.is_err() {
        return page(RESET_ERROR_PAGE.to_string());
    }

    tracing::info!(account = account_id, "password reset");
    page(RESET_PAGE.replace("{PASSWORD}", &password))
}

/// Picks a password of eight to eleven characters, the length the original picks from.
///
/// Drawn from the operating system's randomness rather than a seeded generator: the original uses
/// `new Random()`, which is seeded from the clock and gives two resets a second apart the same
/// password. That is a bug worth not copying, since the password it produces is the account's.
fn new_password() -> String {
    let mut bytes = uuid::Uuid::new_v4().as_bytes().to_vec();
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());

    let length = 8 + (bytes[0] as usize % 4);
    bytes[1..=length]
        .iter()
        .map(|byte| PASSWORD_ALPHABET[*byte as usize % PASSWORD_ALPHABET.len()] as char)
        .collect()
}

// -- guild ---------------------------------------------------------------------------------------

/// `POST /guild/getBoard` -- the asking account's guild noticeboard.
///
/// `guild/getBoard.cs`. The board is written out as the body with nothing around it, so a guild
/// whose board is empty answers with an empty body.
pub async fn guild_board(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal,
    };

    let Ok(Some((id, _))) = app.store.guild_of(account.id).await else {
        return error("Not in guild");
    };

    match app.store.guild(id).await {
        Ok(guild) => Xml(guild.board),
        Err(_) => error("Not in guild"),
    }
}

/// `POST /guild/setBoard` -- changes it.
///
/// `guild/setBoard.cs`. Officers and above only, and the answer on success is the text that was
/// set rather than an acknowledgement, which is how the client knows what it ended up as.
pub async fn set_guild_board(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal,
    };

    let Ok(Some((_, rank))) = app.store.guild_of(account.id).await else {
        return error("No permission");
    };

    if !rank.may_set_board() {
        return error("No permission");
    }

    // The original url-decodes the field itself because its parser does not
    // (`guild/setBoard.cs:19`); ours arrives decoded, form decoding being the extractor's job.
    let board = field(&form, "board").to_string();

    match app.store.set_guild_board(account.id, &board).await {
        Ok(()) => Xml(board),
        Err(_) => error("Failed to set board"),
    }
}

// -- private messages ----------------------------------------------------------------------------

/// `POST /privateMessage/list` -- an account's mail.
///
/// `privateMessage/list.cs`. JSON rather than XML, and answered with an empty list rather than a
/// refusal when the credentials are wrong: the original catches everything and writes
/// `{"messages":[]}`, so a client never sees this fail.
///
/// The fields are `PrivateMessages`' (`common/resources/PrivateMessages.cs:16-83`). Two of them
/// carry nothing here: the store keeps a body and a sender, not a subject, and inventing a column
/// to fill `subject` with would be inventing a mechanic rather than copying one.
pub async fn message_list(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Blob {
    let empty = || Blob(b"{\"messages\":[]}".to_vec(), "text/plain");

    let Ok(account) = verified(&app, &form).await else {
        return empty();
    };

    let Ok(messages) = app.store.messages(account.id, 100).await else {
        return empty();
    };

    // An account that has never been written to has no `privateMessages` field at all there, and
    // the handler writes its `NoMessages` literal rather than serialising an empty object -- so the
    // owner id only ever appears alongside messages.
    if messages.is_empty() {
        return empty();
    }

    let body = serde_json::json!({
        "ownerAccountId": account.id,
        "messages": messages
            .iter()
            .map(|message| serde_json::json!({
                "senderId": 0,
                "recipientId": account.id,
                "subject": "",
                "message": message.body,
                "receiveTime": message.id,
                "senderName": message.from,
                "recipientName": account.name,
            }))
            .collect::<Vec<_>>(),
    });

    match serde_json::to_vec(&body) {
        Ok(bytes) => Blob(bytes, "text/plain"),
        Err(_) => empty(),
    }
}

/// `POST /privateMessage/send` -- writes to somebody.
///
/// `privateMessage/send.cs`. Success is a sentence rather than an element, which is the original's
/// and is what the client shows.
pub async fn message_send(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Xml {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal,
    };

    let recipient = field(&form, "recipient").to_string();
    if account.name.eq_ignore_ascii_case(&recipient) {
        return error("Stop sending yourself messages.");
    }

    let Ok(target) = app.store.account_by_name(&recipient).await else {
        return error(
            "Recipient not found. This account does not exist or its an unnamed account, make \
             sure you wrote the name correctly.",
        );
    };

    match app
        .store
        .send_message(account.id, target.id, field(&form, "message"))
        .await
    {
        Ok(_) => Xml("Your message has been sent successfully.".to_string()),
        Err(err) => {
            tracing::warn!(%err, account = account.id, "could not send a message");
            error("Internal server error")
        }
    }
}

/// `POST /privateMessage/delete` -- removes one.
///
/// `privateMessage/delete.cs`, which identifies the message by the time it arrived. Ours identifies
/// it by row, which is what `time` carries in the list above.
pub async fn message_delete(
    State(app): State<Arc<App>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let account = match verified(&app, &form).await {
        Ok(account) => account,
        Err(refusal) => return refusal.into_response(),
    };

    // As in `privateMessage/delete.cs:14`, where the parse is inside the branch the login check
    // passes into.
    let Ok(message_id) = field(&form, "time").parse::<i64>() else {
        return threw();
    };

    let _ = app.store.delete_message(account.id, message_id).await;
    Xml("<Success />".to_string()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_may_be_written_in_hexadecimal() {
        assert_eq!(from_string("0x0300"), Some(0x300));
        assert_eq!(from_string("768"), Some(768));
        assert_eq!(from_string(""), None);
        assert_eq!(from_string("knight"), None);
    }

    /// The refusals the clients branch on, spelled as the original spells them. A capital letter
    /// out of place here is a client that does not recognise its own error.
    #[test]
    fn the_refusals_are_the_specifications_wording() {
        assert_eq!(error("Not in guild").0, "<Error>Not in guild</Error>");
        assert_eq!(
            error("Insufficient funds").0,
            "<Error>Insufficient funds</Error>"
        );
        assert_eq!(error("Nope.").0, "<Error>Nope.</Error>");
    }

    #[test]
    fn a_generated_password_is_letters_and_digits_only() {
        for _ in 0..32 {
            let password = new_password();
            assert!((8..12).contains(&password.len()));
            assert!(password.chars().all(|c| c.is_ascii_alphanumeric()));
        }
    }

    #[test]
    fn a_reserved_name_is_not_a_chosen_one() {
        assert!(is_guest_name("Eango"));
        assert!(is_guest_name("eango"));
        assert!(
            is_guest_name("Eango2"),
            "a reserved name handed out twice is still reserved"
        );

        assert!(!is_guest_name("Fesal"));
        assert!(!is_guest_name("Eangoth"), "only the whole name is reserved");
    }

    #[test]
    fn text_that_would_break_the_document_is_escaped() {
        assert_eq!(escape("Rock & <Roll>"), "Rock &amp; &lt;Roll&gt;");
    }

    /// Every reserved name is refused by the naming rules, which is what makes a reserved name
    /// readable as "this account has not chosen one".
    #[test]
    fn no_reserved_name_could_ever_be_chosen() {
        for reserved in GUEST_NAMES {
            assert!(
                is_guest_name(reserved),
                "{reserved} would be taken for a chosen name"
            );
        }
    }
}

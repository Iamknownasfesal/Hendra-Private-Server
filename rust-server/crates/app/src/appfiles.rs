//! The files the app-engine endpoints hand back unchanged.
//!
//! Six of the original's endpoints do nothing but return a blob it read at boot: `app/init.cs`,
//! `app/globalNews.cs`, `app/getTextures.cs`, `app/getServerXmls.cs`, `inGameNews/getNews.cs`,
//! `dailyLogin/fetchCalendar.cs` and `weekQuest/getQuests.cs` each keep a `static byte[] _data`
//! filled in by `InitHandler` and write it out on every request. This is that cache.
//!
//! Read once at startup for the same reason the original does: these are files on disk that never
//! change while the process runs, and a request is not the moment to discover one is missing.
//!
//! # Where the files live
//!
//! In a resource folder laid out the way the original's is -- `data/` for the documents,
//! `data/languages/` for the translation tables and `textures/` for the remote artwork -- which
//! means the original's own `XmlDatas` directory can be pointed at unchanged. `HENDRA_APP_DATA`
//! names it; without it the content directory is used, since a deployment that ships one directory
//! ships everything in it.
//!
//! A file that is not there falls back to a built-in default rather than refusing to start, because
//! none of these is worth failing a boot over: the original crashes on a missing `data/init.xml`,
//! which is a worse answer than serving the settings every deployment shares anyway.

use std::collections::BTreeMap;
use std::path::Path;

/// What `data/init.xml` says when the deployment has not supplied one.
///
/// `Server-Side/XmlDatas/data/init.xml`, with `SkinsList` and `FilterList` emptied. The original
/// treats the text of those two tags as a *path* and replaces it with that file's contents, or with
/// nothing when the path does not resolve (`app/init.cs:20-30`); the reference answers with both
/// empty, so this is what it serves.
const DEFAULT_INIT: &str = r#"<AppSettings>
  <MenuMusic></MenuMusic>
  <DeadMusic></DeadMusic>
  <EditorMinRank>0</EditorMinRank>
  <CharacterSlotCost>550</CharacterSlotCost>
  <CharacterSlotCurrency>1</CharacterSlotCurrency>
  <VaultChestCost>400</VaultChestCost>
  <InventorySize>24</InventorySize>
  <UseExternalPayments>0</UseExternalPayments>
  <MaxStackablePotions>6</MaxStackablePotions>
  <PotionPurchaseCooldown>400</PotionPurchaseCooldown>
  <PotionPurchaseCostCooldown>8000</PotionPurchaseCostCooldown>
  <PotionPurchaseCosts>
    <cost>5</cost>
    <cost>10</cost>
    <cost>20</cost>
    <cost>40</cost>
    <cost>80</cost>
    <cost>120</cost>
    <cost>200</cost>
    <cost>300</cost>
    <cost>450</cost>
    <cost>600</cost>
  </PotionPurchaseCosts>
  <SkinsList></SkinsList>
  <FilterList></FilterList>
  <DisableRegist>0</DisableRegist>
  <MysteryBoxRefresh>600</MysteryBoxRefresh>
  <MaxPetCount>50</MaxPetCount>
  <SalesforceMobile>0</SalesforceMobile>
  <NewAccounts>
    <Gold>0</Gold>
    <Fame>0</Fame>
    <ClassesUnlocked>1</ClassesUnlocked>
    <SkinsUnlocked>1</SkinsUnlocked>
    <PetYardType>1</PetYardType>
    <VaultCount>1</VaultCount>
    <MaxCharSlot>2</MaxCharSlot>
  </NewAccounts>
  <NewCharacters>
    <Maxed>0</Maxed>
    <Level>1</Level>
  </NewCharacters>
</AppSettings>"#;

/// What a character slot costs, and in which currency, read out of the settings above.
///
/// `<CharacterSlotCurrency>1</CharacterSlotCurrency>` is `CurrencyType.Fame`, which is what
/// `account/purchaseCharSlot.cs:27-34` compares against `acc.Fame`.
pub const CHARACTER_SLOT_COST: i32 = 550;

/// Which coin a character slot is bought with, as the clients number them.
///
/// `CurrencyType.Fame` is 1, and the AS3 client swaps the icon beside the price on it
/// (`BuyCharacterRect.as:35`), so quoting the wrong number offers gold for a fame charge.
pub const CHARACTER_SLOT_CURRENCY: i32 = 1;

/// Everything the static endpoints answer with, read once.
pub struct AppFiles {
    /// `app/init` -- the `<AppSettings>` document.
    pub init: Vec<u8>,

    /// `app/getServerXmls` -- the extra game xmls, as a count followed by that many length-prefixed
    /// documents.
    pub server_xmls: Vec<u8>,

    /// `app/globalNews` -- the title-screen news, as the client's JSON.
    pub global_news: Vec<u8>,

    /// `inGameNews/getNews` -- the same for the in-game panel.
    pub in_game_news: Vec<u8>,

    /// `dailyLogin/fetchCalendar` -- the login-reward calendar.
    pub calendar: Vec<u8>,

    /// `weekQuest/getQuests` -- the week's quests.
    pub quests: Vec<u8>,

    /// `app/getTextures` -- every remote texture in one blob.
    pub textures: Vec<u8>,

    /// The same textures by name, which is what `picture/get` answers from.
    pub texture_index: BTreeMap<String, Vec<u8>>,

    /// `app/getLanguageStrings` -- the translation table per language, when one ships as a file.
    pub languages: BTreeMap<String, String>,

    /// Where `music/` and `sfx/` live, which the client fetches from by name.
    ///
    /// Read from disk per request rather than held, unlike everything else here: the corpus is
    /// sixty megabytes of audio and a client asks for a handful of it. The original serves the same
    /// folder as static files.
    pub audio: std::path::PathBuf,
}

impl Default for AppFiles {
    fn default() -> AppFiles {
        AppFiles {
            init: DEFAULT_INIT.as_bytes().to_vec(),
            server_xmls: 0i32.to_be_bytes().to_vec(),
            global_news: b"[]".to_vec(),
            in_game_news: Vec::new(),
            calendar: b"<LoginRewards />".to_vec(),
            quests: b"<QuestsResponse />".to_vec(),
            textures: 0i32.to_be_bytes().to_vec(),
            texture_index: BTreeMap::new(),
            languages: BTreeMap::new(),
            audio: std::path::PathBuf::new(),
        }
    }
}

impl AppFiles {
    /// Where the resource folder is, from the environment, or the content directory.
    pub fn resource_folder(content: &Path) -> std::path::PathBuf {
        match std::env::var("HENDRA_APP_DATA") {
            Ok(named) if !named.trim().is_empty() => std::path::PathBuf::from(named.trim()),
            _ => content.to_path_buf(),
        }
    }

    /// Reads what a resource folder has, keeping the defaults for what it does not.
    pub fn load(content: &Path) -> AppFiles {
        let mut files = AppFiles::default();
        let content = AppFiles::resource_folder(content);
        let data = content.join("data");

        if let Some(text) = read_text(&data.join("init.xml")) {
            files.init = inline_listed_files(&text, &content).into_bytes();
        }
        if let Some(text) = read_text(&data.join("news.txt")) {
            files.global_news = text.into_bytes();
        }
        if let Ok(bytes) = std::fs::read(data.join("inGameNews.txt")) {
            files.in_game_news = bytes;
        }
        if let Some(text) = read_text(&data.join("loginRewards.xml")) {
            files.calendar = text.into_bytes();
        }
        if let Some(text) = read_text(&data.join("quests.xml")) {
            files.quests = text.into_bytes();
        }

        files.languages = languages(&data.join("languages"));
        files.texture_index = textures(&content.join("textures"));
        files.textures = pack(&files.texture_index);
        files.audio = content.join("web");

        files
    }

    /// Reads one audio file out of the resource folder, or `None` when it is not there.
    ///
    /// The original serves `web/music` and `web/sfx` as ordinary static files, which is what the
    /// client fetches from: it builds `<app server>/<folder>/<name>.mp3` and hands the bytes to the
    /// player. The name arrives with its extension, and is a single path segment with no separators,
    /// so it cannot walk out of the folder.
    pub fn audio(&self, folder: &str, name: &str) -> Option<Vec<u8>> {
        if !plain_name(folder) || !plain_name(name) {
            return None;
        }

        std::fs::read(self.audio.join(folder).join(name)).ok()
    }
}

/// Whether a path segment is a bare name, with nothing in it that could leave the folder.
fn plain_name(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 128
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !segment.contains("..")
}

/// The two elements of `init.xml` that name a file instead of carrying a value.
///
/// `app/init.cs:22`. Everything else in the document is read as written.
const INLINED_ELEMENTS: [&str; 2] = ["SkinsList", "FilterList"];

/// Replaces the two elements that name a file with what that file holds.
///
/// `init.cs:20-30` treats the text of `<SkinsList>` and `<FilterList>` as a path, reads it if it
/// resolves and puts *nothing* there if it does not -- which is why the reference answers with both
/// empty: the paths in the shipped file are relative to a working directory the server does not run
/// from. Ours resolves them against the resource folder first, so a deployment that ships the files
/// gets them, and empties the element otherwise, exactly as the original does.
fn inline_listed_files(document: &str, resources: &Path) -> String {
    let mut inlined = document.to_string();

    for element in INLINED_ELEMENTS {
        let open = format!("<{element}>");
        let close = format!("</{element}>");

        let Some(start) = inlined.find(&open) else {
            continue;
        };
        let body = start + open.len();
        let Some(end) = inlined[body..].find(&close).map(|at| body + at) else {
            continue;
        };

        let named = inlined[body..end].trim().to_string();
        let value = if named.is_empty() {
            String::new()
        } else {
            std::fs::read_to_string(resources.join(&named))
                .or_else(|_| std::fs::read_to_string(&named))
                .unwrap_or_default()
        };

        inlined.replace_range(body..end, &value);
    }

    inlined
}

/// Reads a text file with any byte-order mark taken off the front.
///
/// The original ships `data/languages/en.txt` with one, and serves the bytes as they are; a mark
/// left in the middle of a response is a parse error for every client that reads the body as JSON
/// rather than sniffing it first.
fn read_text(path: &Path) -> Option<String> {
    Some(without_mark(&std::fs::read_to_string(path).ok()?).to_string())
}

/// The text without a leading byte-order mark.
fn without_mark(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Every `<language>.txt` in a directory, keyed by the name the client asks for.
fn languages(directory: &Path) -> BTreeMap<String, String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return BTreeMap::new();
    };

    let mut found = BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("txt") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if let Some(text) = read_text(&path) {
            found.insert(name.to_string(), text);
        }
    }
    found
}

/// Every remote texture in a directory, keyed by the name a client asks for it under.
///
/// The original names these files with a leading underscore and strips it on the way out:
/// `FetchTexture` reads `<dir>/_<id>.png` and `FetchMask` reads `<dir>/_<id>_mask.png`
/// (`common/resources/Resources.cs:158-190`), and the id is what goes into the blob and what
/// `picture/get` is asked for. The prefix is what separates the seven textures this server serves
/// from the several hundred other images that share the folder, so it is the whole selection rule.
fn textures(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return BTreeMap::new();
    };

    let mut found = BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("png") {
            continue;
        }
        let Some(name) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.strip_prefix('_'))
        else {
            continue;
        };
        if let Ok(bytes) = std::fs::read(&path) {
            found.insert(name.to_string(), bytes);
        }
    }
    found
}

/// Packs named images into the blob `app/getTextures` answers with.
///
/// The shape is `Resources.textures`' (`common/resources/Resources.cs:142-155`): a count, then per
/// image a name written as a two-byte length and its UTF-8, a four-byte length and that many bytes.
/// Every number is big-endian, which is what `NWriter` writes and what the client's reader expects.
pub fn pack(images: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut packed = Vec::new();
    packed.extend_from_slice(&(images.len() as i32).to_be_bytes());

    for (name, bytes) in images {
        packed.extend_from_slice(&(name.len() as u16).to_be_bytes());
        packed.extend_from_slice(name.as_bytes());
        packed.extend_from_slice(&(bytes.len() as i32).to_be_bytes());
        packed.extend_from_slice(bytes);
    }

    packed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_pack_is_a_count_of_zero() {
        assert_eq!(pack(&BTreeMap::new()), vec![0, 0, 0, 0]);
    }

    /// The layout the client reads back: count, then name length, name, byte length, bytes.
    #[test]
    fn a_packed_image_carries_its_name_and_length_first() {
        let mut images = BTreeMap::new();
        images.insert("ab".to_string(), vec![9u8, 8, 7]);

        assert_eq!(
            pack(&images),
            vec![0, 0, 0, 1, 0, 2, b'a', b'b', 0, 0, 0, 3, 9, 8, 7]
        );
    }

    /// A deployment with no files still answers every static endpoint with something a client can
    /// parse, which is the difference between a fresh checkout that boots and one that does not.
    #[test]
    fn the_defaults_are_documents_rather_than_nothing() {
        let files = AppFiles::default();
        assert!(String::from_utf8_lossy(&files.init).contains("<CharacterSlotCost>550"));
        assert_eq!(files.server_xmls, vec![0, 0, 0, 0]);
        assert_eq!(files.global_news, b"[]");
    }

    /// A path that resolves to nothing leaves the element empty rather than leaving the path in it,
    /// which is what the reference answers with the files it ships.
    #[test]
    fn an_unresolvable_listed_file_empties_its_element() {
        let document = "<AppSettings><SkinsList>nowhere/skins.xml</SkinsList>\
                        <FilterList></FilterList></AppSettings>";

        assert_eq!(
            inline_listed_files(document, Path::new("/nonexistent")),
            "<AppSettings><SkinsList></SkinsList><FilterList></FilterList></AppSettings>"
        );
    }

    /// One that resolves is read in, which is the whole point of the indirection.
    #[test]
    fn a_listed_file_is_read_into_the_element() {
        // A directory of this process's own, because several test binaries run at once and a fixed
        // name in the shared temp directory is one of them deleting another's fixture.
        let folder = std::env::temp_dir().join(format!("hendra-appfiles-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("skins.xml"), "<Skins/>").unwrap();

        assert_eq!(
            inline_listed_files("<SkinsList>skins.xml</SkinsList>", &folder),
            "<SkinsList><Skins/></SkinsList>"
        );

        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A byte-order mark in a shipped file is the file's business, not the client's.
    #[test]
    fn a_byte_order_mark_does_not_reach_the_client() {
        assert_eq!(without_mark("\u{feff}[[\"a\",\"b\",\"en\"]]"), "[[\"a\",\"b\",\"en\"]]");
        assert_eq!(without_mark("already clean"), "already clean");
        assert_eq!(
            without_mark("in\u{feff}side"),
            "in\u{feff}side",
            "only a leading one is a mark"
        );
    }
}

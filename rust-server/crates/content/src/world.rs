//! World definitions: what a place is, and which maps make it.
//!
//! These come from `.jw` files, which are JSON in spirit and not quite JSON in fact — portal lists
//! are written with hex literals like `0xff00`, which the specification does not allow and the
//! game's C# parser accepted anyway. Rather than adopt a lenient JSON parser for one field, the
//! literals are rewritten to decimal before parsing. That keeps the tolerance in one visible place
//! instead of spreading it across everything that reads JSON.

use serde::{Deserialize, Deserializer};

/// Everything a world needs before its maps are loaded.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct WorldDef {
    pub id: i32,
    pub name: String,

    /// The name shown on the scoreboard, which is sometimes shorter than `name`.
    #[serde(rename = "sbName")]
    pub sb_name: String,

    pub difficulty: i32,
    pub background: i32,

    /// A staging area rather than a place to play — the character-select world, mainly.
    #[serde(rename = "isLimbo", deserialize_with = "lenient_bool")]
    pub is_limbo: bool,

    /// Whether teleporting between players is refused here.
    #[serde(rename = "restrictTp", deserialize_with = "lenient_bool")]
    pub restrict_tp: bool,

    #[serde(rename = "showDisplays", deserialize_with = "lenient_bool")]
    pub show_displays: bool,

    /// Whether the world outlives the last player in it.
    #[serde(deserialize_with = "lenient_bool")]
    pub persist: bool,

    pub blocking: i32,

    #[serde(deserialize_with = "lenient_bool")]
    pub setpiece: bool,

    /// Object types of the portals that lead here.
    pub portals: Vec<i32>,

    /// Map files, in the order they are laid down.
    pub maps: Vec<String>,

    pub music: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    #[error("parsing world definition: {0}")]
    Json(String),

    #[error("world definition has no name")]
    Unnamed,
}

impl WorldDef {
    /// Parses a `.jw` file.
    pub fn parse(text: &str) -> Result<WorldDef, WorldError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let normalised = rewrite_hex_literals(text);
        let world: WorldDef = serde_json::from_str(&normalised)
            .map_err(|source| WorldError::Json(source.to_string()))?;

        if world.name.is_empty() {
            return Err(WorldError::Unnamed);
        }
        Ok(world)
    }

    /// The map this world starts from, if it names one.
    pub fn first_map(&self) -> Option<&str> {
        self.maps.first().map(String::as_str)
    }
}

/// Accepts a boolean written either as one or as the string `"true"` / `"false"`.
///
/// Eight setpiece definitions in the corpus quote their booleans. The C# parser took both
/// spellings, so the files were never corrected, and refusing them now would drop eight worlds over
/// a pair of quotation marks.
fn lenient_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Written {
        Bool(bool),
        Text(String),
    }

    Ok(match Written::deserialize(deserializer)? {
        Written::Bool(value) => value,
        Written::Text(text) => matches!(text.trim().to_ascii_lowercase().as_str(), "true" | "1"),
    })
}

/// Rewrites `0x`-prefixed numbers as decimal, leaving anything inside a string alone.
///
/// Tracking string state matters: an item description containing "0xdeadbeef" is legitimate JSON
/// and rewriting it would corrupt the text. Escapes are honoured so a `\"` inside a string does not
/// look like the end of it.
fn rewrite_hex_literals(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    while at < bytes.len() {
        let byte = bytes[at];

        if in_string {
            out.push(byte as char);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            at += 1;
            continue;
        }

        if byte == b'"' {
            in_string = true;
            out.push('"');
            at += 1;
            continue;
        }

        // A hex literal starts with `0x` and is preceded by something that is not part of a name,
        // so `{"a0x1": …}` outside a string cannot be mistaken for one.
        if byte == b'0'
            && at + 1 < bytes.len()
            && (bytes[at + 1] == b'x' || bytes[at + 1] == b'X')
            && at
                .checked_sub(1)
                .is_none_or(|before| !bytes[before].is_ascii_alphanumeric())
        {
            let start = at + 2;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                end += 1;
            }

            if end > start
                && let Ok(value) = i64::from_str_radix(&text[start..end], 16)
            {
                out.push_str(&value.to_string());
                at = end;
                continue;
            }
        }

        out.push(byte as char);
        at += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_world_definition_parses() {
        // Taken verbatim from DonorShop.jw, hex portal list and all.
        let world = WorldDef::parse(
            r#"{"id":-16,
"isLimbo":true,
"name":"Donor Shop",
"sbName":"Donor Shop",
"difficulty":-1,
"background":0,
"restrictTp":false,
"showDisplays":false,
"portals":[0xff00],
"maps":["DonorShop.jm"],
"music":["Vault1"]}"#,
        )
        .unwrap();

        assert_eq!(world.id, -16);
        assert_eq!(world.name, "Donor Shop");
        assert!(world.is_limbo);
        assert_eq!(world.difficulty, -1);
        assert_eq!(world.portals, vec![0xff00]);
        assert_eq!(world.first_map(), Some("DonorShop.jm"));
        assert_eq!(world.music, vec!["Vault1"]);
    }

    #[test]
    fn hex_literals_become_decimal_outside_strings() {
        assert_eq!(rewrite_hex_literals("[0xff00]"), "[65280]");
        assert_eq!(rewrite_hex_literals("[0x1,0X2,3]"), "[1,2,3]");
        assert_eq!(rewrite_hex_literals("{\"a\":0xA}"), "{\"a\":10}");
    }

    #[test]
    fn hex_inside_a_string_is_left_alone() {
        // A description mentioning a hex value must survive untouched.
        let text = r#"{"name":"Rune 0xdeadbeef","portals":[0x10]}"#;
        assert_eq!(
            rewrite_hex_literals(text),
            r#"{"name":"Rune 0xdeadbeef","portals":[16]}"#
        );
    }

    #[test]
    fn an_escaped_quote_does_not_end_a_string_early() {
        let text = r#"{"name":"say \"0xff\" aloud","portals":[0xff]}"#;
        let rewritten = rewrite_hex_literals(text);
        assert!(
            rewritten.contains(r#"say \"0xff\" aloud"#),
            "the string body should be untouched: {rewritten}"
        );
        assert!(rewritten.ends_with("[255]}"));
    }

    #[test]
    fn a_definition_without_a_name_is_refused() {
        assert!(matches!(
            WorldDef::parse(r#"{"id":1}"#),
            Err(WorldError::Unnamed)
        ));
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let world = WorldDef::parse(r#"{"name":"Nexus"}"#).unwrap();
        assert_eq!(world.name, "Nexus");
        assert!(!world.is_limbo);
        assert!(world.maps.is_empty());
        assert_eq!(world.first_map(), None);
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(WorldDef::parse("not json at all").is_err());
        assert!(WorldDef::parse("").is_err());
        assert!(WorldDef::parse("{").is_err());
    }
}

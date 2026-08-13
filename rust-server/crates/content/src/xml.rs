//! A small DOM over the game's XML, and the number parsing its authors assumed.
//!
//! The content files are hand-maintained and irregular in ways a derive-based deserialiser fights
//! rather than absorbs: presence-only elements (`<Item/>`, `<NoWalk/>`), the same field spelled as
//! an element in one file and an attribute in another, elements repeated where one was expected,
//! and numbers written in hex, decimal or float interchangeably. Parsing into a generic tree once
//! and reading typed fields off it keeps all of that irregularity in one place, and means a new or
//! misspelled element is a `None` rather than a failed load of the whole file.

use std::path::Path;

use quick_xml::events::Event;

/// One XML element, with its attributes, direct text and children.
///
/// Text from mixed content is concatenated: the content files never rely on the interleaving, and
/// flattening it makes `text` usable directly.
#[derive(Debug, Clone, Default)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Node>,
}

#[derive(Debug, thiserror::Error)]
pub enum XmlError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("parsing {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: quick_xml::Error,
    },

    #[error("{path} has no root element")]
    Empty { path: String },
}

impl Node {
    /// Parses a document and returns its root element.
    pub fn parse(text: &str) -> Result<Node, quick_xml::Error> {
        let mut reader = quick_xml::Reader::from_str(text);
        let config = reader.config_mut();
        config.trim_text(true);
        // The content files contain unmatched markup in a few places (an author's stray `<br>` in a
        // description, for one). Checking end names would reject the whole file over a cosmetic
        // slip in one description string, so we accept the mismatch and keep the data.
        config.check_end_names = false;

        let mut stack: Vec<Node> = vec![Node::default()];
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Start(e) => stack.push(Node::open(&e)?),

                Event::Empty(e) => {
                    let node = Node::open(&e)?;
                    stack
                        .last_mut()
                        .expect("root sentinel is never popped")
                        .children
                        .push(node);
                }

                Event::End(_) => {
                    // The sentinel root must survive an unbalanced closing tag, hence `len() > 1`.
                    if stack.len() > 1 {
                        let done = stack.pop().expect("checked len");
                        stack.last_mut().expect("checked len").children.push(done);
                    }
                }

                Event::Text(e) => {
                    let piece = e.unescape().unwrap_or_else(|_| {
                        String::from_utf8_lossy(e.as_ref()).into_owned().into()
                    });
                    let into = stack.last_mut().expect("root sentinel is never popped");
                    if !into.text.is_empty() && !piece.is_empty() {
                        into.text.push(' ');
                    }
                    into.text.push_str(piece.trim());
                }

                Event::CData(e) => {
                    let raw = String::from_utf8_lossy(e.as_ref()).into_owned();
                    stack
                        .last_mut()
                        .expect("root sentinel is never popped")
                        .text
                        .push_str(raw.trim());
                }

                Event::Eof => break,

                // Declarations, comments, processing instructions and doctypes carry nothing the
                // game reads.
                _ => {}
            }
            buf.clear();
        }

        // Anything left open at EOF still belongs to the tree; unwind it rather than drop it.
        while stack.len() > 1 {
            let done = stack.pop().expect("checked len");
            stack.last_mut().expect("checked len").children.push(done);
        }

        let mut sentinel = stack.pop().expect("root sentinel is never popped");
        Ok(if sentinel.children.len() == 1 {
            sentinel.children.pop().expect("checked len")
        } else {
            // No single root, or none at all: hand back the sentinel so callers still see whatever
            // top-level elements were found.
            sentinel
        })
    }

    /// Reads and parses a file.
    ///
    /// The declared encoding is ISO-8859-1 and the bytes really are Latin-1. A handful of item
    /// descriptions carry accented characters. Latin-1 maps one byte to one codepoint, so the
    /// conversion is exact and needs no encoding tables.
    pub fn parse_file(path: &Path) -> Result<Node, XmlError> {
        let bytes = std::fs::read(path).map_err(|source| XmlError::Io {
            path: path.display().to_string(),
            source,
        })?;

        let text: String = if let Ok(utf8) = std::str::from_utf8(&bytes) {
            utf8.to_owned()
        } else {
            bytes.iter().map(|&b| b as char).collect()
        };

        let root = Node::parse(&text).map_err(|source| XmlError::Parse {
            path: path.display().to_string(),
            source,
        })?;

        if root.name.is_empty() && root.children.is_empty() {
            return Err(XmlError::Empty {
                path: path.display().to_string(),
            });
        }

        Ok(root)
    }

    fn open(e: &quick_xml::events::BytesStart) -> Result<Node, quick_xml::Error> {
        let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();

        let mut attrs = Vec::new();
        for attr in e.attributes().with_checks(false) {
            let Ok(attr) = attr else { continue };
            let key = String::from_utf8_lossy(attr.key.local_name().as_ref()).into_owned();
            let value = attr
                .unescape_value()
                .map(|v| v.into_owned())
                .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).into_owned());
            attrs.push((key, value));
        }

        Ok(Node {
            name,
            attrs,
            text: String::new(),
            children: Vec::new(),
        })
    }

    // -- reading ------------------------------------------------------------------------------

    /// The value of an attribute, if present.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The first child element with this name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every child element with this name, in document order.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Whether a child element is present at all, the shape of every flag in these files, which
    /// are written `<Enemy/>` and carry no value.
    pub fn has(&self, name: &str) -> bool {
        self.child(name).is_some()
    }

    /// The text of a named child, or of a same-named attribute when the field is spelled that way.
    ///
    /// Several fields appear as an element in one file and an attribute in another; taking both
    /// here saves every caller from checking twice.
    pub fn field(&self, name: &str) -> Option<&str> {
        if let Some(child) = self.child(name) {
            return Some(child.text.as_str());
        }
        self.attr(name)
    }

    /// A named field as an integer, accepting `0x`-prefixed hex.
    pub fn int(&self, name: &str) -> Option<i64> {
        self.field(name).and_then(parse_int)
    }

    /// A named field as a float, accepting `0x`-prefixed hex and bare decimals.
    pub fn float(&self, name: &str) -> Option<f64> {
        self.field(name).and_then(parse_float)
    }

    /// This element's own text as an integer.
    pub fn as_int(&self) -> Option<i64> {
        parse_int(&self.text)
    }

    /// This element's own text as a float.
    pub fn as_float(&self) -> Option<f64> {
        parse_float(&self.text)
    }

    /// An attribute as an integer.
    pub fn attr_int(&self, name: &str) -> Option<i64> {
        self.attr(name).and_then(parse_int)
    }

    /// An attribute as a float.
    pub fn attr_float(&self, name: &str) -> Option<f64> {
        self.attr(name).and_then(parse_float)
    }
}

/// Parses an integer the way the content files write them: decimal, or `0x`-prefixed hex.
///
/// Object and ground types are written in hex (`type="0xc85"`), while sizes and costs are decimal.
/// A leading `-` is honoured for both, and a float-looking value truncates rather than failing,
/// `<Size>100.0</Size>` appears in the data and the C# loader accepted it.
pub fn parse_int(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };

    let magnitude = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        i64::from_str_radix(hex, 16).ok()?
    } else if let Ok(value) = digits.parse::<i64>() {
        value
    } else {
        // Truncating toward zero matches the C# loader, which pushed such values through an
        // int cast rather than rejecting them.
        digits.parse::<f64>().ok()? as i64
    };

    Some(if negative { -magnitude } else { magnitude })
}

/// Parses a float, accepting the same hex spelling integers use.
pub fn parse_float(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.contains("0x") || text.contains("0X") {
        return parse_int(text).map(|v| v as f64);
    }
    text.parse::<f64>().ok().or_else(|| {
        // `.5` is written without a leading zero in several animation offsets.
        if let Some(fraction) = text.strip_prefix('.') {
            format!("0.{fraction}").parse::<f64>().ok()
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_parse_in_every_spelling_the_content_uses() {
        assert_eq!(parse_int("0xc85"), Some(0xc85));
        assert_eq!(parse_int("0X1F"), Some(0x1f));
        assert_eq!(parse_int("100"), Some(100));
        assert_eq!(parse_int("-3"), Some(-3));
        assert_eq!(parse_int("100.0"), Some(100));
        assert_eq!(parse_int(""), None);
        assert_eq!(parse_int("bogus"), None);

        assert_eq!(parse_float(".5"), Some(0.5));
        assert_eq!(parse_float("0.7"), Some(0.7));
        assert_eq!(parse_float("-1.25"), Some(-1.25));
        assert_eq!(parse_float("0x10"), Some(16.0));
    }

    #[test]
    fn presence_only_elements_read_as_flags() {
        let root = Node::parse(
            r#"<Object type="0xc85" id="Common Feline Egg">
                 <Class>Equipment</Class>
                 <Item/>
                 <Texture><File>lofiObj2</File><Index>0x100</Index></Texture>
                 <Tier>0</Tier>
               </Object>"#,
        )
        .unwrap();

        assert_eq!(root.name, "Object");
        assert_eq!(root.attr("id"), Some("Common Feline Egg"));
        assert_eq!(root.attr_int("type"), Some(0xc85));
        assert!(root.has("Item"));
        assert!(!root.has("Consumable"));
        assert_eq!(root.field("Class"), Some("Equipment"));
        assert_eq!(root.int("Tier"), Some(0));
        assert_eq!(root.child("Texture").unwrap().int("Index"), Some(0x100));
    }

    #[test]
    fn a_field_spelled_as_an_attribute_reads_the_same_as_an_element() {
        let root = Node::parse(r#"<Ground id="Grass" Speed="1.0"><Sink/></Ground>"#).unwrap();
        assert_eq!(root.float("Speed"), Some(1.0));
        assert!(root.has("Sink"));
    }

    #[test]
    fn repeated_children_are_all_kept() {
        let root = Node::parse(
            r#"<Object id="x">
                 <Projectile id="0"><Speed>100</Speed></Projectile>
                 <Projectile id="1"><Speed>120</Speed></Projectile>
               </Object>"#,
        )
        .unwrap();

        let shots: Vec<_> = root.children_named("Projectile").collect();
        assert_eq!(shots.len(), 2);
        assert_eq!(shots[1].int("Speed"), Some(120));
    }

    #[test]
    fn an_unbalanced_document_still_yields_what_it_contained() {
        // The realm regions file shipped for months with a missing `</Region>`; the old server
        // refused to boot on it. Losing one element is survivable, losing the file is not.
        let root = Node::parse(r#"<Regions><Region id="a"><Region id="b"></Regions>"#).unwrap();
        assert_eq!(root.name, "Regions");
        assert!(!root.children.is_empty());
    }
}

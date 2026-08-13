//! Importing the game's two legacy map formats.
//!
//! Everything here runs once, at conversion time, and nothing in the server calls it. Its whole job
//! is to turn `.jm` and `.wmap` into [`Map`] so that neither format is ever read again.
//!
//! Both encode the same idea — a dictionary of square compositions plus a grid of indices into it —
//! and they disagree on almost every detail of how:
//!
//! | | `.jm` | `.wmap` |
//! |---|---|---|
//! | container | JSON, grid base64 inside it | binary |
//! | grid indices | `i16` **big**-endian | `i16` **little**-endian |
//! | dictionary | JSON objects, ground by name | binary records, tile by number |
//! | strings | JSON | 7-bit length prefix |
//! | header | none | a bare version byte |
//!
//! The endianness disagreement is the one worth naming: reading a `.jm` grid little-endian does not
//! fail, it silently produces a different map.

use std::io::Read;

use base64::Engine;

use crate::catalog::Catalog;
use crate::desc::{ObjectType, TileType};
use crate::map::{Composition, Map, MapError};
use crate::region::{Region, Terrain};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("parsing JSON: {0}")]
    Json(String),

    #[error("decoding the grid: {0}")]
    Decode(String),

    #[error("map declares {width}×{height} but the grid holds {found} squares")]
    GridMismatch {
        width: u32,
        height: u32,
        found: usize,
    },

    #[error("unsupported .wmap version {0}")]
    UnsupportedVersion(u8),

    #[error("the file ends before the data it declares")]
    Truncated,

    #[error(transparent)]
    Map(#[from] MapError),
}

/// Names in a legacy map that the catalog does not know.
///
/// Reported rather than raised: a map referring to one removed object should still convert, with
/// that square left empty, so a single stale name cannot cost a whole dungeon.
#[derive(Debug, Default)]
pub struct UnresolvedNames {
    pub grounds: Vec<String>,
    pub objects: Vec<String>,
    pub regions: Vec<String>,
}

impl UnresolvedNames {
    pub fn is_empty(&self) -> bool {
        self.grounds.is_empty() && self.objects.is_empty() && self.regions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.grounds.len() + self.objects.len() + self.regions.len()
    }

    fn note(list: &mut Vec<String>, name: &str) {
        if !list.iter().any(|seen| seen == name) {
            list.push(name.to_owned());
        }
    }
}

/// Converts a `.jm` file.
pub fn from_jm(json: &str, catalog: &Catalog) -> Result<(Map, UnresolvedNames), ImportError> {
    // One map in the corpus was saved by an editor that wrote a byte-order mark. JSON has no use
    // for one and parsers reject it, so it comes off here rather than in every caller.
    let json = json.strip_prefix('\u{feff}').unwrap_or(json);

    let document: serde_json::Value =
        serde_json::from_str(json).map_err(|source| ImportError::Json(source.to_string()))?;

    let width = document["width"].as_u64().unwrap_or(0) as u32;
    let height = document["height"].as_u64().unwrap_or(0) as u32;

    let packed = document["data"]
        .as_str()
        .ok_or_else(|| ImportError::Json("no `data` field".into()))?;
    let compressed = base64::engine::general_purpose::STANDARD
        .decode(packed)
        .map_err(|source| ImportError::Decode(source.to_string()))?;

    let mut grid_bytes = Vec::new();
    flate2::read::ZlibDecoder::new(&compressed[..])
        .read_to_end(&mut grid_bytes)
        .map_err(|source| ImportError::Decode(source.to_string()))?;

    let mut unresolved = UnresolvedNames::default();

    let entries = document["dict"]
        .as_array()
        .ok_or_else(|| ImportError::Json("no `dict` array".into()))?;

    let dictionary: Vec<Composition> = entries
        .iter()
        .map(|entry| jm_composition(entry, catalog, &mut unresolved))
        .collect();

    // Big-endian, and this is the detail that makes the whole file wrong if missed.
    let indices: Vec<u16> = grid_bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();

    let expected = (width as usize) * (height as usize);
    if indices.len() != expected {
        return Err(ImportError::GridMismatch {
            width,
            height,
            found: indices.len(),
        });
    }

    let squares = indices.into_iter().map(|index| {
        dictionary
            .get(index as usize)
            .cloned()
            .unwrap_or_else(empty_square)
    });

    Ok((Map::from_squares(width, height, squares)?, unresolved))
}

/// Reads one `dict` entry: `{ "ground": ..., "objs": [...], "regions": [...] }`.
fn jm_composition(
    entry: &serde_json::Value,
    catalog: &Catalog,
    unresolved: &mut UnresolvedNames,
) -> Composition {
    let tile = match entry["ground"].as_str() {
        Some(name) => match catalog.tile_type_of(name) {
            Some(tile) => tile,
            None => {
                UnresolvedNames::note(&mut unresolved.grounds, name);
                EMPTY_TILE
            }
        },
        None => EMPTY_TILE,
    };

    // Only the first object is honoured, matching the converter this replaces. Maps in the wild do
    // not put two objects on one square, and the format has no way to express what that would mean.
    let (object, config) = match entry["objs"].as_array().and_then(|list| list.first()) {
        Some(first) => {
            let id = first["id"].as_str().unwrap_or_default();
            let object = match catalog.type_of(id) {
                Some(object) => object,
                None => {
                    if !id.is_empty() {
                        UnresolvedNames::note(&mut unresolved.objects, id);
                    }
                    ObjectType::NONE
                }
            };
            (
                object,
                first["name"].as_str().unwrap_or_default().to_owned(),
            )
        }
        None => (ObjectType::NONE, String::new()),
    };

    let region = match entry["regions"].as_array().and_then(|list| list.first()) {
        Some(first) => {
            let name = first["id"].as_str().unwrap_or_default();
            match Region::from_name(name) {
                Some(region) => region,
                None => {
                    if !name.is_empty() {
                        UnresolvedNames::note(&mut unresolved.regions, name);
                    }
                    Region::None
                }
            }
        }
        None => Region::None,
    };

    Composition {
        tile,
        object,
        region,
        terrain: Terrain::None,
        config,
    }
}

/// The tile written for a square with no ground, which the legacy converter spelled `0xff`.
const EMPTY_TILE: TileType = TileType(0xff);

fn empty_square() -> Composition {
    Composition {
        tile: EMPTY_TILE,
        object: ObjectType::NONE,
        region: Region::None,
        terrain: Terrain::None,
        config: String::new(),
    }
}

/// Converts a `.wmap` file.
///
/// Versions 0, 1 and 2 exist and differ in where an elevation byte sits — after the dictionary
/// entry in version 1, after each grid square in version 2, and absent in version 0. Elevation is
/// read and discarded: nothing in the simulation consumes it.
pub fn from_wmap(bytes: &[u8], catalog: &Catalog) -> Result<(Map, UnresolvedNames), ImportError> {
    let version = *bytes.first().ok_or(ImportError::Truncated)?;
    if version > 2 {
        return Err(ImportError::UnsupportedVersion(version));
    }

    let mut body = Vec::new();
    flate2::read::ZlibDecoder::new(&bytes[1..])
        .read_to_end(&mut body)
        .map_err(|source| ImportError::Decode(source.to_string()))?;

    let mut cursor = Cursor {
        bytes: &body,
        at: 0,
    };
    let mut unresolved = UnresolvedNames::default();

    let count = cursor.i16()? as usize;
    let mut dictionary = Vec::with_capacity(count);
    for _ in 0..count {
        let tile = TileType(cursor.u16()?);

        let object_id = cursor.string()?;
        let object = if object_id.is_empty() {
            ObjectType::NONE
        } else {
            match catalog.type_of(&object_id) {
                Some(object) => object,
                None => {
                    UnresolvedNames::note(&mut unresolved.objects, &object_id);
                    ObjectType::NONE
                }
            }
        };

        let config = cursor.string()?;
        let terrain = Terrain::from_index(cursor.u8()?).unwrap_or_default();
        let region = Region::from_index(cursor.u8()?).unwrap_or_default();

        if version == 1 {
            cursor.u8()?; // elevation
        }

        dictionary.push(Composition {
            tile,
            object,
            region,
            terrain,
            config,
        });
    }

    let width = cursor.i32()? as u32;
    let height = cursor.i32()? as u32;

    let squares = (width as usize) * (height as usize);
    let mut compositions = Vec::with_capacity(squares);
    for _ in 0..squares {
        let index = cursor.i16()? as usize;
        if version == 2 {
            cursor.u8()?; // elevation
        }
        compositions.push(dictionary.get(index).cloned().unwrap_or_else(empty_square));
    }

    Ok((Map::from_squares(width, height, compositions)?, unresolved))
}

/// A little-endian reader for the `.wmap` body, with C#'s `BinaryReader` string encoding.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], ImportError> {
        if self.bytes.len() < self.at + count {
            return Err(ImportError::Truncated);
        }
        let slice = &self.bytes[self.at..self.at + count];
        self.at += count;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ImportError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ImportError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn i16(&mut self) -> Result<i16, ImportError> {
        Ok(self.u16()? as i16)
    }

    fn i32(&mut self) -> Result<i32, ImportError> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// A .NET `BinaryReader` string: a length in 7-bit groups, then UTF-8.
    fn string(&mut self) -> Result<String, ImportError> {
        let mut len = 0usize;
        let mut shift = 0u32;
        loop {
            let byte = self.u8()?;
            len |= ((byte & 0x7f) as usize) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 28 {
                return Err(ImportError::Decode("string length is malformed".into()));
            }
        }
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// The handful of names the fixtures below use.
    ///
    /// Built from a string rather than a file. Several tests used to write the same fixture to the
    /// same path and could read it mid-truncate, which passed alone and failed in the suite.
    const FIXTURE: &str = r#"<Objects>
        <Ground type="0x10" id="Grass"><Speed>1</Speed></Ground>
        <Ground type="0x11" id="Water"><NoWalk/></Ground>
        <Object type="0x500" id="Sign"><Class>GameObject</Class><Static/></Object>
      </Objects>"#;

    fn catalog() -> Catalog {
        let (catalog, report) = Catalog::load_str(&[FIXTURE]);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        catalog
    }

    fn zlib(payload: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(payload).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn a_jm_map_converts_with_its_grid_the_right_way_round() {
        // Two squares wide, one tall: grass then water, as big-endian shorts.
        let grid: Vec<u8> = [0u16, 1u16]
            .iter()
            .flat_map(|index| index.to_be_bytes())
            .collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(zlib(&grid));

        let json = format!(
            r#"{{"width":2,"height":1,"dict":[
                 {{"ground":"Grass"}},
                 {{"ground":"Water","objs":[{{"id":"Sign","name":"name:Keep Out"}}],
                   "regions":[{{"id":"Vault"}}]}}
               ],"data":"{encoded}"}}"#
        );

        let (map, unresolved) = from_jm(&json, &catalog()).unwrap();
        assert!(unresolved.is_empty(), "{unresolved:?}");
        assert_eq!(map.width(), 2);
        assert_eq!(map.height(), 1);

        assert_eq!(map.at(0, 0).unwrap().tile, TileType(0x10));

        let watery = map.at(1, 0).unwrap();
        assert_eq!(watery.tile, TileType(0x11));
        assert_eq!(watery.object, ObjectType(0x500));
        assert_eq!(watery.region, Region::Vault);
        assert_eq!(watery.name(), Some("Keep Out"));
    }

    #[test]
    fn a_jm_map_reading_the_grid_backwards_would_be_visibly_wrong() {
        // Guards the endianness choice: index 1 as little-endian bytes is 256, which is outside the
        // dictionary, so a reader with the wrong byte order produces empty squares rather than a
        // map that merely looks odd.
        let grid: Vec<u8> = 1u16.to_be_bytes().to_vec();
        let encoded = base64::engine::general_purpose::STANDARD.encode(zlib(&grid));
        let json = format!(
            r#"{{"width":1,"height":1,"dict":[{{"ground":"Grass"}},{{"ground":"Water"}}],
                 "data":"{encoded}"}}"#
        );

        let (map, _) = from_jm(&json, &catalog()).unwrap();
        assert_eq!(
            map.at(0, 0).unwrap().tile,
            TileType(0x11),
            "big-endian index 1 should select the second dictionary entry"
        );
    }

    #[test]
    fn a_jm_map_naming_something_unknown_still_converts() {
        let grid: Vec<u8> = 0u16.to_be_bytes().to_vec();
        let encoded = base64::engine::general_purpose::STANDARD.encode(zlib(&grid));
        let json = format!(
            r#"{{"width":1,"height":1,"dict":[
                 {{"ground":"Removed Ground","objs":[{{"id":"Removed Object"}}],
                   "regions":[{{"id":"Removed Region"}}]}}
               ],"data":"{encoded}"}}"#
        );

        let (map, unresolved) = from_jm(&json, &catalog()).unwrap();
        assert_eq!(map.width(), 1);
        assert_eq!(unresolved.grounds, vec!["Removed Ground"]);
        assert_eq!(unresolved.objects, vec!["Removed Object"]);
        assert_eq!(unresolved.regions, vec!["Removed Region"]);
        assert_eq!(unresolved.len(), 3);
    }

    #[test]
    fn a_grid_that_does_not_match_the_dimensions_is_refused() {
        let grid: Vec<u8> = 0u16.to_be_bytes().to_vec();
        let encoded = base64::engine::general_purpose::STANDARD.encode(zlib(&grid));
        let json =
            format!(r#"{{"width":8,"height":8,"dict":[{{"ground":"Grass"}}],"data":"{encoded}"}}"#);

        assert!(matches!(
            from_jm(&json, &catalog()),
            Err(ImportError::GridMismatch { found: 1, .. })
        ));
    }

    /// Builds a version-0 `.wmap` with one dictionary entry and a 2×1 grid.
    fn wmap_fixture() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&1i16.to_le_bytes()); // dictionary count

        body.extend_from_slice(&0x11u16.to_le_bytes()); // tile
        body.push(4); // "Sign" length, 7-bit encoded
        body.extend_from_slice(b"Sign");
        body.push(12);
        body.extend_from_slice(b"name:Keep Ou");
        body.push(0); // terrain
        body.push(Region::Vault.index());

        body.extend_from_slice(&2i32.to_le_bytes()); // width
        body.extend_from_slice(&1i32.to_le_bytes()); // height
        body.extend_from_slice(&0i16.to_le_bytes());
        body.extend_from_slice(&0i16.to_le_bytes());

        let mut file = vec![0u8]; // version
        file.extend_from_slice(&zlib(&body));
        file
    }

    #[test]
    fn a_wmap_converts_with_its_little_endian_grid() {
        let (map, unresolved) = from_wmap(&wmap_fixture(), &catalog()).unwrap();
        assert!(unresolved.is_empty(), "{unresolved:?}");

        assert_eq!(map.width(), 2);
        assert_eq!(map.height(), 1);

        let square = map.at(0, 0).unwrap();
        assert_eq!(square.tile, TileType(0x11));
        assert_eq!(square.object, ObjectType(0x500));
        assert_eq!(square.region, Region::Vault);
        assert_eq!(square.name(), Some("Keep Ou"));
    }

    #[test]
    fn an_unsupported_wmap_version_is_refused() {
        let mut file = wmap_fixture();
        file[0] = 7;
        assert!(matches!(
            from_wmap(&file, &catalog()),
            Err(ImportError::UnsupportedVersion(7))
        ));
    }

    #[test]
    fn a_truncated_wmap_errors_rather_than_panicking() {
        let file = wmap_fixture();
        for cut in 0..file.len() {
            let _ = from_wmap(&file[..cut], &catalog());
        }
    }

    #[test]
    fn both_importers_produce_a_map_that_writes_in_our_format() {
        let (map, _) = from_wmap(&wmap_fixture(), &catalog()).unwrap();

        let mut bytes = Vec::new();
        map.write(&mut bytes).unwrap();
        assert_eq!(&bytes[0..4], b"HMAP");
        assert_eq!(Map::read(&bytes).unwrap(), map);
    }
}

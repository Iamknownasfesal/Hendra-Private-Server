//! The map format, and the grid it describes.
//!
//! # Why a format of our own
//!
//! The game shipped two, and neither is worth keeping. `.jm` is JSON wrapping a base64 blob of
//! big-endian shorts; `.wmap` is a binary format whose only header is a bare version byte, read
//! little-endian. Two encodings, opposite byte orders, and no way to tell either one from an
//! unrelated file except by trying to parse it. Both are now import formats — see [`crate::legacy`]
//! — and this is what the server actually reads.
//!
//! # Layout
//!
//! ```text
//!   "HMAP"        4 bytes, so a wrong file fails immediately and by name
//!   version       u16
//!   index_width   u8      1 when the dictionary fits in a byte, else 2
//!   reserved      u8
//!   width         u32
//!   height        u32
//!   ── everything below is deflate-compressed ──
//!   dict_count    u16
//!   dictionary    dict_count × { tile u16, object u16, region u8, terrain u8, config string }
//!   grid          width × height × index_width, row-major
//! ```
//!
//! The dictionary is the reason maps are small. A square's whole composition — ground, object,
//! region, terrain, configuration — repeats constantly across a map, so the grid stores an index
//! into a table of distinct compositions rather than the composition itself. The realm has millions
//! of squares and, in practice, a few hundred distinct ones.
//!
//! Sizing the index by the dictionary halves most maps again: a map with fewer than 257 distinct
//! squares spends one byte per tile instead of two, and almost all of them do.

use std::io::{Read, Write};

use crate::desc::{ObjectType, TileType};
use crate::region::{Region, Terrain};

/// Identifies the file. Four bytes rather than a version byte alone, because the legacy format's
/// bare version byte means any file at all "parses" until it runs out of plausible data.
pub const MAGIC: [u8; 4] = *b"HMAP";

/// The current format version.
pub const VERSION: u16 = 1;

/// The most squares a map may declare, as a guard against a corrupt header.
const MAX_DIMENSION: u32 = 8192;

/// One distinct square composition.
///
/// Deliberately whole-value: two squares are the same entry only if every field matches, which is
/// what lets the grid be a table of indices.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Composition {
    pub tile: TileType,
    pub object: ObjectType,
    pub region: Region,
    pub terrain: Terrain,

    /// Per-square object settings, in the game's own `key:value;key:value` form.
    ///
    /// Kept verbatim rather than parsed into fields because the keys are open-ended — `name`,
    /// `size`, `eff`, `conn` and others appear — and an unrecognised one should travel through the
    /// pipeline rather than be dropped by it. [`Composition::settings`] reads them.
    pub config: String,
}

impl Composition {
    /// Whether a square holds an object at all.
    pub fn has_object(&self) -> bool {
        !self.object.is_none()
    }

    /// Walks the configuration as key/value pairs.
    pub fn settings(&self) -> impl Iterator<Item = (&str, &str)> {
        self.config.split(';').filter_map(|pair| {
            let (key, value) = pair.split_once(':')?;
            Some((key.trim(), value.trim()))
        })
    }

    /// The `name` setting, which overrides an object's displayed name.
    pub fn name(&self) -> Option<&str> {
        self.settings()
            .find(|(key, _)| *key == "name")
            .map(|(_, value)| value)
    }

    /// The `size` setting, as a percentage of the object's natural size.
    pub fn size(&self) -> Option<i32> {
        self.settings()
            .find(|(key, _)| *key == "size")
            .and_then(|(_, value)| crate::xml::parse_int(value))
            .map(|value| value as i32)
    }
}

/// A loaded map.
#[derive(Debug, Clone, PartialEq)]
pub struct Map {
    width: u32,
    height: u32,
    dictionary: Vec<Composition>,

    /// One index per square, row-major. Held as `u16` in memory regardless of how it was stored,
    /// because the saving is a disk concern and branching per lookup is not worth it.
    grid: Vec<u16>,
}

#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("not a Hendra map: expected magic {expected:?}, found {found:?}")]
    WrongMagic { expected: [u8; 4], found: [u8; 4] },

    #[error("map version {found} is not supported (this build reads {supported})")]
    UnsupportedVersion { found: u16, supported: u16 },

    #[error("map dimensions {width}×{height} are out of range")]
    BadDimensions { width: u32, height: u32 },

    #[error("index width {0} is not 1 or 2")]
    BadIndexWidth(u8),

    #[error("square {at} names dictionary entry {index}, but there are only {count}")]
    BadDictionaryIndex {
        at: usize,
        index: usize,
        count: usize,
    },

    #[error("the file ends before the data it declares")]
    Truncated,

    #[error("reading the map: {0}")]
    Io(String),
}

impl Map {
    /// Builds a map from a grid of compositions in row-major order.
    ///
    /// Deduplication happens here, so callers — importers included — can hand over one composition
    /// per square without thinking about the dictionary.
    pub fn from_squares(
        width: u32,
        height: u32,
        squares: impl IntoIterator<Item = Composition>,
    ) -> Result<Map, MapError> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(MapError::BadDimensions { width, height });
        }

        let expected = (width as usize) * (height as usize);
        let mut dictionary: Vec<Composition> = Vec::new();
        let mut lookup: std::collections::HashMap<Composition, u16> =
            std::collections::HashMap::new();
        let mut grid = Vec::with_capacity(expected);

        for square in squares {
            let index = match lookup.get(&square) {
                Some(&index) => index,
                None => {
                    let index = dictionary.len() as u16;
                    lookup.insert(square.clone(), index);
                    dictionary.push(square);
                    index
                }
            };
            grid.push(index);
        }

        if grid.len() != expected {
            return Err(MapError::Truncated);
        }

        Ok(Map {
            width,
            height,
            dictionary,
            grid,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// How many distinct square compositions the map uses.
    pub fn distinct_squares(&self) -> usize {
        self.dictionary.len()
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x < self.width && y < self.height
    }

    /// The composition at a square, or `None` outside the map.
    #[inline]
    pub fn at(&self, x: u32, y: u32) -> Option<&Composition> {
        if !self.contains(x, y) {
            return None;
        }
        let index = self.grid[(y as usize) * (self.width as usize) + (x as usize)];
        self.dictionary.get(index as usize)
    }

    /// Every square that holds an object, with its position.
    ///
    /// This is what a world uses to populate itself, and it is a filtered scan rather than a stored
    /// list because the dictionary already tells us which entries have objects.
    pub fn objects(&self) -> impl Iterator<Item = (u32, u32, &Composition)> {
        self.grid
            .iter()
            .enumerate()
            .filter_map(move |(at, &index)| {
                let square = self.dictionary.get(index as usize)?;
                if !square.has_object() {
                    return None;
                }
                let x = (at % self.width as usize) as u32;
                let y = (at / self.width as usize) as u32;
                Some((x, y, square))
            })
    }

    /// Every square carrying a region marker.
    pub fn regions(&self) -> impl Iterator<Item = (u32, u32, Region)> {
        self.grid
            .iter()
            .enumerate()
            .filter_map(move |(at, &index)| {
                let square = self.dictionary.get(index as usize)?;
                if square.region == Region::None {
                    return None;
                }
                let x = (at % self.width as usize) as u32;
                let y = (at / self.width as usize) as u32;
                Some((x, y, square.region))
            })
    }

    // -- format --------------------------------------------------------------------------------

    /// How wide a grid index has to be for this dictionary.
    fn index_width(&self) -> u8 {
        if self.dictionary.len() <= u8::MAX as usize + 1 {
            1
        } else {
            2
        }
    }

    /// Writes the map in our format.
    pub fn write(&self, out: &mut Vec<u8>) -> Result<(), MapError> {
        let index_width = self.index_width();

        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.push(index_width);
        out.push(0); // reserved
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());

        let mut body = Vec::new();
        body.extend_from_slice(&(self.dictionary.len() as u16).to_le_bytes());
        for square in &self.dictionary {
            body.extend_from_slice(&square.tile.0.to_le_bytes());
            body.extend_from_slice(&square.object.0.to_le_bytes());
            body.push(square.region.index());
            body.push(square.terrain.index());

            let config = square.config.as_bytes();
            let len = config.len().min(u16::MAX as usize);
            body.extend_from_slice(&(len as u16).to_le_bytes());
            body.extend_from_slice(&config[..len]);
        }

        for &index in &self.grid {
            if index_width == 1 {
                body.push(index as u8);
            } else {
                body.extend_from_slice(&index.to_le_bytes());
            }
        }

        let mut encoder = flate2::write::ZlibEncoder::new(out, flate2::Compression::default());
        encoder
            .write_all(&body)
            .map_err(|source| MapError::Io(source.to_string()))?;
        encoder
            .finish()
            .map_err(|source| MapError::Io(source.to_string()))?;

        Ok(())
    }

    /// Reads a map in our format.
    pub fn read(bytes: &[u8]) -> Result<Map, MapError> {
        if bytes.len() < 16 {
            return Err(MapError::Truncated);
        }

        let found: [u8; 4] = bytes[0..4].try_into().expect("checked length");
        if found != MAGIC {
            return Err(MapError::WrongMagic {
                expected: MAGIC,
                found,
            });
        }

        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != VERSION {
            return Err(MapError::UnsupportedVersion {
                found: version,
                supported: VERSION,
            });
        }

        let index_width = bytes[6];
        if index_width != 1 && index_width != 2 {
            return Err(MapError::BadIndexWidth(index_width));
        }

        let width = u32::from_le_bytes(bytes[8..12].try_into().expect("checked length"));
        let height = u32::from_le_bytes(bytes[12..16].try_into().expect("checked length"));
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(MapError::BadDimensions { width, height });
        }

        let mut body = Vec::new();
        flate2::read::ZlibDecoder::new(&bytes[16..])
            .read_to_end(&mut body)
            .map_err(|source| MapError::Io(source.to_string()))?;

        let mut cursor = 0usize;
        let mut take = |count: usize| -> Result<&[u8], MapError> {
            if body.len() < cursor + count {
                return Err(MapError::Truncated);
            }
            let slice = &body[cursor..cursor + count];
            cursor += count;
            Ok(slice)
        };

        let dict_count = u16::from_le_bytes(take(2)?.try_into().expect("two bytes")) as usize;
        let mut dictionary = Vec::with_capacity(dict_count);
        for _ in 0..dict_count {
            let head = take(6)?;
            let tile = TileType(u16::from_le_bytes([head[0], head[1]]));
            let object = ObjectType(u16::from_le_bytes([head[2], head[3]]));
            let region = Region::from_index(head[4]).unwrap_or_default();
            let terrain = Terrain::from_index(head[5]).unwrap_or_default();

            let len = u16::from_le_bytes(take(2)?.try_into().expect("two bytes")) as usize;
            let config = String::from_utf8_lossy(take(len)?).into_owned();

            dictionary.push(Composition {
                tile,
                object,
                region,
                terrain,
                config,
            });
        }

        let squares = (width as usize) * (height as usize);
        let mut grid = Vec::with_capacity(squares);
        for at in 0..squares {
            let index = if index_width == 1 {
                take(1)?[0] as u16
            } else {
                u16::from_le_bytes(take(2)?.try_into().expect("two bytes"))
            };

            if index as usize >= dictionary.len() {
                return Err(MapError::BadDictionaryIndex {
                    at,
                    index: index as usize,
                    count: dictionary.len(),
                });
            }
            grid.push(index);
        }

        Ok(Map {
            width,
            height,
            dictionary,
            grid,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(tile: u16, object: u16, region: Region) -> Composition {
        Composition {
            tile: TileType(tile),
            object: ObjectType(object),
            region,
            terrain: Terrain::None,
            config: String::new(),
        }
    }

    fn checkerboard(width: u32, height: u32) -> Map {
        let squares = (0..width * height).map(|n| {
            if n % 2 == 0 {
                square(0x10, ObjectType::NONE.0, Region::None)
            } else {
                square(0x11, ObjectType::NONE.0, Region::None)
            }
        });
        Map::from_squares(width, height, squares).unwrap()
    }

    #[test]
    fn repeated_squares_collapse_into_a_dictionary() {
        let map = checkerboard(64, 64);
        assert_eq!(map.distinct_squares(), 2, "a checkerboard has two squares");
        assert_eq!(map.width(), 64);
        assert_eq!(map.height(), 64);
    }

    #[test]
    fn a_map_survives_a_round_trip_through_the_format() {
        let map = checkerboard(37, 21);

        let mut bytes = Vec::new();
        map.write(&mut bytes).unwrap();
        let loaded = Map::read(&bytes).unwrap();

        assert_eq!(loaded, map);
        assert_eq!(&bytes[0..4], b"HMAP");
    }

    #[test]
    fn squares_read_back_at_the_right_coordinates() {
        let mut squares: Vec<Composition> = (0..6 * 4)
            .map(|_| square(0x10, ObjectType::NONE.0, Region::None))
            .collect();
        // Row-major: (x=2, y=3) is index 3*6 + 2.
        squares[3 * 6 + 2] = square(0x77, 0x900, Region::Vault);

        let map = Map::from_squares(6, 4, squares).unwrap();
        let mut bytes = Vec::new();
        map.write(&mut bytes).unwrap();
        let loaded = Map::read(&bytes).unwrap();

        let found = loaded.at(2, 3).unwrap();
        assert_eq!(found.tile, TileType(0x77));
        assert_eq!(found.region, Region::Vault);

        assert_eq!(loaded.at(0, 0).unwrap().tile, TileType(0x10));
        assert!(loaded.at(6, 0).is_none(), "outside the map");
        assert!(loaded.at(0, 4).is_none(), "outside the map");
    }

    #[test]
    fn a_small_dictionary_spends_one_byte_per_square() {
        let map = checkerboard(100, 100);
        assert_eq!(map.index_width(), 1);

        // A dictionary of exactly 256 still fits a byte; 257 does not.
        let wide = Map::from_squares(
            300,
            1,
            (0..300u32).map(|n| square((n % 257) as u16, ObjectType::NONE.0, Region::None)),
        )
        .unwrap();
        assert_eq!(wide.distinct_squares(), 257);
        assert_eq!(wide.index_width(), 2);
    }

    #[test]
    fn objects_and_regions_report_their_positions() {
        let mut squares: Vec<Composition> = (0..5 * 5)
            .map(|_| square(0x10, ObjectType::NONE.0, Region::None))
            .collect();
        squares[2 * 5 + 1] = square(0x10, 0x500, Region::None);
        squares[4 * 5 + 3] = square(0x10, ObjectType::NONE.0, Region::Spawn);

        let map = Map::from_squares(5, 5, squares).unwrap();

        let objects: Vec<(u32, u32)> = map.objects().map(|(x, y, _)| (x, y)).collect();
        assert_eq!(objects, vec![(1, 2)]);

        let regions: Vec<(u32, u32, Region)> = map.regions().collect();
        assert_eq!(regions, vec![(3, 4, Region::Spawn)]);
    }

    #[test]
    fn object_settings_parse_out_of_the_configuration() {
        let composition = Composition {
            tile: TileType(1),
            object: ObjectType(2),
            region: Region::None,
            terrain: Terrain::None,
            config: "name:Guard Post;size:140;eff:0x2".into(),
        };

        assert_eq!(composition.name(), Some("Guard Post"));
        assert_eq!(composition.size(), Some(140));

        let settings: Vec<(&str, &str)> = composition.settings().collect();
        assert_eq!(settings.len(), 3);
        assert_eq!(settings[2], ("eff", "0x2"));
    }

    #[test]
    fn an_empty_configuration_yields_no_settings() {
        let plain = square(1, 2, Region::None);
        assert_eq!(plain.settings().count(), 0);
        assert_eq!(plain.name(), None);
        assert_eq!(plain.size(), None);
    }

    // -- refusing bad input --------------------------------------------------------------------

    #[test]
    fn a_foreign_file_is_named_rather_than_misparsed() {
        // The legacy .wmap started with a bare version byte, so any file at all began to parse.
        let legacy = [
            0x01u8, 0x78, 0x9c, 0x00, 0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert!(matches!(
            Map::read(&legacy),
            Err(MapError::WrongMagic { .. })
        ));

        assert!(matches!(Map::read(b"{}"), Err(MapError::Truncated)));
    }

    #[test]
    fn a_future_version_is_refused_with_both_numbers() {
        let map = checkerboard(4, 4);
        let mut bytes = Vec::new();
        map.write(&mut bytes).unwrap();
        bytes[4] = 99;

        match Map::read(&bytes) {
            Err(MapError::UnsupportedVersion { found, supported }) => {
                assert_eq!(found, 99);
                assert_eq!(supported, VERSION);
            }
            other => panic!("expected a version refusal, got {other:?}"),
        }
    }

    #[test]
    fn absurd_dimensions_are_refused_before_allocating() {
        let map = checkerboard(4, 4);
        let mut bytes = Vec::new();
        map.write(&mut bytes).unwrap();
        bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());

        assert!(matches!(
            Map::read(&bytes),
            Err(MapError::BadDimensions { .. })
        ));

        assert!(matches!(
            Map::from_squares(0, 10, std::iter::empty()),
            Err(MapError::BadDimensions { .. })
        ));
    }

    #[test]
    fn a_truncated_file_errors_rather_than_panicking() {
        let map = checkerboard(20, 20);
        let mut bytes = Vec::new();
        map.write(&mut bytes).unwrap();

        for cut in 0..bytes.len() {
            let _ = Map::read(&bytes[..cut]);
        }
    }

    #[test]
    fn too_few_squares_is_refused() {
        assert!(matches!(
            Map::from_squares(10, 10, (0..50).map(|_| Composition::default())),
            Err(MapError::Truncated)
        ));
    }
}

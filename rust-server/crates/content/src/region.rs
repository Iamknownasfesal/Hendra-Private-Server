//! Tile regions and terrain bands.
//!
//! A region marks a square as meaning something to the simulation beyond its artwork: where
//! players arrive, where a shop stands, where loot may drop. The numbering is not cosmetic: the
//! binary map format stores it as a single byte, so these ordinals are a file format and reordering
//! them silently reinterprets every map on disk.

use std::fmt;

/// What a square is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Region {
    #[default]
    None = 0,
    Spawn = 1,
    RealmPortals = 2,
    Store1 = 3,
    Store2 = 4,
    Store3 = 5,
    Store4 = 6,
    Store5 = 7,
    Store6 = 8,
    Vault = 9,
    Loot = 10,
    Defender = 11,
    Hallway = 12,
    Enemy = 13,
    Hallway1 = 14,
    Hallway2 = 15,
    Hallway3 = 16,
    Store7 = 17,
    Store8 = 18,
    Store9 = 19,
    GiftingChest = 20,
    Store10 = 21,
    Store11 = 22,
    Store12 = 23,
    Store13 = 24,
    Store14 = 25,
    Store15 = 26,
    Store16 = 27,
    Store17 = 28,
    Store18 = 29,
    Store19 = 30,
    Store20 = 31,
    Store21 = 32,
    Store22 = 33,
    Store23 = 34,
    Store24 = 35,
    ItemSpawnPoint = 36,
    Store25 = 37,
    Store26 = 38,
    Store27 = 39,
    Store28 = 40,
    Store29 = 41,
    Store30 = 42,
    Store31 = 43,
    Store32 = 44,
    Store33 = 45,
    Store34 = 46,
    Store35 = 47,
    Store36 = 48,
    Store37 = 49,
    Store38 = 50,
    Store39 = 51,
    Store40 = 52,
    Biome1 = 53,
    Biome2 = 54,
    Biome3 = 55,
    Biome4 = 56,
}

/// How many regions are defined.
pub const REGION_COUNT: usize = 57;

impl Region {
    /// Every region, in ordinal order.
    ///
    /// Built from the discriminants rather than written out twice, so the list cannot drift from
    /// the enum.
    pub fn from_index(index: u8) -> Option<Region> {
        if (index as usize) >= REGION_COUNT {
            return None;
        }
        // Safe because the enum is `repr(u8)`, its discriminants are contiguous from zero, and the
        // bound above is the count of them.
        Some(unsafe { std::mem::transmute::<u8, Region>(index) })
    }

    pub fn index(self) -> u8 {
        self as u8
    }

    /// Parses the spelling the JSON maps use, where words are separated by spaces or underscores.
    pub fn from_name(name: &str) -> Option<Region> {
        let wanted: String = name
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '_')
            .flat_map(char::to_lowercase)
            .collect();

        (0..REGION_COUNT as u8)
            .filter_map(Region::from_index)
            .find(|region| {
                let candidate: String =
                    region.name().chars().flat_map(char::to_lowercase).collect();
                candidate == wanted
            })
    }

    /// The canonical name, matching the map editor's spelling with underscores removed.
    pub fn name(self) -> &'static str {
        use Region::*;
        match self {
            None => "None",
            Spawn => "Spawn",
            RealmPortals => "RealmPortals",
            Vault => "Vault",
            Loot => "Loot",
            Defender => "Defender",
            Hallway => "Hallway",
            Enemy => "Enemy",
            Hallway1 => "Hallway1",
            Hallway2 => "Hallway2",
            Hallway3 => "Hallway3",
            GiftingChest => "GiftingChest",
            ItemSpawnPoint => "ItemSpawnPoint",
            Biome1 => "Biome1",
            Biome2 => "Biome2",
            Biome3 => "Biome3",
            Biome4 => "Biome4",
            other => STORE_NAMES[store_slot(other)],
        }
    }

    /// Which shop slot this region is, if it is one.
    pub fn store_slot(self) -> Option<usize> {
        matches!(self.index(), 3..=8 | 17..=19 | 21..=35 | 37..=52).then(|| store_slot(self) + 1)
    }
}

/// Maps a store region to its zero-based position in [`STORE_NAMES`].
///
/// The store numbers are not contiguous in the enum, because Vault through Hallway_3 and several
/// others were inserted between them over the years, so the mapping is a lookup rather than arithmetic.
fn store_slot(region: Region) -> usize {
    STORE_ORDINALS
        .iter()
        .position(|&ordinal| ordinal == region.index())
        .unwrap_or(0)
}

const STORE_ORDINALS: [u8; 40] = [
    3, 4, 5, 6, 7, 8, 17, 18, 19, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 37,
    38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52,
];

const STORE_NAMES: [&str; 40] = [
    "Store1", "Store2", "Store3", "Store4", "Store5", "Store6", "Store7", "Store8", "Store9",
    "Store10", "Store11", "Store12", "Store13", "Store14", "Store15", "Store16", "Store17",
    "Store18", "Store19", "Store20", "Store21", "Store22", "Store23", "Store24", "Store25",
    "Store26", "Store27", "Store28", "Store29", "Store30", "Store31", "Store32", "Store33",
    "Store34", "Store35", "Store36", "Store37", "Store38", "Store39", "Store40",
];

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The terrain band a square sits in, which decides what may spawn there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum Terrain {
    #[default]
    None = 0,
    Mountains = 1,
    HighSand = 2,
    HighPlains = 3,
    HighForest = 4,
    MidSand = 5,
    MidPlains = 6,
    MidForest = 7,
    LowSand = 8,
    LowPlains = 9,
    LowForest = 10,
    ShoreSand = 11,
    ShorePlains = 12,
    BeachTowels = 13,
}

pub const TERRAIN_COUNT: usize = 14;

impl Terrain {
    pub fn from_index(index: u8) -> Option<Terrain> {
        if (index as usize) >= TERRAIN_COUNT {
            return None;
        }
        Some(unsafe { std::mem::transmute::<u8, Terrain>(index) })
    }

    pub fn index(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_round_trip_through_their_ordinals() {
        for index in 0..REGION_COUNT as u8 {
            let region = Region::from_index(index).expect("every ordinal should resolve");
            assert_eq!(region.index(), index);
        }
        assert_eq!(Region::from_index(REGION_COUNT as u8), None);
        assert_eq!(Region::from_index(255), None);
    }

    #[test]
    fn the_ordinals_match_the_format_on_disk() {
        // Spot checks against the enum the maps were written with. These are a file format, so a
        // change here silently reinterprets every map.
        assert_eq!(Region::None.index(), 0);
        assert_eq!(Region::Spawn.index(), 1);
        assert_eq!(Region::Vault.index(), 9);
        assert_eq!(Region::Enemy.index(), 13);
        assert_eq!(Region::GiftingChest.index(), 20);
        assert_eq!(Region::ItemSpawnPoint.index(), 36);
        assert_eq!(Region::Biome4.index(), 56);
    }

    #[test]
    fn names_parse_in_the_spellings_the_maps_use() {
        assert_eq!(
            Region::from_name("Realm_Portals"),
            Some(Region::RealmPortals)
        );
        assert_eq!(
            Region::from_name("Realm Portals"),
            Some(Region::RealmPortals)
        );
        assert_eq!(Region::from_name("Store_1"), Some(Region::Store1));
        assert_eq!(
            Region::from_name("Gifting_Chest"),
            Some(Region::GiftingChest)
        );
        assert_eq!(Region::from_name("Vault"), Some(Region::Vault));
        assert_eq!(Region::from_name("nonsense"), None);
    }

    #[test]
    fn every_region_has_a_distinct_name() {
        let mut names: Vec<&str> = (0..REGION_COUNT as u8)
            .filter_map(Region::from_index)
            .map(Region::name)
            .collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        assert_eq!(names.len(), total, "two regions share a name");
    }

    #[test]
    fn store_regions_report_their_slot_and_others_do_not() {
        assert_eq!(Region::Store1.store_slot(), Some(1));
        assert_eq!(Region::Store6.store_slot(), Some(6));
        assert_eq!(Region::Store7.store_slot(), Some(7));
        assert_eq!(Region::Store40.store_slot(), Some(40));

        assert_eq!(Region::Vault.store_slot(), None);
        assert_eq!(Region::None.store_slot(), None);
        assert_eq!(Region::ItemSpawnPoint.store_slot(), None);
    }

    #[test]
    fn terrain_bands_round_trip() {
        for index in 0..TERRAIN_COUNT as u8 {
            assert_eq!(Terrain::from_index(index).unwrap().index(), index);
        }
        assert_eq!(Terrain::from_index(TERRAIN_COUNT as u8), None);
        assert_eq!(Terrain::Mountains.index(), 1);
        assert_eq!(Terrain::BeachTowels.index(), 13);
    }
}

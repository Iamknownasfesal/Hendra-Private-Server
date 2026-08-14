//! The loaded content, indexed for the shapes the simulation actually queries.
//!
//! Loading happens once at boot and the result is immutable and shared. Everything the tick loop
//! touches is a dense index lookup: object types index a `Vec` directly, and names are resolved to
//! types at load so no comparison in the simulation is ever a string comparison.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::desc::{ObjectDesc, ObjectType, TileDesc, TileType};
use crate::player::PlayerDesc;
use crate::xml::{Node, XmlError};

/// Everything loaded from the content files.
#[derive(Debug, Default)]
pub struct Catalog {
    /// Indexed by object type. Sparse over the type space, so most entries are `None`; the space
    /// costs a pointer-width word per unused type and buys a branchless lookup.
    objects: Vec<Option<ObjectDesc>>,

    /// Indexed by tile type.
    tiles: Vec<Option<TileDesc>>,

    by_id: HashMap<String, ObjectType>,
    tiles_by_id: HashMap<String, TileType>,

    /// Object types that can be held in a slot, for the loot and vault paths.
    items: Vec<ObjectType>,

    /// The playable classes, in the order the files list them, which is the order a character
    /// select screen shows them in.
    classes: Vec<PlayerDesc>,

    /// Sets of equipment that give something extra when all of them are worn. What they give is on
    /// none of the pieces, so an item read alone can never say what wearing it with three others is
    /// worth.
    equipment_sets: Vec<crate::EquipmentSet>,

    /// Files that were read, in load order. Kept for diagnostics and for the bake step's staleness
    /// check.
    sources: Vec<PathBuf>,
}

/// What went wrong, per file, without aborting the load.
///
/// A malformed file should cost you that file, not the server's ability to boot. The old server
/// refused to start over one unclosed `<Region>` tag.
#[derive(Debug)]
pub struct LoadReport {
    pub files_read: usize,
    pub objects: usize,
    pub tiles: usize,
    pub items: usize,
    pub classes: usize,
    pub problems: Vec<LoadProblem>,
}

#[derive(Debug)]
pub enum LoadProblem {
    File(XmlError),

    /// Two files declare the same type. The first one loaded wins.
    DuplicateObject {
        object_type: ObjectType,
        kept: String,
        dropped: String,
    },

    /// The type space is full, which needs sixty thousand pieces of unnumbered content.
    NoSpaceLeft {
        id: String,
    },

    /// Two pieces of content wanted the same number, so one was moved.
    ///
    /// Harmless on its own, and worth reporting because a probed number depends on what else is
    /// loaded: adding content can move it again. Anything durable must refer to the identity.
    NumberProbed {
        id: String,
        wanted: u16,
        assigned: u16,
    },

    DuplicateTile {
        tile_type: TileType,
        kept: String,
        dropped: String,
    },

    /// A projectile names an object that no file declares.
    UnknownProjectileObject {
        owner: String,
        object_id: String,
    },
}

impl std::fmt::Display for LoadProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadProblem::File(err) => write!(f, "{err}"),
            LoadProblem::NoSpaceLeft { id } => {
                write!(f, "no runtime number left to give {id}")
            }
            LoadProblem::NumberProbed {
                id,
                wanted,
                assigned,
            } => write!(
                f,
                "{id} wanted 0x{wanted:x} and was given 0x{assigned:x}; \
                 a probed number moves when other content is added"
            ),
            LoadProblem::DuplicateObject {
                object_type,
                kept,
                dropped,
            } => write!(
                f,
                "object type 0x{:x} declared twice: kept {kept}, dropped {dropped}",
                object_type.0
            ),
            LoadProblem::DuplicateTile {
                tile_type,
                kept,
                dropped,
            } => write!(
                f,
                "tile type 0x{:x} declared twice: kept {kept}, dropped {dropped}",
                tile_type.0
            ),
            LoadProblem::UnknownProjectileObject { owner, object_id } => {
                write!(f, "{owner} fires unknown projectile object {object_id:?}")
            }
        }
    }
}

impl Catalog {
    /// Loads every `.xml` and `.dat` file in a directory.
    ///
    /// Files are read in sorted order so a duplicate type resolves the same way on every machine.
    pub fn load_dir(dir: &Path) -> Result<(Catalog, LoadReport), std::io::Error> {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("xml") | Some("dat")
                )
            })
            .collect();
        paths.sort();

        Ok(Catalog::load_files(&paths))
    }

    /// Loads content already in memory.
    ///
    /// Useful wherever the source is not a file on disk: a baked artifact, content embedded in the
    /// binary, or a test that wants a catalog without touching the filesystem, which also removes
    /// the temptation to have several tests share a fixture file and race each other over it.
    pub fn load_str(sources: &[&str]) -> (Catalog, LoadReport) {
        let mut catalog = Catalog::default();
        let mut problems = Vec::new();

        for text in sources {
            match Node::parse(text) {
                Ok(root) => catalog.absorb(&root, &mut problems),
                Err(source) => problems.push(LoadProblem::File(XmlError::Parse {
                    path: "<memory>".into(),
                    source,
                })),
            }
        }

        catalog.resolve(&mut problems);

        let report = LoadReport {
            files_read: sources.len(),
            objects: catalog.object_count(),
            tiles: catalog.tile_count(),
            items: catalog.items.len(),
            classes: catalog.classes.len(),
            problems,
        };

        (catalog, report)
    }

    /// Loads a specific list of files.
    pub fn load_files(paths: &[PathBuf]) -> (Catalog, LoadReport) {
        let mut catalog = Catalog::default();
        let mut problems = Vec::new();
        let mut files_read = 0;

        for path in paths {
            match Node::parse_file(path) {
                Ok(root) => {
                    files_read += 1;
                    catalog.sources.push(path.clone());
                    catalog.absorb(&root, &mut problems);
                }
                Err(err) => problems.push(LoadProblem::File(err)),
            }
        }

        catalog.resolve(&mut problems);

        let report = LoadReport {
            files_read,
            objects: catalog.object_count(),
            tiles: catalog.tile_count(),
            items: catalog.items.len(),
            classes: catalog.classes.len(),
            problems,
        };

        (catalog, report)
    }

    /// Takes every `<Object>` and `<Ground>` anywhere in a parsed file.
    ///
    /// The files disagree on their root element. `<Objects>`, `<GroundTypes>` and several with
    /// both nested under one root all occur, so this walks rather than assuming a shape.
    fn absorb(&mut self, node: &Node, problems: &mut Vec<LoadProblem>) {
        match node.name.as_str() {
            "Object" => {
                if let Some(desc) = ObjectDesc::parse(node) {
                    // Parsed from the same element, because a class needs fields no other object
                    // has and re-finding the element later would mean keeping it around.
                    let class = PlayerDesc::parse(node, desc.object_type);
                    self.insert_object(desc, problems);
                    if let Some(class) = class
                        && !self
                            .classes
                            .iter()
                            .any(|c| c.object_type == class.object_type)
                    {
                        self.classes.push(class);
                    }
                }
                return;
            }
            "Ground" => {
                if let Some(tile) = TileDesc::parse(node) {
                    self.insert_tile(tile, problems);
                }
                return;
            }
            "EquipmentSet" => {
                // Read here rather than in a pass of its own, because a set arrives in the same
                // stream of files as everything else and there is nothing to gain from a second
                // walk of it.
                if let Some(set) = crate::EquipmentSet::parse(node)
                    && !self
                        .equipment_sets
                        .iter()
                        .any(|held| held.set_type == set.set_type)
                {
                    self.equipment_sets.push(set);
                }
                return;
            }
            _ => {}
        }

        for child in &node.children {
            self.absorb(child, problems);
        }
    }

    fn insert_object(&mut self, mut desc: ObjectDesc, problems: &mut Vec<LoadProblem>) {
        // Content that did not number itself is numbered here, from its identity rather than from a
        // counter, so the same object gets the same number whatever order the files are read in.
        if !desc.object_type.is_assigned() {
            let wanted = crate::identity::preferred_slot(desc.uuid);
            match self.free_slot(wanted) {
                Some(assigned) => {
                    // A number reached by probing is not a function of this object alone: adding
                    // other content can move it. Anything durable that refers to it by number
                    // rather than by identity would break, so the load says so.
                    if assigned != wanted {
                        problems.push(LoadProblem::NumberProbed {
                            id: desc.id.clone(),
                            wanted,
                            assigned,
                        });
                    }
                    desc.object_type = ObjectType(assigned);
                }
                None => {
                    problems.push(LoadProblem::NoSpaceLeft {
                        id: desc.id.clone(),
                    });
                    return;
                }
            }
        }

        let index = desc.object_type.0 as usize;
        if self.objects.len() <= index {
            self.objects.resize_with(index + 1, || None);
        }

        if let Some(existing) = &self.objects[index] {
            problems.push(LoadProblem::DuplicateObject {
                object_type: desc.object_type,
                kept: existing.id.clone(),
                dropped: desc.id.clone(),
            });
            return;
        }

        self.by_id.insert(desc.id.clone(), desc.object_type);
        if desc.is_item() {
            self.items.push(desc.object_type);
        }
        self.objects[index] = Some(desc);
    }

    /// The same, for ground.
    fn free_tile_slot(&self, wanted: u16) -> Option<u16> {
        let span = u16::MAX as u32 - crate::identity::FIRST_ASSIGNED as u32;

        for step in 0..=span {
            let candidate = crate::identity::FIRST_ASSIGNED
                + ((wanted - crate::identity::FIRST_ASSIGNED) as u32 + step) as u16
                    % (span as u16 + 1);

            let taken = self
                .tiles
                .get(candidate as usize)
                .is_some_and(Option::is_some);
            if !taken && TileType(candidate).is_assigned() {
                return Some(candidate);
            }
        }

        None
    }

    /// The first free number at or after `wanted`.
    ///
    /// Linear probing rather than a counter: the preferred slot is already spread across the range
    /// by the identity hash, so a probe almost never runs, and when it does it keeps the numbering
    /// a function of the content rather than of the load order.
    fn free_slot(&self, wanted: u16) -> Option<u16> {
        let span = u16::MAX as u32 - crate::identity::FIRST_ASSIGNED as u32;

        for step in 0..=span {
            let candidate = crate::identity::FIRST_ASSIGNED
                + ((wanted - crate::identity::FIRST_ASSIGNED) as u32 + step) as u16
                    % (span as u16 + 1);

            let taken = self
                .objects
                .get(candidate as usize)
                .is_some_and(Option::is_some);

            if !taken && ObjectType(candidate).is_assigned() && !ObjectType(candidate).is_none() {
                return Some(candidate);
            }
        }

        None
    }

    /// The runtime number an identity was given, if the catalog holds it.
    ///
    /// A scan rather than a map: this is asked at load and on a character screen, never per tick,
    /// and a second index would have to be kept in step for no gain.
    pub fn type_of_uuid(&self, uuid: uuid::Uuid) -> Option<ObjectType> {
        self.objects
            .iter()
            .flatten()
            .find(|desc| desc.uuid == uuid)
            .map(|desc| desc.object_type)
    }

    /// Every playable class, in the order the files list them.
    pub fn classes(&self) -> &[PlayerDesc] {
        &self.classes
    }

    /// One class by its object type, or `None` if that type is not playable.
    pub fn class(&self, object_type: ObjectType) -> Option<&PlayerDesc> {
        self.classes
            .iter()
            .find(|class| class.object_type == object_type)
    }

    /// The lowest-tier item that fits a slot, which is what a class starts holding.
    ///
    /// Starting equipment is not written down anywhere: every class in the files lists its
    /// `Equipment` as empty, so a character built from the files alone would arrive with nothing to
    /// shoot. Tier zero of the slot the class's weapon goes in is what the game has always given
    /// out, and deriving it means a new class needs no new configuration.
    ///
    /// Untiered items, meaning everything unique or special, are skipped, since "no tier" sorts as
    /// nothing rather than as the bottom.
    pub fn lowest_tier_for_slot(&self, slot_type: i32) -> Option<ObjectType> {
        self.items
            .iter()
            .filter_map(|object_type| {
                let desc = self.object(*object_type)?;
                let item = desc.item.as_ref()?;
                if item.slot_type != slot_type || item.soulbound {
                    return None;
                }
                Some((item.tier?, *object_type))
            })
            .min_by_key(|(tier, object_type)| (*tier, object_type.0))
            .map(|(_, object_type)| object_type)
    }

    fn insert_tile(&mut self, mut tile: TileDesc, problems: &mut Vec<LoadProblem>) {
        if !tile.tile_type.is_assigned() {
            match self.free_tile_slot(crate::identity::preferred_slot(tile.uuid)) {
                Some(assigned) => tile.tile_type = TileType(assigned),
                None => {
                    problems.push(LoadProblem::NoSpaceLeft {
                        id: tile.id.clone(),
                    });
                    return;
                }
            }
        }

        let index = tile.tile_type.0 as usize;
        if self.tiles.len() <= index {
            self.tiles.resize_with(index + 1, || None);
        }

        if let Some(existing) = &self.tiles[index] {
            problems.push(LoadProblem::DuplicateTile {
                tile_type: tile.tile_type,
                kept: existing.id.clone(),
                dropped: tile.id.clone(),
            });
            return;
        }

        self.tiles_by_id.insert(tile.id.clone(), tile.tile_type);
        self.tiles[index] = Some(tile);
    }

    /// Turns the names projectiles carry into types, now that every file has been read.
    ///
    /// Projectiles reference objects across file boundaries, so this cannot happen during parsing.
    /// Doing it once here is what lets the simulation spawn a bullet without a hash lookup.
    fn resolve(&mut self, problems: &mut Vec<LoadProblem>) {
        let by_id = std::mem::take(&mut self.by_id);

        for slot in self.objects.iter_mut() {
            let Some(desc) = slot else { continue };
            for shot in desc.projectiles.iter_mut() {
                match by_id.get(&shot.object_id) {
                    Some(&object_type) => shot.object_type = object_type,
                    None => problems.push(LoadProblem::UnknownProjectileObject {
                        owner: desc.id.clone(),
                        object_id: shot.object_id.clone(),
                    }),
                }
            }
        }

        self.by_id = by_id;
    }

    // -- lookups ------------------------------------------------------------------------------

    /// The descriptor for a type, or `None` if nothing declares it.
    #[inline]
    pub fn object(&self, object_type: ObjectType) -> Option<&ObjectDesc> {
        self.objects.get(object_type.0 as usize)?.as_ref()
    }

    #[inline]
    /// Every set of equipment the content describes.
    pub fn equipment_sets(&self) -> &[crate::EquipmentSet] {
        &self.equipment_sets
    }

    /// Which sets somebody wearing these things has completed.
    ///
    /// `worn` answers what is in a slot, or `None` for an empty one. A set gives nothing for three
    /// of its four pieces, which is the whole shape of it.
    pub fn sets_worn(&self, worn: &dyn Fn(u16) -> Option<ObjectType>) -> Vec<&crate::EquipmentSet> {
        self.equipment_sets
            .iter()
            .filter(|set| set.worn_by(worn))
            .collect()
    }

    pub fn tile(&self, tile_type: TileType) -> Option<&TileDesc> {
        self.tiles.get(tile_type.0 as usize)?.as_ref()
    }

    /// Resolves a content name to its type. Load-time and tooling only; never call this per tick.
    pub fn type_of(&self, id: &str) -> Option<ObjectType> {
        self.by_id.get(id).copied()
    }

    pub fn tile_type_of(&self, id: &str) -> Option<TileType> {
        self.tiles_by_id.get(id).copied()
    }

    pub fn by_name(&self, id: &str) -> Option<&ObjectDesc> {
        self.object(self.type_of(id)?)
    }

    pub fn object_count(&self) -> usize {
        self.objects.iter().filter(|slot| slot.is_some()).count()
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.iter().filter(|slot| slot.is_some()).count()
    }

    pub fn objects(&self) -> impl Iterator<Item = &ObjectDesc> {
        self.objects.iter().filter_map(Option::as_ref)
    }

    pub fn tiles(&self) -> impl Iterator<Item = &TileDesc> {
        self.tiles.iter().filter_map(Option::as_ref)
    }

    /// Every object that can be held in a slot.
    pub fn items(&self) -> impl Iterator<Item = &ObjectDesc> {
        self.items
            .iter()
            .filter_map(|&object_type| self.object(object_type))
    }

    pub fn enemies(&self) -> impl Iterator<Item = &ObjectDesc> {
        self.objects().filter(|desc| desc.enemy)
    }

    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn content_without_a_written_number_is_numbered_at_load() {
        // The point of the change: an author writes a name, not a free hex value found by reading
        // every other file first.
        let (catalog, report) = Catalog::load_str(&[r#"<Objects>
            <Object id="Wand of Dawn"><Class>Equipment</Class><Item/><SlotType>8</SlotType></Object>
            <Object id="Robe of Dusk"><Class>Equipment</Class><Item/><SlotType>6</SlotType></Object>
            <Ground id="Warm Grass"><Speed>1</Speed></Ground>
        </Objects>"#]);

        assert!(report.problems.is_empty(), "{:?}", report.problems);

        let wand = catalog.type_of("Wand of Dawn").expect("numbered");
        let robe = catalog.type_of("Robe of Dusk").expect("numbered");

        assert_ne!(wand, robe);
        assert!(wand.is_assigned() && !wand.is_none());
        assert!(catalog.tile_type_of("Warm Grass").is_some());
    }

    #[test]
    fn an_assigned_number_is_the_same_on_every_run() {
        // A number that moved between runs would orphan every saved inventory that referred to it.
        let source = r#"<Objects>
            <Object id="Wand of Dawn"><Class>Equipment</Class><Item/></Object>
        </Objects>"#;

        let first = Catalog::load_str(&[source]).0.type_of("Wand of Dawn");
        let second = Catalog::load_str(&[source]).0.type_of("Wand of Dawn");

        assert_eq!(first, second);
    }

    #[test]
    fn an_assigned_number_does_not_depend_on_the_order_files_are_read() {
        let wand =
            r#"<Objects><Object id="Wand"><Class>Equipment</Class><Item/></Object></Objects>"#;
        let robe =
            r#"<Objects><Object id="Robe"><Class>Equipment</Class><Item/></Object></Objects>"#;

        let forwards = Catalog::load_str(&[wand, robe]).0;
        let backwards = Catalog::load_str(&[robe, wand]).0;

        assert_eq!(forwards.type_of("Wand"), backwards.type_of("Wand"));
        assert_eq!(forwards.type_of("Robe"), backwards.type_of("Robe"));
    }

    #[test]
    fn an_assigned_number_never_lands_on_one_the_legacy_files_use() {
        let (catalog, _) = Catalog::load_str(&[r#"<Objects>
            <Object type="0x0a00" id="Old Wand"><Class>Equipment</Class><Item/></Object>
            <Object id="New Wand"><Class>Equipment</Class><Item/></Object>
        </Objects>"#]);

        assert_eq!(catalog.type_of("Old Wand"), Some(ObjectType(0x0a00)));
        assert!(catalog.type_of("New Wand").unwrap().0 >= crate::identity::FIRST_ASSIGNED);
    }

    #[test]
    fn an_identity_written_by_hand_survives_a_rename() {
        // What the UUID is for: the number follows the identity, so renaming an item leaves every
        // saved inventory pointing at the same thing.
        let before = Catalog::load_str(&[r#"<Objects>
            <Object uuid="0f8fad5b-d9cb-469f-a165-70867728950e" id="Wand of Dawn">
              <Class>Equipment</Class><Item/></Object>
        </Objects>"#])
        .0;

        let after = Catalog::load_str(&[r#"<Objects>
            <Object uuid="0f8fad5b-d9cb-469f-a165-70867728950e" id="Wand of Morning">
              <Class>Equipment</Class><Item/></Object>
        </Objects>"#])
        .0;

        assert_eq!(
            before.type_of("Wand of Dawn"),
            after.type_of("Wand of Morning")
        );
    }

    #[test]
    fn two_objects_wanting_the_same_number_both_get_one() {
        // Probing has to work, or the second of a colliding pair would silently vanish.
        let mut source = String::from("<Objects>");
        for n in 0..500 {
            source.push_str(&format!(
                r#"<Object id="Item {n}"><Class>Equipment</Class><Item/></Object>"#
            ));
        }
        source.push_str("</Objects>");

        let (catalog, report) = Catalog::load_str(&[&source]);

        let numbers: std::collections::HashSet<_> = (0..500)
            .filter_map(|n| catalog.type_of(&format!("Item {n}")))
            .collect();
        assert_eq!(numbers.len(), 500, "every one got a distinct number");

        // Five hundred names in sixty thousand slots collide a handful of times, and each one is
        // reported: a probed number depends on what else is loaded, so nothing durable may use it.
        let probed = report
            .problems
            .iter()
            .filter(|problem| matches!(problem, LoadProblem::NumberProbed { .. }))
            .count();
        assert_eq!(probed, report.problems.len(), "{:?}", report.problems);
        assert!(probed < 50, "{probed} collisions is more than chance");
    }

    use super::*;

    fn catalog_from(files: &[&str]) -> (Catalog, LoadReport) {
        let mut catalog = Catalog::default();
        let mut problems = Vec::new();
        for text in files {
            let root = Node::parse(text).unwrap();
            catalog.absorb(&root, &mut problems);
        }
        catalog.resolve(&mut problems);
        let report = LoadReport {
            files_read: files.len(),
            objects: catalog.object_count(),
            tiles: catalog.tile_count(),
            items: catalog.items.len(),
            classes: catalog.classes.len(),
            problems,
        };
        (catalog, report)
    }

    #[test]
    fn objects_and_tiles_are_found_under_any_root() {
        let (catalog, report) = catalog_from(&[
            r#"<Objects>
                 <Object type="0x01" id="Slime"><Class>Character</Class><Enemy/></Object>
               </Objects>"#,
            r#"<GroundTypes>
                 <Ground type="0x00" id="Black Water"><NoWalk/></Ground>
               </GroundTypes>"#,
        ]);

        assert!(report.problems.is_empty());
        assert_eq!(catalog.object_count(), 1);
        assert_eq!(catalog.tile_count(), 1);
        assert_eq!(catalog.by_name("Slime").unwrap().id, "Slime");
        assert!(catalog.tile(TileType(0)).unwrap().no_walk);
    }

    #[test]
    fn projectile_names_resolve_to_types_across_files() {
        let (catalog, report) = catalog_from(&[
            r#"<Objects>
                 <Object type="0xb0b" id="Sword">
                   <Class>Equipment</Class><Item/><SlotType>1</SlotType>
                   <Projectile><ObjectId>Purple Bolt</ObjectId><Speed>100</Speed>
                     <MinDamage>1</MinDamage><MaxDamage>2</MaxDamage></Projectile>
                 </Object>
               </Objects>"#,
            r#"<Objects>
                 <Object type="0x999" id="Purple Bolt"><Class>Projectile</Class></Object>
               </Objects>"#,
        ]);

        assert!(report.problems.is_empty());
        let sword = catalog.by_name("Sword").unwrap();
        assert_eq!(sword.projectiles[0].object_type, ObjectType(0x999));
    }

    #[test]
    fn a_dangling_projectile_reference_is_reported_not_fatal() {
        let (catalog, report) = catalog_from(&[r#"<Objects>
             <Object type="0xb0b" id="Sword">
               <Class>Equipment</Class><Item/><SlotType>1</SlotType>
               <Projectile><ObjectId>Nothing</ObjectId><Speed>100</Speed>
                 <MinDamage>1</MinDamage><MaxDamage>2</MaxDamage></Projectile>
             </Object>
           </Objects>"#]);

        assert_eq!(catalog.object_count(), 1);
        assert!(matches!(
            report.problems.as_slice(),
            [LoadProblem::UnknownProjectileObject { .. }]
        ));
    }

    #[test]
    fn a_duplicate_type_keeps_the_first_and_reports_the_second() {
        let (catalog, report) = catalog_from(&[
            r#"<Objects><Object type="0x01" id="First"><Class>Character</Class></Object></Objects>"#,
            r#"<Objects><Object type="0x01" id="Second"><Class>Character</Class></Object></Objects>"#,
        ]);

        assert_eq!(catalog.object(ObjectType(1)).unwrap().id, "First");
        assert!(matches!(
            report.problems.as_slice(),
            [LoadProblem::DuplicateObject { .. }]
        ));
    }

    #[test]
    fn items_are_indexed_separately_from_everything_else() {
        let (catalog, _) = catalog_from(&[r#"<Objects>
             <Object type="0x01" id="Slime"><Class>Character</Class><Enemy/></Object>
             <Object type="0x02" id="Potion"><Class>Equipment</Class><Item/><SlotType>0</SlotType></Object>
           </Objects>"#]);

        let items: Vec<&str> = catalog.items().map(|desc| desc.id.as_str()).collect();
        assert_eq!(items, vec!["Potion"]);

        let enemies: Vec<&str> = catalog.enemies().map(|desc| desc.id.as_str()).collect();
        assert_eq!(enemies, vec!["Slime"]);
    }
}

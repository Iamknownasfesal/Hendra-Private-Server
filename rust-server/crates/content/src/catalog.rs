//! The loaded content, indexed for the shapes the simulation actually queries.
//!
//! Loading happens once at boot and the result is immutable and shared. Everything the tick loop
//! touches is a dense index lookup: object types index a `Vec` directly, and names are resolved to
//! types at load so no comparison in the simulation is ever a string comparison.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::desc::{ObjectDesc, ObjectType, TileDesc, TileType};
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

    /// Files that were read, in load order. Kept for diagnostics and for the bake step's staleness
    /// check.
    sources: Vec<PathBuf>,
}

/// What went wrong, per file, without aborting the load.
///
/// A malformed file should cost you that file, not the server's ability to boot — the old server
/// refused to start over one unclosed `<Region>` tag, which is exactly the failure mode to avoid.
#[derive(Debug)]
pub struct LoadReport {
    pub files_read: usize,
    pub objects: usize,
    pub tiles: usize,
    pub items: usize,
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

    DuplicateTile {
        tile_type: TileType,
        kept: String,
        dropped: String,
    },

    /// A projectile names an object that no file declares.
    UnknownProjectileObject { owner: String, object_id: String },
}

impl std::fmt::Display for LoadProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadProblem::File(err) => write!(f, "{err}"),
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
    /// binary, or a test that wants a catalog without touching the filesystem — which also removes
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
            problems,
        };

        (catalog, report)
    }

    /// Takes every `<Object>` and `<Ground>` anywhere in a parsed file.
    ///
    /// The files disagree on their root element — `<Objects>`, `<GroundTypes>`, and several with
    /// both nested under one root — so this walks rather than assuming a shape.
    fn absorb(&mut self, node: &Node, problems: &mut Vec<LoadProblem>) {
        match node.name.as_str() {
            "Object" => {
                if let Some(desc) = ObjectDesc::parse(node) {
                    self.insert_object(desc, problems);
                }
                return;
            }
            "Ground" => {
                if let Some(tile) = TileDesc::parse(node) {
                    self.insert_tile(tile, problems);
                }
                return;
            }
            _ => {}
        }

        for child in &node.children {
            self.absorb(child, problems);
        }
    }

    fn insert_object(&mut self, desc: ObjectDesc, problems: &mut Vec<LoadProblem>) {
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

    fn insert_tile(&mut self, tile: TileDesc, problems: &mut Vec<LoadProblem>) {
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
    pub fn tile(&self, tile_type: TileType) -> Option<&TileDesc> {
        self.tiles.get(tile_type.0 as usize)?.as_ref()
    }

    /// Resolves a content name to its type. Load-time and tooling only — never call this per tick.
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

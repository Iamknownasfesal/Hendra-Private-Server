//! Game content: objects, items, projectiles and tiles.
//!
//! Everything here runs once, at startup, and is immutable afterwards. The simulation only ever
//! sees [`Catalog`], where every lookup is an array index, with no parsing, no hashing and no string
//! comparison happens on a tick.
//!
//! The source files are hand-maintained and irregular, so [`xml`] is built to survive them:
//! presence-only elements, fields spelled as attributes in one file and elements in another,
//! numbers in hex or decimal, and the occasional unclosed tag. A malformed file costs its own
//! contents and nothing else. [`Catalog::load_dir`] collects every problem it hit and hands them
//! back rather than failing, because a server that refuses to boot over one stray tag is worse
//! than a server missing one object.

pub mod catalog;
pub mod desc;
pub mod effect;
pub mod legacy;
pub mod map;
pub mod player;
pub mod region;
pub mod world;
pub mod xml;

pub use catalog::{Catalog, LoadProblem, LoadReport};
pub use desc::{
    ActivateDesc, ItemDesc, ObjectDesc, ObjectType, ProjectileDesc, SizeRange, StatBoost, TileDesc,
    TileType,
};
pub use effect::{AppliedEffect, ConditionEffect, ConditionSet};
pub use map::{Composition, Map, MapError};
pub use player::{PlayerDesc, Stat, StatGrowth, Unlock};
pub use region::{Region, Terrain};
pub use world::{WorldDef, WorldError};
pub use xml::{Node, XmlError};

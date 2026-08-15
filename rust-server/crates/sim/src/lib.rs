//! The world simulation.
//!
//! One world is owned by one task and ticked in isolation. Nothing here takes a lock, because
//! nothing here is shared: worlds are independent, so parallelism belongs between them rather than
//! inside them.

pub mod effects;
pub mod fame;
pub mod grid;
pub mod inventory;
pub mod leveling;
pub mod market;
pub mod metrics;
pub mod projectile;
pub mod quest;
pub mod realm;
pub mod setpiece;
pub mod shop;
pub mod sight;
pub mod slab;
pub mod stats;
pub mod tiles;
pub mod world;

pub use grid::Grid;
pub use inventory::{Container, ContainerKind, MoveError, Slot};
pub use metrics::TickMetrics;
pub use projectile::{Hit, Projectile, Projectiles};
pub use sight::{Sight, SightCircle};
pub use slab::{Handle, MAX_ENTITIES, Slab};
pub use tiles::{Terrain, Walker};
pub use world::{Entity, Kind, MoveOutcome, MoveRefusal, SIGHT_RADIUS, World};

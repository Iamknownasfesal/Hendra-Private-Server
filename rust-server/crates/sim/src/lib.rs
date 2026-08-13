//! The world simulation.
//!
//! One world is owned by one task and ticked in isolation. Nothing here takes a lock, because
//! nothing here is shared: worlds are independent, so parallelism belongs between them rather than
//! inside them.

pub mod effects;
pub mod grid;
pub mod inventory;
pub mod metrics;
pub mod projectile;
pub mod slab;
pub mod tiles;
pub mod world;

pub use grid::Grid;
pub use inventory::{Container, ContainerKind, MoveError, Slot};
pub use metrics::TickMetrics;
pub use projectile::{Hit, Projectile, Projectiles};
pub use slab::{Handle, MAX_ENTITIES, Slab};
pub use tiles::Terrain;
pub use world::{Entity, Kind, MoveOutcome, MoveRefusal, SIGHT_RADIUS, World};

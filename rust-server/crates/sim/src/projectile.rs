//! Projectiles, and who they hit.
//!
//! # The client does not report hits
//!
//! The legacy server let clients say "my bullet 47 hit that enemy" and then tried to decide whether
//! to believe them. That check is what disconnected the owner of this server from his own game, and
//! it could not have worked: bullet ids were a byte that wrapped every 256 shots, so a report could
//! name a bullet that had expired and resolve against whichever one had taken its slot. Nothing
//! could tell the difference.
//!
//! So there is no hit report. The server advances every projectile itself and decides what it
//! struck. That removes the class of bug rather than tightening the check, and it makes the
//! question "is this client lying?" not arise. A client that claims anything about a hit is simply
//! not asked.
//!
//! The cost is honest: a client draws its shot connecting slightly before the server agrees, and
//! very occasionally the server disagrees. Every authoritative shooter has that trade, and it is
//! preferable to trusting the one participant with a motive to lie.
//!
//! # Movement
//!
//! Projectiles step by whole ticks and test the segment they crossed, not just where they landed.
//! At 20 ticks per second a fast bullet covers well over a tile per tick, so testing only the
//! endpoint would let it pass straight through anything thinner than its step.

use hendra_content::{AppliedEffect, Catalog, ObjectType, ProjectileDesc};

use crate::grid::Grid;
use crate::slab::{Handle, Slab};
use crate::tiles::Terrain;
use crate::world::{Entity, Kind};

/// How many segments a projectile's path is divided into per tick when testing for hits.
///
/// Four is enough that the fastest projectile in the content, around 1.6 tiles per tick, tests at
/// least once every 0.4 tiles, comfortably inside any hitbox.
const PATH_STEPS: u32 = 4;

/// How close a projectile must come to count as a hit, in tiles.
///
/// A single figure rather than per-entity hitboxes: the game's own collision was this coarse, and
/// making it finer would change how the game plays rather than how correct it is.
pub const HIT_RADIUS: f32 = 0.5;

/// A projectile in flight.
#[derive(Debug, Clone)]
pub struct Projectile {
    /// Who fired it. A projectile outlives its owner, so this handle may stop resolving.
    pub owner: Handle,

    /// Whether it was fired by a player, which decides what it can hit.
    pub from_player: bool,

    pub object_type: ObjectType,

    /// What the bullet's descriptor was read off: the weapon, the ability item, or the enemy.
    ///
    /// A projectile's speed, damage, lifetime and texture all live in a `<Projectile>` element on
    /// something else, and the bullet's own object type names only its sprite. This is the half a
    /// client needs to draw it, and it is what the original puts on the wire in both of its shot
    /// packets: `ContainerType = item.ObjectType` (`Player.UseItem.cs:1135`, `:1160`).
    pub container: ObjectType,

    pub x: f32,
    pub y: f32,

    /// Where it was fired from.
    ///
    /// Kept because a projectile's position is a function of how long it has been alive rather than
    /// of where it was last tick. Anything that does not travel in a straight line is defined that
    /// way in the original, and integrating a curve step by step drifts away from the path the
    /// client draws, which is the path the player is dodging.
    pub start_x: f32,
    pub start_y: f32,

    /// Direction of travel, in radians.
    pub angle: f32,

    /// Which shot of a volley this is.
    ///
    /// Curves read its parity: neighbouring bullets wave to opposite sides, so a volley spreads into
    /// a pattern rather than every bullet tracing the same line.
    pub id: u8,

    /// Travels in a sine wave about three degrees wide.
    pub wavy: bool,

    /// Traces a figure of eight rather than travelling outward at all.
    pub parametric: bool,

    /// Turns round at the halfway point of its life and comes back.
    pub boomerang: bool,

    /// How far a sine wave carries it sideways, in tiles. Zero for a straight shot.
    pub amplitude: f32,

    /// How many full waves it makes over its lifetime.
    pub frequency: f32,

    /// How large a parametric figure is drawn, in tiles.
    pub magnitude: f32,

    /// Tiles per second.
    pub speed: f32,

    pub damage: i32,

    /// How long it has been alive, in milliseconds.
    pub age_ms: u32,
    pub lifetime_ms: u32,

    /// Whether it continues after striking something.
    pub multi_hit: bool,

    /// Whether terrain stops it.
    pub passes_cover: bool,

    /// Whether the target's defence is ignored.
    pub armor_piercing: bool,

    /// Condition effects the shot puts on whatever it strikes, and how long each lasts.
    ///
    /// Carried on the projectile rather than looked up from the descriptor when a hit lands,
    /// because a projectile outlives the tick it was fired on and the hit is applied after the
    /// path has already been walked. Empty for all but the debuff weapons and the bosses that
    /// paralyse, which is why it costs nothing to carry.
    pub effects: Vec<AppliedEffect>,

    /// Entities already struck, so a multi-hit projectile does not hit the same target twice.
    pub struck: Vec<Handle>,

    /// Whether the one who fired it draws it without being told.
    ///
    /// The original splits its two announcements on exactly this. A volley that leaves the player's
    /// own body goes out as `AllyShoot` to everybody `p != this` (`Player.UseItem.cs:1140`), because
    /// the client that pulled the trigger already drew it. A nova starts at the cursor instead, and
    /// its `ServerPlayerShoot` goes to every nearby player with nobody left out (`:1176-1180`) --
    /// the caster included, since a bullet that appears somewhere they are not standing is not
    /// something their client could have known to draw.
    pub predicted_by_owner: bool,
}

impl Projectile {
    /// Builds a projectile from a content descriptor.
    #[allow(clippy::too_many_arguments)]
    pub fn from_desc(
        owner: Handle,
        from_player: bool,
        container: ObjectType,
        desc: &ProjectileDesc,
        x: f32,
        y: f32,
        angle: f32,
        roll: f32,
        id: u8,
    ) -> Projectile {
        Projectile {
            owner,
            from_player,
            object_type: desc.object_type,
            container,
            x,
            y,
            start_x: x,
            start_y: y,
            angle,
            id,
            wavy: desc.wavy,
            parametric: desc.parametric,
            boomerang: desc.boomerang,
            amplitude: desc.amplitude,
            frequency: desc.frequency,
            magnitude: desc.magnitude,
            // Content quotes speed in tiles per 10,000 ms, which is tiles per second times ten.
            speed: desc.speed / 10.0,
            damage: desc.roll_damage(roll),
            age_ms: 0,
            lifetime_ms: desc.lifetime_ms.max(0) as u32,
            multi_hit: desc.multi_hit,
            passes_cover: desc.passes_cover,
            armor_piercing: desc.armor_piercing,
            effects: desc.effects.clone(),
            struck: Vec::new(),
            predicted_by_owner: true,
        }
    }

    pub fn expired(&self) -> bool {
        self.age_ms >= self.lifetime_ms
    }

    /// Where this projectile is after being alive for `age_ms`.
    ///
    /// Follows `Projectile.GetPosition`. A function of age rather than a step from the last
    /// position, because that is how the client draws it: the two must agree or a bullet is dodged
    /// where it is not and hits where it is not drawn.
    pub fn position_at(&self, age_ms: u32) -> (f32, f32) {
        let age = age_ms as f32;
        let travelled = self.speed * (age / 1000.0);

        // Alternate bullets of a volley start half a wave apart, which is what turns a line of them
        // into a braid rather than a single thick line.
        let phase = if self.id.is_multiple_of(2) {
            0.0
        } else {
            std::f32::consts::PI
        };

        if self.wavy {
            // Divided rather than multiplied: a wobble of about three degrees, which is what the
            // client draws and therefore what the player dodges.
            let theta = self.angle
                + (std::f32::consts::PI / 64.0)
                    * (phase + 6.0 * std::f32::consts::PI * (age / 1000.0)).sin();

            return (
                self.start_x + travelled * theta.cos(),
                self.start_y + travelled * theta.sin(),
            );
        }

        if self.parametric {
            let lifetime = self.lifetime_ms.max(1) as f32;
            let theta = age / lifetime * 2.0 * std::f32::consts::PI;

            let a = theta.sin() * if self.id.is_multiple_of(2) { -1.0 } else { 1.0 };
            let b = (theta * 2.0).sin() * if self.id % 4 < 2 { 1.0 } else { -1.0 };
            let (sin, cos) = (self.angle.sin(), self.angle.cos());

            return (
                self.start_x + (a * cos - b * sin) * self.magnitude,
                self.start_y + (a * sin + b * cos) * self.magnitude,
            );
        }

        // A boomerang turns round at the halfway point of its life rather than at a distance, so
        // it comes back to the hand that threw it exactly as it expires.
        let travelled = if self.boomerang {
            let half = (self.lifetime_ms as f32 * self.speed / 1000.0) / 2.0;
            if travelled > half {
                half - (travelled - half)
            } else {
                travelled
            }
        } else {
            travelled
        };

        let (mut x, mut y) = (
            self.start_x + travelled * self.angle.cos(),
            self.start_y + travelled * self.angle.sin(),
        );

        if self.amplitude != 0.0 {
            let lifetime = self.lifetime_ms.max(1) as f32;
            let sideways = self.amplitude
                * (phase + age / lifetime * self.frequency * 2.0 * std::f32::consts::PI).sin();

            let across = self.angle + std::f32::consts::FRAC_PI_2;
            x += sideways * across.cos();
            y += sideways * across.sin();
        }

        (x, y)
    }
}

/// One projectile striking one entity.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub projectile: Handle,
    pub target: Handle,

    /// Who fired it, which is who earned whatever this kill drops.
    pub owner: Handle,

    pub damage: i32,

    /// Whether the target keeps its health anyway.
    ///
    /// `Enemy.HitByProjectile` (`Enemy.cs:104-105`) and `Player.HitByProjectile` (`Player.cs:786`)
    /// both guard only the `HP -=` with Invulnerable and report the blow in full, so the number the
    /// room is shown and the number the body loses are two different things.
    pub absorbed: bool,

    /// Which shot of the volley landed, so the broadcast can name the bullet the client drew.
    pub bullet: u8,

    /// Whether this killed the target, at the threshold that target's kind dies on.
    pub fatal: bool,

    /// What the shot puts on the target besides damage.
    ///
    /// Copied onto the hit rather than read back off the projectile when the hit is applied,
    /// because a projectile that stopped on what it struck is already gone by then. Empty for
    /// every plain shot, so it costs nothing to carry.
    pub effects: Vec<AppliedEffect>,
}

/// How much defence an entity has against a hit.
///
/// The two kinds keep it in different places, and using the wrong one is silent. An enemy's defence
/// is a property of its type and lives in the content. A player's is a stat: it starts at the class
/// base, rises with every level, and is added to by armour, rings and boosts. Reading a player's
/// from the descriptor gets the level-one base, which for most classes is zero — armour, rings and
/// levelling all stop counting and nothing says so.
pub fn defence_of(entity: &Entity, catalog: &Catalog) -> i32 {
    if entity.kind == Kind::Player {
        return entity.stats.defence();
    }

    catalog
        .object(entity.object_type)
        .map(|desc| desc.defense)
        .unwrap_or(0)
}

/// Every projectile in one world.
pub struct Projectiles {
    live: Slab<Projectile>,

    /// What was fired since the last time anybody asked.
    ///
    /// Kept because a projectile is drawn by the client rather than described to it every tick: the
    /// shot is announced once and both sides then step it themselves. Without this list only the
    /// shots a player asked for by name could be announced, and everything an enemy fired would
    /// travel and land while remaining invisible.
    fired: Vec<Handle>,

    /// Reused between ticks so a warm world allocates nothing.
    nearby: Vec<Handle>,
    expired: Vec<Handle>,
}

impl Default for Projectiles {
    fn default() -> Self {
        Projectiles::new()
    }
}

impl Projectiles {
    pub fn new() -> Projectiles {
        Projectiles {
            live: Slab::new(),
            fired: Vec::new(),
            nearby: Vec::new(),
            expired: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    pub fn get(&self, handle: Handle) -> Option<&Projectile> {
        self.live.get(handle)
    }

    pub fn iter(&self) -> impl Iterator<Item = (Handle, &Projectile)> {
        self.live.iter()
    }

    /// Adds a projectile, returning the handle that names it.
    ///
    /// The handle is generational, so a reference to a projectile that has since expired stops
    /// resolving rather than resolving to whatever took its slot.
    pub fn fire(&mut self, projectile: Projectile) -> Option<Handle> {
        let handle = self.live.insert(projectile);
        if let Some(handle) = handle {
            self.fired.push(handle);
        }
        handle
    }

    /// Takes what has been fired since this was last called.
    pub fn take_fired(&mut self) -> Vec<Handle> {
        std::mem::take(&mut self.fired)
    }

    pub fn remove(&mut self, handle: Handle) -> Option<Projectile> {
        self.live.remove(handle)
    }

    /// Advances every projectile and reports what they struck.
    ///
    /// Hits are appended to `hits` rather than applied here, because applying damage needs mutable
    /// access to the entities this is already reading.
    pub fn advance(
        &mut self,
        entities: &Slab<Entity>,
        grid: &Grid,
        terrain: &Terrain,
        catalog: &Catalog,
        elapsed_ms: u32,
        hits: &mut Vec<Hit>,
    ) {
        hits.clear();
        self.expired.clear();

        for (handle, projectile) in self.live.iter_mut() {
            let was = projectile.age_ms;
            projectile.age_ms = projectile.age_ms.saturating_add(elapsed_ms);

            let mut stopped = false;

            // Walked in time rather than along a line, so a shot that curves is tested against the
            // path it actually takes. A straight one comes out the same, since sampling a straight
            // line at even intervals is the same as stepping along it.
            for step in 1..=PATH_STEPS {
                let age = was + (elapsed_ms * step) / PATH_STEPS;
                let (x, y) = projectile.position_at(age);

                if terrain.stops_shot(x, y, projectile.passes_cover) {
                    projectile.x = x;
                    projectile.y = y;
                    stopped = true;
                    break;
                }

                Self::strike_at(
                    handle,
                    projectile,
                    entities,
                    grid,
                    catalog,
                    x,
                    y,
                    hits,
                    &mut self.nearby,
                );

                projectile.x = x;
                projectile.y = y;

                // A spent bullet is only spent for the side it was aimed at. `ForceHit`
                // (`Projectile.cs:472`) refuses a second target on anything that is not a player
                // and applies it on anything that is, so one bullet through a crowd is taken by
                // everybody it passes and one through two monsters is taken by the first. That is
                // what the players see as well: each client tests bullets against itself first
                // (`Projectile.as:307`), so two people standing together each report the same
                // shot.
                if !projectile.multi_hit && projectile.from_player && !projectile.struck.is_empty()
                {
                    stopped = true;
                    break;
                }
            }

            if stopped || projectile.expired() {
                self.expired.push(handle);
            }
        }

        for handle in self.expired.drain(..) {
            self.live.remove(handle);
        }
    }

    /// Tests one point along a projectile's path against nearby entities.
    #[allow(clippy::too_many_arguments)]
    fn strike_at(
        handle: Handle,
        projectile: &mut Projectile,
        entities: &Slab<Entity>,
        grid: &Grid,
        catalog: &Catalog,
        x: f32,
        y: f32,
        hits: &mut Vec<Hit>,
        nearby: &mut Vec<Handle>,
    ) {
        grid.within(x, y, HIT_RADIUS, nearby);
        if nearby.is_empty() {
            return;
        }

        // A hidden player's shots do nothing. `EnemyHitHandler.cs:24` drops the claim before it
        // looks at anything else, and the client will not make one either (`Projectile.as:249`),
        // so an admin watching a fight unseen cannot join it by firing into it.
        if projectile.from_player
            && entities.get(projectile.owner).is_some_and(|shooter| {
                shooter
                    .conditions
                    .contains(hendra_content::ConditionEffect::Hidden)
            })
        {
            return;
        }

        for target in nearby.iter() {
            if projectile.struck.contains(target) {
                continue;
            }

            let Some(entity) = entities.get(*target) else {
                continue;
            };

            // The other half of the same guard: a spent bullet still reaches players and reaches
            // nothing else, however far it has left to fly.
            if !projectile.multi_hit && !projectile.struck.is_empty() && entity.kind != Kind::Player
            {
                continue;
            }

            if !can_hit(projectile, *target, entity, catalog) {
                continue;
            }

            let defence = defence_of(entity, catalog);

            // Armour and curses belong to the target. The shooter's own multipliers were applied
            // when the projectile was made, because its state at the moment of firing is what
            // should decide the shot.
            let rules = crate::effects::Rules::of(entity.conditions);
            let damage = rules.damage_after_defence(
                projectile.damage,
                defence,
                projectile.armor_piercing,
                entity.kind == Kind::Player,
            );

            hits.push(Hit {
                projectile: handle,
                target: *target,
                owner: projectile.owner,
                damage,
                absorbed: rules.no_damage,
                bullet: projectile.id,
                fatal: !rules.no_damage && entity.slain_by(damage),
                effects: projectile.effects.clone(),
            });

            projectile.struck.push(*target);

            // A bullet that is not multi-hit is spent, and being spent only stops it hitting
            // something that is not a player: `ForceHit` (`Projectile.cs:472`) lets a player
            // through that guard, so a crowd standing on one square all take the same shot.
            if !projectile.multi_hit && entity.kind != Kind::Player {
                return;
            }
        }
    }

    /// Removes every projectile fired by an entity that has gone.
    ///
    /// Not strictly required, since an orphaned projectile still flies and still hits, but a world that
    /// keeps firing on a player's behalf after they disconnect is surprising.
    pub fn drop_orphans(&mut self, entities: &Slab<Entity>) {
        self.live
            .retain(|_, projectile| entities.contains(projectile.owner));
    }

    pub fn clear(&mut self) {
        self.live.clear();
    }
}

/// Whether a projectile may strike a particular entity.
fn can_hit(projectile: &Projectile, target: Handle, entity: &Entity, catalog: &Catalog) -> bool {
    if target == projectile.owner {
        return false;
    }

    // `stat = ObjectDesc.MaxHP == 0` (`Enemy.cs:20`), and both `Enemy.Damage` and
    // `Enemy.HitByProjectile` return before anything else when it is set. Of the 1309 objects the
    // shipped content marks `Class=Character`, 181 declare no `MaxHitPoints` at all: spawners,
    // encounter managers, turrets and breakable-looking scenery, every one of which the content
    // expects to be indestructible. Take that away and a player can shoot the machinery that runs
    // a dungeon.
    //
    // The same question is asked of a `StaticObject`, where it is spelled `Vulnerable`
    // (`StaticObject.cs:36`, `:57`), and both are asked alongside the `<Enemy/>` flag the client
    // uses to decide what to hit-test at all — see `world::is_shootable`.
    if matches!(entity.kind, Kind::Enemy | Kind::StaticObject)
        && !catalog
            .object(entity.object_type)
            .is_some_and(crate::world::is_shootable)
    {
        return false;
    }
    // Marked dead means already reaped this tick and simply not there any more. Health is not the
    // test: `Enemy.HitByProjectile` gates on nothing but the effects, and an enemy left on exactly
    // zero is still alive (`Enemy.cs:127` kills on `HP < 0`). Refusing to strike anything at zero
    // makes that enemy permanently unhittable and therefore permanently alive — health that falls
    // to nothing and a body that never dies.
    if entity.dead {
        return false;
    }

    // An untouchable target is not hit at all: the shot passes through and none of the effects it
    // carries land. A target that merely takes no damage is still hit, which is a different thing
    // and is decided later.
    //
    // Invulnerable is on which side of that line depending on what is being shot, because the
    // original wrote the two hit paths separately. `Enemy.HitByProjectile` (`Enemy.cs:99`) checks
    // it only around the `HP -=`, so an invulnerable enemy still takes the shot's effects and
    // still shows the damage number. A player goes through `IsInvulnerable` (`Player.cs:758`),
    // which lists Invulnerable beside Paused, Stasis and Invincible, and `HitByProjectile` returns
    // on it before anything at all — so a bullet passes an invulnerable player in silence.
    let rules = crate::effects::Rules::of(entity.conditions);
    let refuses = rules.untouchable
        || (entity.kind == Kind::Player
            && entity
                .conditions
                .contains(hendra_content::ConditionEffect::Invulnerable));
    if refuses {
        return false;
    }

    // Players shoot enemies and breakable scenery; enemies shoot players and nothing else. Players
    // do not shoot each other.
    //
    // A decoy is on neither list. It is a `StaticObject` in the original (`Decoy.cs:29`) whose
    // health *is* its remaining lifetime, and `StaticObject.HitByProjectile` (`:57`) acts only on a
    // projectile owned by a `Player` — so its owner's shots pass through it and an enemy's shot
    // takes nothing off it. A decoy cannot be destroyed; it can only run out.
    matches!(
        (projectile.from_player, entity.kind),
        (true, Kind::Enemy) | (true, Kind::StaticObject) | (false, Kind::Player)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hendra_content::ConditionSet;

    const FIXTURE: &str = r#"<Objects>
        <Ground type="0x10" id="Grass"><Speed>1</Speed></Ground>
        <Object type="0x501" id="Tree"><Class>GameObject</Class>
          <BlocksSight/><Static/><OccupySquare/></Object>
        <Object type="0x505" id="Fence"><Class>GameObject</Class><Static/><OccupySquare/></Object>
        <Object type="0x506" id="Bollard"><Class>GameObject</Class><Static/><EnemyOccupySquare/></Object>
        <Object type="0x502" id="Slime"><Class>Character</Class><Enemy/>
          <MaxHitPoints>200</MaxHitPoints><Defense>10</Defense></Object>
        <Object type="0x503" id="Armoured"><Class>Character</Class><Enemy/>
          <MaxHitPoints>500</MaxHitPoints><Defense>1000</Defense></Object>
        <Object type="0x504" id="Spawner"><Class>Character</Class><Enemy/></Object>
        <Object type="0x600" id="Hero"><Class>Player</Class><Player/></Object>
      </Objects>"#;

    fn catalog() -> Catalog {
        Catalog::load_str(&[FIXTURE]).0
    }

    fn terrain(catalog: &Catalog) -> Terrain {
        use hendra_content::map::Composition;
        use hendra_content::{Map, Region, TileType};

        let squares = (0..32 * 32).map(|_| Composition {
            tile: TileType(0x10),
            object: ObjectType::NONE,
            region: Region::None,
            terrain: hendra_content::Terrain::None,
            config: String::new(),
        });
        Terrain::build(Map::from_squares(32, 32, squares).unwrap(), catalog)
    }

    fn enemy(object_type: u16, x: f32, y: f32, hp: i32) -> Entity {
        Entity {
            object_type: ObjectType(object_type),
            kind: Kind::Enemy,
            terrain: hendra_content::Terrain::None,
            selling: None,
            glow: 0,
            belongs_to: None,
            tex1: 0,
            tex2: 0,
            connection: 0,
            portal_unusable: false,
            admin: false,
            has_backpack: false,
            name_chosen: false,
            fame_goal: 0,
            loot_drop_boost_ms: 0,
            loot_tier_boost_ms: 0,
            damage_by: Vec::new(),
            seen: None,
            sight: None,
            quest_target: None,
            watching: None,
            unseen_ms: 0,
            stars: 0,
            last_hurt_by: None,
            loot_drop: 1.0,
            experience_boost_ms: 0,
            purse: crate::world::Purse::default(),
            awards_experience: true,
            burn_due_ms: 0,
            oxygen: 100,
            boosts: Vec::new(),
            tally: crate::fame::Tally::default(),
            teleport_cooldown_ms: 0,
            move_grace_ms: 0,
            x,
            y,
            hp,
            max_hp: hp,
            mp: 0,
            max_mp: 0,
            conditions: ConditionSet::EMPTY,
            size: 100,
            name: None,
            guild: None,
            guild_rank: 0,
            stats: crate::stats::Stats::still(),
            weapon: None,
            ready_at_ms: 0,
            spawn_x: x,
            spawn_y: y,
            mind: None,
            container: None,
            expires_in_ms: None,
            dead: false,
            damage_since_tick: 0,
            texture: 0,
            skin: 0,
            default_skin: 0,
            default_size: 100,
            resizing: None,
            spawned: false,
            removed: false,
            effects: Vec::new(),
            armed: None,
            decoy: None,
            holds_conditions: true,
            progress: crate::leveling::Progress::new(),
            health_fraction: 0.0,
            health_regen_fraction: 0.0,
            magic_regen_fraction: 0.0,
            magic_fraction: 0.0,
            flash: None,
            base_max_hp: None,
            trail: crate::world::Trail::default(),
        }
    }

    fn bullet(owner: Handle, x: f32, y: f32, angle: f32, damage: i32) -> Projectile {
        Projectile {
            owner,
            from_player: true,
            object_type: ObjectType(0x900),
            x,
            y,
            start_x: x,
            start_y: y,
            angle,
            id: 0,
            wavy: false,
            parametric: false,
            boomerang: false,
            amplitude: 0.0,
            frequency: 0.0,
            magnitude: 0.0,
            speed: 10.0,
            damage,
            age_ms: 0,
            lifetime_ms: 1000,
            multi_hit: false,
            passes_cover: false,
            armor_piercing: false,
            effects: Vec::new(),
            struck: Vec::new(),
            predicted_by_owner: true,
            container: ObjectType(0x901),
        }
    }

    #[test]
    fn a_plain_shot_travels_in_a_straight_line() {
        let shot = bullet(Handle::NONE, 0.0, 0.0, 0.0, 10);

        // Ten tiles a second, aimed along the x axis.
        assert!((shot.position_at(1000).0 - 10.0).abs() < 0.001);
        assert!(shot.position_at(1000).1.abs() < 0.001);

        // And half the time is half the distance, which is what "straight" means here.
        assert!((shot.position_at(500).0 - 5.0).abs() < 0.001);
    }

    #[test]
    fn a_boomerang_comes_back_to_where_it_was_thrown() {
        // It turns round at the halfway point of its life rather than at a distance, so it returns
        // to the hand exactly as it expires.
        let mut shot = bullet(Handle::NONE, 0.0, 0.0, 0.0, 10);
        shot.boomerang = true;
        shot.lifetime_ms = 1000;

        let out = shot.position_at(500).0;
        assert!(out > 4.0, "it did not go anywhere: {out}");

        assert!(
            shot.position_at(1000).0.abs() < 0.001,
            "it did not come back: {}",
            shot.position_at(1000).0
        );

        // Symmetrical about the turn: the same distance out and back.
        assert!((shot.position_at(250).0 - shot.position_at(750).0).abs() < 0.001);
    }

    #[test]
    fn an_amplitude_carries_a_shot_sideways_without_slowing_it() {
        let mut shot = bullet(Handle::NONE, 0.0, 0.0, 0.0, 10);
        shot.amplitude = 2.0;
        shot.frequency = 1.0;
        shot.lifetime_ms = 1000;

        // A quarter of the way through one full wave is the far side of it.
        let (x, y) = shot.position_at(250);
        assert!((y - 2.0).abs() < 0.001, "sideways was {y}");

        // Forward progress is untouched: the wave is across the line of travel, not along it.
        assert!((x - 2.5).abs() < 0.001, "forward was {x}");

        // And it crosses back, which is what makes it a wave rather than a curve.
        assert!(shot.position_at(750).1 < -1.9);
    }

    #[test]
    fn neighbouring_bullets_of_a_volley_wave_to_opposite_sides() {
        // The parity of the shot's place in the volley. Without it every bullet traces the same
        // line and a wave pattern is a single thick line.
        let mut even = bullet(Handle::NONE, 0.0, 0.0, 0.0, 10);
        even.amplitude = 2.0;
        even.frequency = 1.0;
        even.id = 0;

        let mut odd = even.clone();
        odd.id = 1;

        let (left, right) = (even.position_at(250).1, odd.position_at(250).1);
        assert!(left > 1.9 && right < -1.9, "both went the same way");
    }

    #[test]
    fn a_wavy_shot_wobbles_about_its_line_rather_than_sweeping_around_it() {
        // Divided rather than multiplied, which is a wobble of about three degrees. Written the
        // other way up it sweeps some thirty full turns, and this path decides whether a bullet
        // went through somebody.
        let mut shot = bullet(Handle::NONE, 0.0, 0.0, 0.0, 10);
        shot.wavy = true;
        shot.lifetime_ms = 1000;

        for age in [100, 250, 500, 750, 1000] {
            let (x, y) = shot.position_at(age);
            let off = y.atan2(x).to_degrees().abs();
            assert!(off < 4.0, "at {age}ms it was {off} degrees off its line");
        }

        // And it does wobble, rather than being straight by accident.
        let worst = (0..20)
            .map(|step| {
                let (x, y) = shot.position_at(step * 50);
                y.atan2(x).to_degrees().abs()
            })
            .fold(0.0f32, f32::max);
        assert!(worst > 1.0, "it never left its line: {worst} degrees");
    }

    #[test]
    fn a_parametric_shot_traces_a_figure_and_returns() {
        let mut shot = bullet(Handle::NONE, 0.0, 0.0, 0.0, 10);
        shot.parametric = true;
        shot.magnitude = 3.0;
        shot.lifetime_ms = 1000;

        // It starts and ends where it was fired, which is what makes it a figure rather than a path
        // outward.
        assert!(shot.position_at(0).0.abs() < 0.001);
        assert!(shot.position_at(1000).0.abs() < 0.01);

        // And it goes somewhere in between.
        let reach = (0..20)
            .map(|step| {
                let (x, y) = shot.position_at(step * 50);
                (x * x + y * y).sqrt()
            })
            .fold(0.0f32, f32::max);
        assert!(reach > 2.0, "it never left the muzzle: {reach}");
        assert!(
            reach <= 3.0 * 1.5,
            "it went further than its magnitude: {reach}"
        );
    }

    #[test]
    fn the_content_that_curves_reaches_the_simulation() {
        // The bug this guards was not a wrong formula but a right one nothing called: the fields
        // were read from the files, held on the descriptor, and never asked for. Every shot in the
        // game travelled in a straight line and the content said otherwise.
        let Ok((catalog, _)) =
            Catalog::load_dir(std::path::Path::new("../../../godot-client/assets/xml"))
        else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let mut curved = 0;
        let mut checked = 0;

        for number in 0..u16::MAX {
            let Some(desc) = catalog.object(ObjectType(number)) else {
                continue;
            };

            for shot in &desc.projectiles {
                if !shot.wavy && !shot.parametric && !shot.boomerang && shot.amplitude == 0.0 {
                    continue;
                }
                checked += 1;

                let built = Projectile::from_desc(
                    Handle::NONE,
                    false,
                    ObjectType::NONE,
                    shot,
                    0.0,
                    0.0,
                    0.0,
                    0.5,
                    0,
                );

                // Compared against the same shot with nothing curving it, rather than against a
                // threshold. Some of these are a tenth of a tile wide and some turn round inside
                // fifty milliseconds, and any fixed distance that catches one calls the other
                // straight. What is being asked is only whether the fields changed the path.
                let mut straight = built.clone();
                straight.wavy = false;
                straight.parametric = false;
                straight.boomerang = false;
                straight.amplitude = 0.0;

                let moved = (1..=200).any(|step| {
                    let age = built.lifetime_ms * step / 200;
                    let (x, y) = built.position_at(age);
                    let (sx, sy) = straight.position_at(age);
                    (x - sx).abs() > 1e-4 || (y - sy).abs() > 1e-4
                });

                if moved {
                    curved += 1;
                } else {
                    eprintln!(
                        "{} fires a shot the content curves and the path does not",
                        desc.id
                    );
                }
            }
        }

        assert!(checked > 100, "only {checked} curving shots were found");
        assert_eq!(
            curved,
            checked,
            "{} shots the content says do something travelled straight out and kept going",
            checked - curved
        );
    }

    /// Builds a world's worth of state: entities, an index over them, and terrain.
    fn scene(catalog: &Catalog, placed: Vec<Entity>) -> (Slab<Entity>, Grid, Terrain, Vec<Handle>) {
        let mut entities = Slab::new();
        let handles: Vec<Handle> = placed
            .into_iter()
            .map(|entity| entities.insert(entity).unwrap())
            .collect();

        let mut grid = Grid::new(32, 32);
        grid.rebuild(
            entities
                .iter()
                .map(|(handle, entity)| (handle, entity.x, entity.y))
                .collect::<Vec<_>>(),
        );

        (entities, grid, terrain(catalog), handles)
    }

    #[test]
    fn a_projectile_travels_and_expires() {
        let catalog = catalog();
        let (entities, grid, terrain, _) = scene(&catalog, vec![]);

        let mut projectiles = Projectiles::new();
        let handle = projectiles
            .fire(bullet(Handle::NONE, 5.0, 5.0, 0.0, 100))
            .unwrap();

        let mut hits = Vec::new();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 50, &mut hits);

        let flying = projectiles.get(handle).expect("still in flight");
        assert!(flying.x > 5.0, "should have moved east");
        assert!((flying.y - 5.0).abs() < 1e-4, "and not drifted north");

        // A one-second lifetime at 50 ms a tick.
        for _ in 0..25 {
            projectiles.advance(&entities, &grid, &terrain, &catalog, 50, &mut hits);
        }
        assert!(projectiles.is_empty(), "it should have expired");
    }

    #[test]
    fn a_projectile_strikes_an_enemy_in_its_path() {
        let catalog = catalog();
        let (entities, grid, terrain, handles) = scene(&catalog, vec![enemy(0x502, 6.0, 5.0, 200)]);

        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(Handle::NONE, 5.0, 5.0, 0.0, 100))
            .unwrap();

        let mut hits = Vec::new();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 100, &mut hits);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].target, handles[0]);
        // 100 raw against 10 defence.
        assert_eq!(hits[0].damage, 90);
        assert!(!hits[0].fatal);

        assert!(
            projectiles.is_empty(),
            "a single-hit bullet stops on impact"
        );
    }

    #[test]
    fn a_fast_projectile_does_not_pass_through_its_target() {
        // The bug that stepping the path prevents: at 20 ticks a second a fast bullet covers more
        // than a tile per tick, so testing only where it lands would miss anything it crossed.
        let catalog = catalog();
        let (entities, grid, terrain, _) = scene(&catalog, vec![enemy(0x502, 6.0, 5.0, 200)]);

        let mut fast = bullet(Handle::NONE, 5.0, 5.0, 0.0, 100);
        fast.speed = 40.0; // two tiles per 50 ms tick

        let mut projectiles = Projectiles::new();
        projectiles.fire(fast).unwrap();

        let mut hits = Vec::new();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 50, &mut hits);

        assert_eq!(
            hits.len(),
            1,
            "it crossed the target and must have struck it"
        );
    }

    #[test]
    fn a_projectile_never_hits_its_owner() {
        let catalog = catalog();
        let mut entities = Slab::new();
        let shooter = entities
            .insert(Entity::player(ObjectType(0x600), 5.0, 5.0, 800))
            .unwrap();
        let mut grid = Grid::new(32, 32);
        grid.rebuild(vec![(shooter, 5.0, 5.0)]);

        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(shooter, 5.0, 5.0, 0.0, 100))
            .unwrap();

        let mut hits = Vec::new();
        projectiles.advance(
            &entities,
            &grid,
            &terrain(&catalog),
            &catalog,
            50,
            &mut hits,
        );

        assert!(
            hits.is_empty(),
            "a shot must not strike the one who fired it"
        );
    }

    #[test]
    fn players_do_not_shoot_players_and_enemies_do_not_shoot_enemies() {
        let catalog = catalog();

        let mut entities = Slab::new();
        let other_player = entities
            .insert(Entity::player(ObjectType(0x600), 6.0, 5.0, 800))
            .unwrap();
        let slime = entities.insert(enemy(0x502, 7.0, 5.0, 200)).unwrap();

        let mut grid = Grid::new(32, 32);
        grid.rebuild(vec![(other_player, 6.0, 5.0), (slime, 7.0, 5.0)]);
        let terrain = terrain(&catalog);

        // 200 ms at ten tiles a second carries the shot two tiles, past the player at 6 and into
        // the enemy at 7. A shorter step would stop between them and prove nothing.
        let flight_ms = 200;

        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(Handle::NONE, 5.0, 5.0, 0.0, 100))
            .unwrap();

        let mut hits = Vec::new();
        projectiles.advance(&entities, &grid, &terrain, &catalog, flight_ms, &mut hits);

        assert_eq!(hits.len(), 1, "a player's shot should reach the enemy");
        assert_eq!(hits[0].target, slime, "it should have passed the player by");

        // An enemy's shot does the opposite: it stops at the player and never reaches the slime.
        let mut enemy_shot = bullet(Handle::NONE, 5.0, 5.0, 0.0, 100);
        enemy_shot.from_player = false;

        let mut projectiles = Projectiles::new();
        projectiles.fire(enemy_shot).unwrap();
        projectiles.advance(&entities, &grid, &terrain, &catalog, flight_ms, &mut hits);

        assert_eq!(hits.len(), 1, "an enemy's shot should reach the player");
        assert_eq!(hits[0].target, other_player);
    }

    #[test]
    fn cover_stops_a_projectile_unless_it_passes_cover() {
        use hendra_content::map::Composition;
        use hendra_content::{Map, Region, TileType};

        let catalog = catalog();

        // A tree at x = 6 blocks the line between 5 and 7.
        let mut squares: Vec<Composition> = (0..32 * 32)
            .map(|_| Composition {
                tile: TileType(0x10),
                object: ObjectType::NONE,
                region: Region::None,
                terrain: hendra_content::Terrain::None,
                config: String::new(),
            })
            .collect();
        squares[5 * 32 + 6].object = ObjectType(0x501);

        let terrain = Terrain::build(Map::from_squares(32, 32, squares).unwrap(), &catalog);

        let mut entities = Slab::new();
        let target = entities.insert(enemy(0x502, 7.5, 5.5, 200)).unwrap();
        let mut grid = Grid::new(32, 32);
        grid.rebuild(vec![(target, 7.5, 5.5)]);

        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(Handle::NONE, 5.5, 5.5, 0.0, 100))
            .unwrap();

        let mut hits = Vec::new();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 200, &mut hits);
        assert!(hits.is_empty(), "the tree should have stopped it");
        assert!(projectiles.is_empty());

        // The same shot, allowed through cover.
        let mut through = bullet(Handle::NONE, 5.5, 5.5, 0.0, 100);
        through.passes_cover = true;

        let mut projectiles = Projectiles::new();
        projectiles.fire(through).unwrap();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 200, &mut hits);
        assert_eq!(hits.len(), 1, "PassesCover should ignore the tree");
    }

    #[test]
    fn a_multi_hit_projectile_hits_each_target_once() {
        let catalog = catalog();
        let (entities, grid, terrain, handles) = scene(
            &catalog,
            vec![enemy(0x502, 6.0, 5.0, 200), enemy(0x502, 7.0, 5.0, 200)],
        );

        let mut piercing = bullet(Handle::NONE, 5.0, 5.0, 0.0, 100);
        piercing.multi_hit = true;

        let mut projectiles = Projectiles::new();
        projectiles.fire(piercing).unwrap();

        let mut hits = Vec::new();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 200, &mut hits);

        let struck: Vec<Handle> = hits.iter().map(|hit| hit.target).collect();
        assert!(struck.contains(&handles[0]));
        assert!(struck.contains(&handles[1]));
        assert_eq!(struck.len(), 2, "each target exactly once");

        // And it must not strike the same target again on a later tick.
        let before = hits.len();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 50, &mut hits);
        assert!(hits.len() <= before, "no repeats on a later tick");
    }

    #[test]
    fn one_enemy_bullet_is_taken_by_everybody_it_passes() {
        // `ForceHit` (`Projectile.cs:472`) refuses a spent bullet a second target unless that
        // target is a player, and the players' own clients each test a bullet against themselves
        // before anybody else (`Projectile.as:307`). So a crowd standing together all take the
        // same shot, which is what a boss room is: stopping at the first body would leave everyone
        // behind the front rank untouched.
        let catalog = catalog();

        let mut entities = Slab::new();
        let front = entities
            .insert(Entity::player(ObjectType(0x600), 6.0, 5.0, 800))
            .unwrap();
        let behind = entities
            .insert(Entity::player(ObjectType(0x600), 7.0, 5.0, 800))
            .unwrap();

        let mut grid = Grid::new(32, 32);
        grid.rebuild(vec![(front, 6.0, 5.0), (behind, 7.0, 5.0)]);
        let terrain = terrain(&catalog);

        let mut shot = bullet(Handle::NONE, 5.0, 5.0, 0.0, 100);
        shot.from_player = false;

        let mut projectiles = Projectiles::new();
        projectiles.fire(shot).unwrap();

        let mut hits = Vec::new();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 200, &mut hits);

        let struck: Vec<Handle> = hits.iter().map(|hit| hit.target).collect();
        assert!(struck.contains(&front), "the first player was missed");
        assert!(
            struck.contains(&behind),
            "the second player was sheltered by the first"
        );

        // A player's own shot is the other half of the same rule: spent on the first monster and
        // no use against the second.
        let (entities, grid, terrain, handles) = scene(
            &catalog,
            vec![enemy(0x502, 6.0, 5.0, 200), enemy(0x502, 7.0, 5.0, 200)],
        );

        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(Handle::NONE, 5.0, 5.0, 0.0, 100))
            .unwrap();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 200, &mut hits);

        assert_eq!(hits.len(), 1, "one bullet, one monster");
        assert_eq!(hits[0].target, handles[0]);
        assert!(projectiles.is_empty(), "and the bullet is spent");
    }

    #[test]
    fn what_stops_a_shot_is_what_occupies_the_square_rather_than_what_blocks_the_view() {
        use hendra_content::map::Composition;
        use hendra_content::{Map, Region, TileType};

        // 154 objects in the shipped content occupy their square without blocking sight -- fences,
        // pillars, statues, benches -- and the client ends a bullet's flight on every one of them
        // (`Projectile.as:229-232`). Judging by sight instead let a shot through all of them: the
        // player watched their bullet break against a fence while the server carried it on into
        // whoever was standing behind it.
        let catalog = catalog();

        let square = |object: u16| Composition {
            tile: TileType(0x10),
            object: ObjectType(object),
            region: Region::None,
            terrain: hendra_content::Terrain::None,
            config: String::new(),
        };

        let mut squares: Vec<Composition> =
            (0..32 * 32).map(|_| square(ObjectType::NONE.0)).collect();
        squares[5 * 32 + 6] = square(0x505);
        let fenced = Terrain::build(Map::from_squares(32, 32, squares).unwrap(), &catalog);

        let mut entities = Slab::new();
        let target = entities.insert(enemy(0x502, 7.5, 5.5, 200)).unwrap();
        let mut grid = Grid::new(32, 32);
        grid.rebuild(vec![(target, 7.5, 5.5)]);

        let mut hits = Vec::new();
        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(Handle::NONE, 5.5, 5.5, 0.0, 100))
            .unwrap();
        projectiles.advance(&entities, &grid, &fenced, &catalog, 200, &mut hits);

        assert!(hits.is_empty(), "the fence should have stopped the shot");
        assert!(projectiles.is_empty());

        // A shot that passes cover goes through a fence, and does not go through something that
        // occupies the square against enemies as well.
        let mut through = bullet(Handle::NONE, 5.5, 5.5, 0.0, 100);
        through.passes_cover = true;

        let mut projectiles = Projectiles::new();
        projectiles.fire(through.clone()).unwrap();
        projectiles.advance(&entities, &grid, &fenced, &catalog, 200, &mut hits);
        assert_eq!(hits.len(), 1, "PassesCover should ignore a fence");

        let mut squares: Vec<Composition> =
            (0..32 * 32).map(|_| square(ObjectType::NONE.0)).collect();
        squares[5 * 32 + 6] = square(0x506);
        let bollarded = Terrain::build(Map::from_squares(32, 32, squares).unwrap(), &catalog);

        let mut projectiles = Projectiles::new();
        projectiles.fire(through).unwrap();
        projectiles.advance(&entities, &grid, &bollarded, &catalog, 200, &mut hits);
        assert!(
            hits.is_empty(),
            "EnemyOccupySquare stops a shot however it was declared"
        );
    }

    #[test]
    fn a_hidden_player_shoots_nothing() {
        // `EnemyHitHandler.cs:24` drops a hit claim outright while the shooter is Hidden, which is
        // the admin's own invisibility: watching a fight unseen is not a way to join it.
        let catalog = catalog();

        let mut entities = Slab::new();
        let shooter = entities
            .insert(Entity::player(ObjectType(0x600), 5.0, 5.0, 800))
            .unwrap();
        let slime = entities.insert(enemy(0x502, 6.0, 5.0, 200)).unwrap();

        let mut grid = Grid::new(32, 32);
        grid.rebuild(vec![(shooter, 5.0, 5.0), (slime, 6.0, 5.0)]);
        let terrain = terrain(&catalog);

        let mut hits = Vec::new();
        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(shooter, 5.0, 5.0, 0.0, 100))
            .unwrap();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 100, &mut hits);
        assert_eq!(hits.len(), 1, "an ordinary shot lands");

        entities
            .get_mut(shooter)
            .unwrap()
            .conditions
            .insert(hendra_content::ConditionEffect::Hidden);

        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(shooter, 5.0, 5.0, 0.0, 100))
            .unwrap();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 100, &mut hits);
        assert!(hits.is_empty(), "a hidden player's shot does nothing");
    }

    #[test]
    fn defence_reduces_damage_but_never_to_nothing() {
        let plain = crate::effects::Rules::NONE;

        assert_eq!(plain.damage_after_defence(100, 10, false, false), 90);
        assert_eq!(plain.damage_after_defence(100, 0, false, false), 100);

        // Enough defence to cancel the shot outright still lets a quarter through, so a heavily
        // armoured target is slow to kill rather than immune.
        assert_eq!(plain.damage_after_defence(100, 1000, false, false), 25);
        assert_eq!(plain.damage_after_defence(20, 1000, false, false), 5);

        // Armour piercing ignores it entirely.
        assert_eq!(plain.damage_after_defence(100, 1000, true, false), 100);

        // Nothing goes negative.
        assert_eq!(plain.damage_after_defence(0, 50, false, false), 0);
    }

    #[test]
    fn a_player_is_defended_by_their_stats_and_an_enemy_by_its_type() {
        // Reading one from the other's place is silent: an enemy has no stats and a player's
        // descriptor holds the level-one class base, which for most classes is zero.
        let catalog = catalog();

        // A player wearing seventeen points of defence.
        let mut player = enemy(0x503, 0.0, 0.0, 100);
        player.kind = Kind::Player;
        player.stats.set_equipment({
            let mut worn = [0i32; hendra_content::STAT_COUNT];
            worn[3] = 17;
            worn
        });
        assert_eq!(defence_of(&player, &catalog), 17);

        // The same object type read as an enemy answers from the content instead, which is a
        // different number: this is what made a player's armour count for nothing.
        let armoured = enemy(0x503, 0.0, 0.0, 100);
        let from_content = catalog
            .object(armoured.object_type)
            .map(|desc| desc.defense)
            .unwrap_or(0);
        assert_ne!(from_content, 17);
        assert_eq!(defence_of(&armoured, &catalog), from_content);
    }

    #[test]
    fn an_extremely_armoured_enemy_still_takes_damage() {
        let catalog = catalog();
        let (entities, grid, terrain, _) = scene(&catalog, vec![enemy(0x503, 6.0, 5.0, 500)]);

        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(Handle::NONE, 5.0, 5.0, 0.0, 100))
            .unwrap();

        let mut hits = Vec::new();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 100, &mut hits);

        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].damage, 25,
            "1000 defence, and a quarter of the hit still lands"
        );
    }

    #[test]
    fn a_handle_to_an_expired_projectile_stops_resolving() {
        // The property the whole design rests on. The legacy server's byte-wide bullet ids wrapped
        // every 256 shots and a stale one resolved against a live projectile.
        let catalog = catalog();
        let (entities, grid, terrain, _) = scene(&catalog, vec![]);

        let mut projectiles = Projectiles::new();
        let first = projectiles
            .fire(bullet(Handle::NONE, 5.0, 5.0, 0.0, 100))
            .unwrap();

        projectiles.remove(first);

        let second = projectiles
            .fire(bullet(Handle::NONE, 9.0, 9.0, 0.0, 100))
            .unwrap();

        assert_eq!(first.index(), second.index(), "the slot was reused");
        assert_ne!(first, second, "but the handle was not");
        assert!(
            projectiles.get(first).is_none(),
            "the stale handle must miss"
        );
        assert!(projectiles.get(second).is_some());

        let mut hits = Vec::new();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 50, &mut hits);
    }

    #[test]
    fn a_target_marked_dead_is_not_struck_again_but_one_merely_at_zero_is() {
        // Health is not what makes a body untargetable — being marked dead is. An enemy dies on
        // `HP < 0` (`Enemy.cs:127`), so one resting on exactly zero is alive and `HitByProjectile`
        // gates on nothing that would refuse it. Reading zero as untargetable is what leaves an
        // enemy with no health left that no shot can ever finish.
        let catalog = catalog();
        let mut entities = Slab::new();
        let spent = entities.insert(enemy(0x502, 6.0, 5.0, 0)).unwrap();
        let mut grid = Grid::new(32, 32);
        grid.rebuild(vec![(spent, 6.0, 5.0)]);

        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(Handle::NONE, 5.0, 5.0, 0.0, 100))
            .unwrap();

        let mut hits = Vec::new();
        projectiles.advance(
            &entities,
            &grid,
            &terrain(&catalog),
            &catalog,
            100,
            &mut hits,
        );

        assert_eq!(hits.len(), 1, "an enemy on nothing is still there to hit");
        assert!(hits[0].fatal, "and this one finishes it");

        entities.get_mut(spent).unwrap().dead = true;
        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(Handle::NONE, 5.0, 5.0, 0.0, 100))
            .unwrap();

        hits.clear();
        projectiles.advance(
            &entities,
            &grid,
            &terrain(&catalog),
            &catalog,
            100,
            &mut hits,
        );

        assert!(hits.is_empty(), "a body already reaped is not there");
    }

    #[test]
    fn an_enemy_whose_type_declares_no_health_cannot_be_hit_at_all() {
        // `stat = ObjectDesc.MaxHP == 0` (`Enemy.cs:20`), and both `Enemy.Damage` and
        // `Enemy.HitByProjectile` return before anything else when it is set. Three hundred and
        // forty-six of the shipped enemies declare no `MaxHitPoints`: spawners, encounter managers
        // and turrets, all of which the content expects to be indestructible. A server that lets
        // them be shot lets a player switch a dungeon off.
        let catalog = catalog();
        let mut entities = Slab::new();
        let spawner = entities.insert(enemy(0x504, 6.0, 5.0, 0)).unwrap();
        let mut grid = Grid::new(32, 32);
        grid.rebuild(vec![(spawner, 6.0, 5.0)]);

        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(Handle::NONE, 5.0, 5.0, 0.0, 100))
            .unwrap();

        let mut hits = Vec::new();
        projectiles.advance(
            &entities,
            &grid,
            &terrain(&catalog),
            &catalog,
            100,
            &mut hits,
        );

        assert!(hits.is_empty(), "a spawner is machinery, not a target");
    }

    #[test]
    fn orphaned_projectiles_are_dropped() {
        let catalog = catalog();
        let mut entities = Slab::new();
        let shooter = entities
            .insert(Entity::player(ObjectType(0x600), 5.0, 5.0, 800))
            .unwrap();

        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(shooter, 5.0, 5.0, 0.0, 100))
            .unwrap();
        assert_eq!(projectiles.len(), 1);

        entities.remove(shooter);
        projectiles.drop_orphans(&entities);

        assert!(
            projectiles.is_empty(),
            "the shooter left; the shot goes with them"
        );
        let _ = catalog;
    }

    #[test]
    fn a_fatal_hit_is_reported_as_fatal() {
        let catalog = catalog();
        let (entities, grid, terrain, _) = scene(&catalog, vec![enemy(0x502, 6.0, 5.0, 50)]);

        let mut projectiles = Projectiles::new();
        projectiles
            .fire(bullet(Handle::NONE, 5.0, 5.0, 0.0, 100))
            .unwrap();

        let mut hits = Vec::new();
        projectiles.advance(&entities, &grid, &terrain, &catalog, 100, &mut hits);

        assert_eq!(hits.len(), 1);
        assert!(hits[0].fatal, "90 damage against 50 hit points");
    }
}

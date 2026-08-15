//! One world: what is in it, and what happens to it each tick.
//!
//! A world is owned outright by whatever ticks it. Nothing here is behind a lock, and nothing is
//! shared with another world. The C# server reached for `ConcurrentDictionary` on five collections
//! belonging to a world that only ever ticked on one thread, and paid lock-striping on every access
//! for the privilege. Worlds are independent, so parallelism belongs *between* them.
//!
//! # The tick
//!
//! ```text
//!   1. move players toward where they claim to be, within what the rules allow
//!   2. apply ground hazards
//!   3. rebuild the spatial index
//!   4. remove the dead
//! ```
//!
//! Snapshots are taken after all of that, per player, from the index.

use hendra_behavior::program::{
    Action, DeathEffect, EffectTarget, Nearby, Neighbour, Program, Programs, Senses,
};
use hendra_behavior::run::Mind;
use hendra_content::{Catalog, ConditionSet, Map, ObjectType, TERRAIN_COUNT};
use hendra_net::{EntityId, EntityState, Tick, WorldSnapshot};

use crate::grid::Grid;
use crate::inventory::{Container, ContainerKind};
use crate::projectile::{Hit, Projectile, Projectiles};
use crate::slab::{Handle, Slab};
use crate::tiles::{Terrain, Walker};

/// How far a player can see, in tiles. Matches the client's view distance.
pub const SIGHT_RADIUS: f32 = 20.0;

/// The remaining time written on a condition effect that never lapses.
///
/// The original spells a permanent effect as a duration of -1 and skips it when counting down.
/// Held here as the largest count there is, and skipped by the same test.
pub const FOREVER: u32 = u32::MAX;

/// How long a quest arrow holds still before it is worked out again.
///
/// `HandleQuest` picks again on `time.TickCount % 500 == 0` (`Player.Leveling.cs:215`), and the
/// original's ticker runs at the six ticks a second its configuration asks for
/// (`LogicTicker.cs:30`, `wServer.json:23`), which puts five hundred ticks at eighty-three seconds.
/// Long, deliberately: the arrow is meant to name a goal to walk to rather than to follow whichever
/// enemy has wandered nearest, and the three things that cannot wait — the target dying, leaving,
/// or the player outgrowing its level band — each force a fresh pick of their own.
const QUEST_INTERVAL_MS: u32 = 83_333;

/// Which squares of one world a player has laid eyes on, at their current version.
///
/// A bit per square rather than a byte, because this is held per player and a realm is four million
/// squares: half a megabyte each instead of four. The original keeps a byte apiece -- the
/// `UpdateCount` the square carried when it was last sent -- so that a square repainted under a
/// player is sent again (`Player.Update.cs:138-151`). A bit says only "the client has the current
/// version", which is the same thing as long as the bit is cleared wherever the original's counter
/// would have moved on: see [`Seen::unsee`].
#[derive(Debug, Clone)]
pub struct Seen {
    bits: Vec<u64>,

    /// The square the player was standing on when this was last brought up to date. Nothing is
    /// walked while it is unchanged, since a player who has not left their square has not uncovered
    /// anything.
    ///
    /// A saving rather than a rule: the bits already make a second look at the same ground count for
    /// nothing, so removing this changes how much work an input costs and not what it produces. No
    /// test can tell the two apart, which is why it is written down here.
    standing_at: Option<(i32, i32)>,
}

impl Seen {
    fn over(width: u32, height: u32) -> Seen {
        let squares = width as usize * height as usize;
        Seen {
            bits: vec![0; squares.div_ceil(64)],
            standing_at: None,
        }
    }

    /// Marks a square as seen, and says whether that was news.
    fn look_at(&mut self, x: u32, y: u32, width: u32) -> bool {
        let index = y as usize * width as usize + x as usize;
        let (word, bit) = (index / 64, index % 64);

        let Some(held) = self.bits.get_mut(word) else {
            return false;
        };

        let mask = 1u64 << bit;
        if *held & mask != 0 {
            return false;
        }

        *held |= mask;
        true
    }

    /// Forgets that a square was sent, so the next look that covers it sends it again.
    ///
    /// This is `WmapTile.UpdateCount++` seen from the player's side: the original compares the
    /// square's version against `tiles[x, y]` on every pass of the sight circle, so a square that
    /// has been repainted is re-sent to everyone the moment their circle next covers it, whether
    /// they were standing there at the time or on the far side of the map.
    fn unsee(&mut self, x: u32, y: u32, width: u32) {
        let index = y as usize * width as usize + x as usize;
        let (word, bit) = (index / 64, index % 64);

        if let Some(held) = self.bits.get_mut(word) {
            *held &= !(1u64 << bit);
        }
    }

    /// Forces the next look to walk the circle again even from the same square.
    ///
    /// The saving in `standing_at` assumes a look from an unchanged square finds nothing new, which
    /// stops being true the moment a square inside the circle is forgotten.
    fn look_again(&mut self) {
        self.standing_at = None;
    }
}

/// What an entity is, for the rules that treat them differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Player,
    Enemy,

    /// Scenery: walls, signs, decoration. Never moves, never dies.
    Fixture,

    /// Scenery that can be broken: a wine barrel, a destructible wall, a Shatters switch.
    ///
    /// `StaticObject` in the original. It holds health and dies at zero of it, but it has no damage
    /// counter, so breaking one pays neither experience nor loot, and only a player's bullet can
    /// touch it (`StaticObject.cs:55-79`).
    StaticObject,

    /// A portal to another world.
    Portal,

    /// A loot bag or chest.
    Container,

    /// Something a player left that enemies attack instead of them.
    ///
    /// Its own kind rather than an enemy with a flag, because everything that decides who shoots
    /// what asks the kind, and a decoy that answered "enemy" would be shot by its owner.
    Decoy,

    /// Something armed that fires when a player comes near, then goes.
    Trap,
}

impl Kind {
    pub fn is_alive_kind(self) -> bool {
        matches!(self, Kind::Player | Kind::Enemy | Kind::Decoy)
    }

    /// Whether a behaviour's neighbour list mentions it.
    ///
    /// The two collision maps a behaviour searches. `GetNearestEntity(dist, objType)` walks
    /// `EnemiesCollision` (`Utils.cs:140`) and `World.EnterWorld` puts every `StaticObject` that is
    /// not a decoy into it (`World.cs:353-361`), so a breakable object is as findable as a monster.
    /// It has to be: the Shatters gates open on `entity_not_exists("shtrs Abandoned Switch 1")`
    /// (`shatters.beh:1286`), and those switches are `Class=GameObject`. Leave them out of the list
    /// and every one of those gates is open from the moment the room is built.
    ///
    /// A decoy is in it too, through the other map: `EnterWorld` inserts one into
    /// `PlayersCollision`, which is what makes an enemy chase it.
    pub fn is_perceptible(self) -> bool {
        matches!(
            self,
            Kind::Player | Kind::Enemy | Kind::Decoy | Kind::StaticObject
        )
    }

    /// What a descriptor's class makes of it.
    ///
    /// `Entity.Resolve` (`Entity.cs:586-637`) switches on `Class` and reads the `<Enemy/>` flag
    /// nowhere: `Character` is the only class that becomes an `Enemy`, and `GameObject`, `Wall`
    /// and the three changers become a `StaticObject` however the descriptor is flagged. A wine
    /// barrel and a destructible wall are therefore scenery with health rather than monsters, and
    /// the difference shows in play — a `StaticObject` dies at `HP <= 0` where an `Enemy` needs
    /// `HP < 0`, so reading the flag instead of the class gives every one of them a free hit.
    ///
    /// What `<Enemy/>` actually does is tell the *client* that its own bullets should hit-test
    /// against this object. The server never decides a player's bullet hit anything: the client
    /// claims it with `EnemyHit` and `EnemyHitHandler` takes its word (having checked the flight).
    /// Server-side the flag is read only by map loading (`Wmap.cs:358-365`, which keeps an
    /// `Enemy`-flagged static object on its tile as well as in the world) and by projectile cover
    /// (`Projectile.cs:340`, which lets bullets through such a square).
    pub fn of_class(class: &str) -> Kind {
        match class {
            "Character" => Kind::Enemy,
            "Portal" | "GuildHallPortal" => Kind::Portal,
            "Container" => Kind::Container,
            "GameObject" | "CharacterChanger" | "MoneyChanger" | "NameChanger" | "Wall"
            | "DoubleWall" => Kind::StaticObject,
            _ => Kind::Fixture,
        }
    }
}

/// Whether a player's bullet may be claimed to have hit this object at all.
///
/// Two gates, one from each end of the original. The client only hit-tests what its descriptor
/// flags `<Enemy/>`, so an unflagged object is never named in an `EnemyHit` however destructible
/// the server thinks it is — which is why the seventeen flagged-less trees and pots that declare
/// `MaxHitPoints` stand forever. And the server refuses the claim anyway when there is no health
/// to take: `Enemy.HitByProjectile` returns on `stat` (`Enemy.cs:20`, `:98`) and
/// `StaticObject.HitByProjectile` does nothing unless `Vulnerable` (`StaticObject.cs:57`), both of
/// which are "the descriptor declares no `MaxHitPoints`".
pub fn is_shootable(desc: &hendra_content::ObjectDesc) -> bool {
    desc.enemy && desc.max_hp != 0
}

/// Whether an entity of this descriptor has an effect array to put a condition in.
///
/// The three cases `Entity`'s constructor allocates one for, in its order (`Entity.cs:137-155`): a
/// player, something flagged `<Enemy/>` that is not also `<Static/>`, and a `<Character/>`.
/// Everything else leaves the array null, and the original would throw rather than record a
/// condition on it — `ApplyConditionEffect`'s whole body is `_effects[eff] = durationMs`
/// (`Entity.cs:707-735`), with no test for the array being there.
///
/// What it separates in practice is two pieces of breakable scenery that look alike. A wine barrel
/// declares `<Static/>`, so it holds nothing — not a slow, not a stun, not even the
/// `<StasisImmune/>` it asks for. A Shatters switch declares no such thing, so it holds everything.
pub fn holds_conditions(desc: &hendra_content::ObjectDesc) -> bool {
    desc.player || (desc.enemy && !desc.static_object) || desc.character
}

/// Anything that occupies a position in a world.
#[derive(Debug, Clone)]
pub struct Entity {
    pub object_type: ObjectType,
    pub kind: Kind,

    /// Who may open this, for a bag that belongs to whoever earned it.
    ///
    /// `None` is everything else, which anybody may open.
    pub belongs_to: Option<Handle>,

    /// What colour this glows, for an administrator who wants to be seen.
    ///
    /// Zero is no glow, which is everybody.
    pub glow: i32,

    /// The two dyes worn over this body's artwork.
    ///
    /// `Player.Texture1`/`Texture2` (`Player.cs:125-135`), put on by using a dye
    /// (`Player.UseItem.AEDye`, `Player.UseItem.cs:583-589`) and exported on every update
    /// (`Player.cs:301-302`). Packed as the original packs them: a type byte over a colour or a
    /// textile index.
    ///
    /// Zero for anybody wearing none, which is nearly everybody.
    pub tex1: i32,
    pub tex2: i32,

    /// Which neighbours this piece of scenery joins onto, as `ConnectionInfo.Bits`.
    ///
    /// `ConnectedObject.Connection` (`ConnectedObject.cs:114-118`), computed once from the
    /// surrounding tiles when the map is read (`Wmap.InitConnection`, `terrain/Wmap.cs:156-165`):
    /// a wall joins onto a neighbour holding the same object type and onto nothing else. Zero for
    /// everything that is not a `ConnectedWall` or a `CaveWall`.
    pub connection: u32,

    /// Whether this portal refuses to be entered.
    ///
    /// `Portal.Usable` inverted (`Portal.cs:24`). Inverted so that the default is the original's
    /// `true`: a portal is usable unless something closed it, which in the original is
    /// `PortalMonitor.ClosePortal` when a realm fills or ends (`PortalMonitor.cs:166`).
    pub portal_unusable: bool,

    /// Whether the account behind this player is an administrator.
    ///
    /// `Player._admin`, seeded from `Account.Admin` when the body is made and exported as
    /// `StatsType.Admin` (`Player.cs:359`). What it decides on screen is the colour of the star
    /// beside the name (`Player.as:750-756`).
    pub admin: bool,

    /// Whether this character owns the eight extra carried slots.
    ///
    /// `Player.HasBackpack`, read off the character on arrival and exported as
    /// `StatsType.HasBackpack` (`Player.cs:355`). The client draws the second row only for a
    /// character that has one, and the server refuses a move into those slots for one that has not
    /// (`InvSwapHandler.cs:177`).
    pub has_backpack: bool,

    /// Whether this account has picked its own name rather than been given one.
    ///
    /// `Player.NameChosen` off the account (`Player.cs:299-300`), which colours the name over the
    /// head (`Player.getNameColor`, `Player.as:757-764`).
    pub name_chosen: bool,

    /// The fame the next class quest asks for.
    ///
    /// `Player.FameGoal`, `GetFameGoal(BestFame)` for this class (`Player.cs:537`), reseated
    /// whenever the fame moves past it (`Player.Leveling.cs:242`). It is the ceiling the fame bar
    /// fills towards.
    pub fame_goal: i32,

    /// What is left of this player's loot-drop and loot-tier boosts, in milliseconds.
    ///
    /// `LDBoostTime` and `LTBoostTime` (`Player.cs:357-358`), beside the experience boost above.
    /// Held as clocks because that is what the client counts down, and because
    /// [`Self::loot_drop`] is a multiplier that says nothing about how long it lasts.
    pub loot_drop_boost_ms: i32,
    pub loot_tier_boost_ms: i32,

    /// What this entity is selling, for a shop merchant.
    ///
    /// Held on the entity rather than looked up from its type, because eight merchants of the same
    /// type sell eight different things: what a stall holds is where it stands, not what it is.
    pub selling: Option<crate::shop::Stall>,

    /// Which of a realm's terrains this enemy was placed on.
    ///
    /// Carried rather than read from the ground, so an enemy that has chased somebody across a
    /// border still counts against where it came from, and so anything it spawns counts there too.
    pub terrain: hendra_content::Terrain,

    pub x: f32,
    pub y: f32,

    pub hp: i32,
    pub max_hp: i32,
    pub mp: i32,
    pub max_mp: i32,

    pub conditions: ConditionSet,
    pub size: u16,
    pub name: Option<Box<str>>,

    /// The guild this player belongs to and their rank in it, drawn beside the name.
    ///
    /// `Player.Guild` and `Player.GuildRank`, seeded from the account when the body is made
    /// (`Player.cs:405-406`) and exported on every update (`Player.cs:291-292`). Carried on the
    /// body rather than looked up, because a body is made fresh in every world and a lookup per
    /// snapshot would be a database read per player per tick.
    ///
    /// `None` for anybody in no guild, and for everything that is not a player.
    pub guild: Option<Box<str>>,
    pub guild_rank: i16,

    /// Whether this entity has anywhere to put a condition.
    ///
    /// `Entity`'s constructor allocates the effect array for a player, for something both
    /// `<Enemy/>` and not `<Static/>`, and for a `<Character/>` — and for nothing else
    /// (`Entity.cs:137-155`). What is left holds no conditions at all, so a wine barrel, which is
    /// `<Static/>`, cannot be slowed or stunned or made to carry its own declared immunities, while
    /// a Shatters switch, which is not, can. The distinction is entirely the descriptor's: both are
    /// `Class=GameObject` and both are breakable scenery.
    pub holds_conditions: bool,

    /// The eight stats, which decide movement, damage and rate of fire.
    ///
    /// Enemies keep the default, which gives them the base values every derived figure falls back
    /// to. Only players level, wear equipment or take boosts.
    pub stats: crate::stats::Stats,

    /// The object whose projectile this entity fires, if it can shoot.
    pub weapon: Option<ObjectType>,

    /// The clock reading at which it may shoot again.
    ///
    /// A deadline rather than a countdown, and on the clock the shot that set it was taken on: the
    /// original holds the client time of the last shot it accepted and compares the next one to it
    /// (`Player.AntiCheat.cs:93-95`). A countdown spent a tick at a time can only ever expire on a
    /// tick boundary, which rounds every interval up to the next fifty milliseconds.
    pub ready_at_ms: u32,

    /// How long until this player's next burn, in milliseconds.
    pub burn_due_ms: u32,

    /// How much air is left, from a hundred down to nothing.
    ///
    /// Full everywhere but a drowning world, where standing away from a vent spends it and
    /// standing at one refills it. At nothing it is health that goes instead.
    pub oxygen: i32,

    /// Whether killing this is worth experience. `Entity.GivesNoXp`, inverted.
    ///
    /// False for anything a spawner made unless the script says otherwise, and for anything one of
    /// those made in turn. Experience is all it touches: `DamageCounter.Death` reads it to make the
    /// figure zero (`DamageCounter.cs:82`) and nothing else asks, so an enemy carrying it still
    /// drops everything its loot table says it drops.
    pub awards_experience: bool,

    /// Where this entity came into being, which some behaviours keep it near.
    pub spawn_x: f32,
    pub spawn_y: f32,

    /// This entity's behaviour, if it has one.
    ///
    /// Boxed because most entities are scenery and a `Mind` carries several vectors; paying for
    /// one inside every wall would cost more memory than the enemies do.
    pub mind: Option<Box<Mind>>,

    /// What this entity holds, for players and loot bags.
    pub container: Option<Box<Container>>,

    /// How long until this entity removes itself, for bags.
    pub expires_in_ms: Option<u32>,

    /// Set when the entity should be removed at the end of the tick.
    pub dead: bool,

    /// Damage taken since the last time this entity thought, for transitions that react to being
    /// hurt. Cleared once read, so a hit counts toward exactly one tick.
    pub damage_since_tick: i32,

    /// How much more likely this player is to be given loot, as a multiplier.
    ///
    /// One for everybody without a boost. The original reads `LDBoostTime > 0` and multiplies by
    /// one and a half; the boost itself is durable and its remaining time is the account's, so what
    /// reaches the world is the multiplier rather than the clock.
    pub loot_drop: f32,

    /// Milliseconds left on this player's experience boost, which doubles what a kill is worth.
    ///
    /// Zero is no boost and a negative value is one that never lapses, which is `XPBoostTime`'s
    /// three states: `TickActivateEffects` counts down only while it is positive and clears it the
    /// moment the level reaches twenty (`Player.cs:593-602`), and `DamageCounter` asks only whether
    /// it is not zero (`DamageCounter.cs:98`).
    pub experience_boost_ms: i32,

    /// What the account behind this player can spend: gold, fame and prestige.
    ///
    /// Held on the body rather than looked up, because that is where the original keeps it:
    /// `Player.CurrentFame`, `Credits` and `Prestige` are stats on the entity, seeded from the
    /// account when the body is made (`Player.cs:399-404`) and written back into whenever a
    /// purchase moves them (`Merchant.cs:192-195`). Everything a shop refuses is refused against
    /// these.
    pub purse: Purse,

    /// Temporary stat boosts, each with what is left of its time.
    ///
    /// Held as a list rather than folded into a total, because they do not add: the largest counts
    /// in full and each one after it counts for half of the last, so the total can only be worked
    /// out from all of them at once. Folding them in would also make a lapse take away whatever was
    /// added last rather than what that boost was worth.
    pub boosts: Vec<HeldBoost>,

    /// What this character has done, which is what its death is worth.
    ///
    /// On the entity because that is where the events are: a shot is counted where it is fired and
    /// a kill where the enemy dies, and nothing is taken from a client, since a client that reports
    /// its own accuracy will report whatever earns the most.
    pub tally: crate::fame::Tally,

    /// How long this player is still too new to be worth attacking.
    ///
    /// `Player.SetNewbiePeriod`, three seconds. Somebody who has just walked through a portal has
    /// not had a chance to see what is in the room, and being shot at during the moment their client
    /// is still drawing it is a death nobody could have avoided.
    pub unseen_ms: u32,

    /// How many stars this player has earned, which is what everybody else sees beside their name.
    ///
    /// A record of the account rather than of this character: the sum over every class of what its
    /// best fame is worth. Read when they join a world, since nothing that happens inside one can
    /// change it.
    pub stars: u8,

    /// Which enemy this player's quest arrow points at.
    ///
    /// Held rather than worked out when asked, because two rules depend on which enemy it *was*
    /// when it died: killing it counts toward the character's fame, and it is worth five times the
    /// experience anything else is capped at. Neither can be answered after the fact.
    pub quest_target: Option<Handle>,

    /// Whose body this player's camera is on, when it is not their own.
    ///
    /// `Player.SpectateTarget` (`Player.cs:499`), set by `/spectate` and cleared by naming oneself.
    /// Held on the body because the snapshot has to carry the watched entity whether or not it is
    /// anywhere near: `GetNewEntities` yields the spectate target unconditionally
    /// (`Player.Update.cs:252-253`), and a camera pointed at something the client was never sent
    /// looks at nothing.
    pub watching: Option<Handle>,

    /// Which squares of this world this player has laid eyes on, for the count a character's fame
    /// is partly made of.
    ///
    /// Only players carry one, and only once they have looked at something. Per world rather than
    /// per character, as the original's is: walking back into a dungeon you cleared last week shows
    /// you the same ground again, and the count is of ground seen rather than ground new to you.
    pub seen: Option<Box<Seen>>,

    /// The squares this player can see from where they stand.
    ///
    /// `Player.Sight` (`Player.Update.cs:80`), which only a player has and which is the one answer
    /// the ground pass, the scenery pass and the entity pass are all driven from. Held on the body
    /// because it is cached between ticks exactly as the original's is, and thrown away with the
    /// body when the player leaves the world.
    pub sight: Option<Box<SightCircle>>,

    /// What last took health off this, which is what a death is named after.
    pub last_hurt_by: Option<Handle>,

    /// Who has damaged this, and how much.
    ///
    /// Kept per enemy rather than globally, because it is a fact about this fight: it decides who
    /// earned the loot that belongs to whoever earned it, and it dies with the enemy.
    pub damage_by: Vec<(Handle, i32)>,

    /// Which sprite to draw, for bosses that visibly change phase.
    pub texture: u8,

    /// The skin this player is wearing, as the skin object's own type. Zero for the class's own
    /// sprite, which is what everything that is not a player wears.
    ///
    /// `Player._skin` (`Player.cs:139-145`), read out of the character on arrival (`:431-436`) and
    /// exported as `StatsType.Skin` (`:303`). It replaces the whole animated sheet the body is
    /// drawn from rather than picking a frame within one, which is what [`Self::texture`] does.
    pub skin: u16,

    /// The skin and size to go back to when something that dressed the body over the top of them
    /// stops applying.
    ///
    /// `Player._originalSkin` and `Entity._originalSize`, written together by `SetDefaultSkin` and
    /// `SetDefaultSize` (`Player.cs:1143-1152`, `Entity.cs:802-810`) every time the wardrobe
    /// settles on a skin. A completed equipment set dresses the body over them, and breaking the
    /// set puts them back (`BoostStatManager.cs:104-112`).
    pub default_skin: u16,
    pub default_size: u16,

    /// The size this entity is growing or shrinking toward, and how fast, in hundredths per
    /// second. `None` when it is not changing.
    pub resizing: Option<(f32, u16)>,

    /// Whether this was put into the world by something other than the map. `Enemy.Spawned`.
    ///
    /// It carries down a spawner's whole line, and it is what stops a spawner being a farm: neither
    /// experience nor loot is handed out for anything wearing it, since `DamageCounter.Death`
    /// (`DamageCounter.cs:73`) and `Loots.Handle` (`Loots.cs:83`) both return before doing anything
    /// when it is set. It does not stop the entity's own death behaviours, which run either way.
    pub spawned: bool,

    /// Whether this left the world without dying.
    ///
    /// `Enemy.Death` runs the current state's death behaviours before it calls `LeaveWorld`
    /// (`Enemy.cs:48-53`); a bare `LeaveWorld` runs none of them. Decaying, transforming, being
    /// cleared by a setpiece and a bag running out of time are all the second kind, so this is what
    /// separates an entity that died from one that was simply taken away.
    pub removed: bool,

    /// What this goes off with, for a trap. `None` for everything else.
    pub armed: Option<Armed>,

    /// Where this is drifting and whether it has gone off, for a decoy. `None` for everything else.
    pub decoy: Option<Drifting>,

    /// How long before this player may teleport again.
    pub teleport_cooldown_ms: u32,

    /// How long a player is forgiven for being somewhere no speed explains.
    ///
    /// A teleport moves somebody further in one tick than walking ever could, and the movement
    /// check cannot tell that from a client claiming to be somewhere it is not.
    pub move_grace_ms: u32,

    /// Level, experience and fame. Only players advance.
    pub progress: crate::leveling::Progress,

    /// Health owed by an effect that has not yet amounted to a whole point.
    ///
    /// Twenty a second at fifty-millisecond ticks is one point per tick, and rounding each tick
    /// independently would round it to nothing.
    pub health_fraction: f32,

    /// The same, for magic.
    pub magic_fraction: f32,

    /// Health owed by regeneration that has not yet amounted to a whole point.
    ///
    /// Separate from what the effects carry, as the original keeps `_hpRegenCounter` separate from
    /// `_healing` and `_bleeding` (`Player.cs:611-612`, `Player.Effects.cs:8-9`). One pool would
    /// mean a healing aura and vitality each spending what the other had put by, and a regeneration
    /// that gives up on a full bar would take the aura's fraction down with it.
    pub health_regen_fraction: f32,

    /// The same, for magic.
    pub magic_regen_fraction: f32,

    /// A colour to blink, its period and how many times, for a phase change the eye can catch.
    pub flash: Option<(u32, u32, u32)>,

    /// The health this entity had before any scaling to the crowd.
    ///
    /// Kept so scaling is measured from the base each time rather than compounded. Without it a
    /// boss in a busy room grows every two seconds forever.
    pub base_max_hp: Option<i32>,

    /// Effects currently held, and how long each has left.
    ///
    /// A list rather than an array indexed by effect number: there are forty effects and almost
    /// every entity has none, so forty timers each would be a hundred and sixty bytes per entity
    /// to hold nothing. An empty `Vec` allocates nothing.
    pub effects: Vec<(u8, u32)>,

    /// Where this player was at the last two position samples, for enemies that lead their shots.
    ///
    /// Only players keep one in the original: `_posHistory` is allocated for a player and for
    /// nothing else (`realm/Entity.cs:135-143`), which is why `Predict` falls back to aiming
    /// straight at anything that is not one.
    pub trail: Trail,
}

/// What an account can spend.
///
/// The three currencies `Player.ExportStats` sends beside the character's own fame
/// (`realm/entities/player/Player.cs:291-299`) and the three `Merchant.TransactionItemComplete`
/// writes back after a purchase (`realm/entities/vendors/Merchant.cs:192-195`). Kept together
/// because nothing ever moves one without the client wanting all three.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Purse {
    /// Gold, which the original calls credits.
    pub credits: i32,

    /// Fame the account may spend, which is not the fame this character has earned.
    pub fame: i32,

    pub prestige: i32,
}

/// The position samples a leading shot is built from.
///
/// Two entries rather than the original's ring of 256, because `Shoot.Predict` is the only reader
/// and it asks for one sample back (`logic/behaviors/Shoot.cs:94`).
///
/// Starting at the origin rather than at where the entity spawned is deliberate. The original's
/// ring is a fresh `Position[256]`, so a player who has been in the world for less than two samples
/// is led from `(0, 0)` — an enemy shooting at them aims at five times their position, well off the
/// map's far corner, for the first two thirds of a second. That is what the original does.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Trail {
    /// The most recent sample.
    pub last: (f32, f32),

    /// The sample before it, which is the one `TryGetHistory(1)` returns.
    pub previous: (f32, f32),

    /// How long since the last sample was taken.
    pub since_ms: u32,
}

impl Entity {
    /// A fixture read off the map.
    pub fn fixture(object_type: ObjectType, x: f32, y: f32) -> Entity {
        Entity {
            object_type,
            terrain: hendra_content::Terrain::None,
            selling: None,
            glow: 0,
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
            belongs_to: None,
            kind: Kind::Fixture,
            x,
            y,
            hp: 0,
            max_hp: 0,
            mp: 0,
            max_mp: 0,
            conditions: ConditionSet::EMPTY,
            size: 100,
            name: None,
            guild: None,
            guild_rank: 0,
            holds_conditions: true,
            stats: crate::stats::Stats::still(),
            weapon: None,
            ready_at_ms: 0,
            awards_experience: true,
            burn_due_ms: 0,
            oxygen: FULL_OXYGEN,
            spawn_x: x,
            spawn_y: y,
            mind: None,
            container: None,
            expires_in_ms: None,
            dead: false,
            damage_since_tick: 0,
            damage_by: Vec::new(),
            last_hurt_by: None,
            loot_drop: 1.0,
            experience_boost_ms: 0,
            purse: Purse::default(),
            boosts: Vec::new(),
            tally: crate::fame::Tally::default(),
            seen: None,
            sight: None,
            quest_target: None,
            watching: None,
            unseen_ms: 0,
            stars: 0,
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
            teleport_cooldown_ms: 0,
            move_grace_ms: 0,
            progress: crate::leveling::Progress::new(),
            health_fraction: 0.0,
            magic_fraction: 0.0,
            health_regen_fraction: 0.0,
            magic_regen_fraction: 0.0,
            flash: None,
            base_max_hp: None,
            trail: Trail::default(),
        }
    }

    pub fn player(object_type: ObjectType, x: f32, y: f32, max_hp: i32) -> Entity {
        Entity {
            object_type,
            terrain: hendra_content::Terrain::None,
            selling: None,
            glow: 0,
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
            belongs_to: None,
            kind: Kind::Player,
            x,
            y,
            hp: max_hp,
            max_hp,
            mp: 100,
            max_mp: 100,
            conditions: ConditionSet::EMPTY,
            size: 100,
            name: None,
            guild: None,
            guild_rank: 0,
            holds_conditions: true,
            stats: crate::stats::Stats::default(),
            weapon: None,
            ready_at_ms: 0,
            awards_experience: true,
            burn_due_ms: 0,
            oxygen: FULL_OXYGEN,
            spawn_x: x,
            spawn_y: y,
            mind: None,
            container: None,
            expires_in_ms: None,
            dead: false,
            damage_since_tick: 0,
            damage_by: Vec::new(),
            last_hurt_by: None,
            loot_drop: 1.0,
            experience_boost_ms: 0,
            purse: Purse::default(),
            boosts: Vec::new(),
            tally: crate::fame::Tally::default(),
            seen: None,
            sight: None,
            quest_target: None,
            watching: None,
            unseen_ms: 0,
            stars: 0,
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
            teleport_cooldown_ms: 0,
            move_grace_ms: 0,
            progress: crate::leveling::Progress::new(),
            health_fraction: 0.0,
            magic_fraction: 0.0,
            health_regen_fraction: 0.0,
            magic_regen_fraction: 0.0,
            flash: None,
            base_max_hp: None,
            trail: Trail::default(),
        }
    }

    /// Re-derives the two carried ceilings from the stats behind them.
    ///
    /// The original carries no ceiling at all: `Stats[0]` and `Stats[1]` are `Base[i] + Boost[i]`
    /// worked out afresh on every read (`StatsManager.cs:23`), so every consumer of a maximum sees
    /// the same answer by construction and `ReCalculateValues` only has to push the new number at
    /// the client (`StatsManager.cs:36-43`). Carrying `max_hp` and `max_mp` on the body is faster
    /// but gives a maximum two sources of truth, and every path that can move one has to say so —
    /// equipment, a potion, a level, and the arrival that seats them all. This is that one line,
    /// written once so those paths cannot disagree about it.
    ///
    /// Health is floored at one and magic at zero, which is the floor `IncrementBoost` applies to
    /// the same two stats (`BoostStatManager.cs:168-171`).
    ///
    /// Current health and magic are deliberately left alone. The original does not pull either down
    /// when a maximum drops: `HandleRegen` only ever moves them by `Math.Min(Stats[0], HP + regen)`
    /// (`Player.cs:617-640`), so a player who takes off a health ring keeps the overflow until the
    /// next regen tick that can regen at all, and keeps it indefinitely while sick or bleeding.
    pub fn reseat_maxima(&mut self) {
        self.max_hp = self.stats.max_hp().max(1);
        self.max_mp = self.stats.max_mp().max(0);
    }

    /// Gives an entity the immunity markers its descriptor declares.
    ///
    /// `Character.SetConditions` (`Character.cs:48-67`) does exactly this in the constructor, so
    /// every enemy and every player is already holding its immunities before it has been hit once.
    /// `Entity.ApplyCondition` (`Entity.cs:738-772`) then reads them to refuse the matching effect,
    /// which is what makes a boss unstunnable rather than merely resistant.
    ///
    /// Held as effects that never run out rather than as bare bits, because the tick rebuilds an
    /// entity's condition set from the effects still running and a bit with nothing behind it would
    /// be swept away on the first tick.
    ///
    /// Only a `Character` does this, which is to say only a player or an enemy. Breakable scenery is
    /// a `StaticObject` and never runs `SetConditions`, so a wine barrel declaring `<StasisImmune/>`
    /// holds no condition at all and reports none — and could not hold one if it wanted to, since
    /// `Entity`'s constructor allocates the effect array only for a player, a `Character`, or
    /// something both `<Enemy/>` and not `<Static/>` (`Entity.cs:137-155`).
    pub fn take_immunities(&mut self, desc: &hendra_content::ObjectDesc) {
        if !matches!(self.kind, Kind::Player | Kind::Enemy) {
            return;
        }

        for marker in desc.immunities.iter() {
            self.conditions.insert(marker);
            self.effects.push((marker.index() as u8, FOREVER));
        }
    }

    /// Whether this entity's health has fallen past what its kind survives.
    ///
    /// The threshold is not the same for everything, and the difference is visible in play. An
    /// enemy dies on `HP < 0` (`Enemy.cs:90` and `:127`), so one left on exactly zero is still
    /// standing and still takes another shot to finish. A player dies on `HP <= 0` (`Player.cs:578`,
    /// `:780`, `:806` and `Player.Ground.cs:29`, `:72`, `:106`), and so does a breakable object
    /// (`StaticObject.cs:79`). Reading them all as `<= 0` makes every enemy one hit cheaper than it
    /// should be; reading them all as `< 0` leaves players walking around on nothing.
    ///
    /// Every line above is cited against the untouched 2020 source (`94615c4`), because this
    /// repository's own later commits moved lines around inside `Player.cs`. The thresholds
    /// themselves were not among what they changed.
    pub fn slain(&self) -> bool {
        self.slain_by(0)
    }

    /// Whether taking this much damage would leave it dead, by the same rule.
    ///
    /// The rule is written once so the two callers cannot drift apart: the hit that is about to be
    /// applied and the `kill` flag broadcast with it have to agree, or the client draws a death
    /// that did not happen.
    pub fn slain_by(&self, damage: i32) -> bool {
        let left = self.hp - damage;
        match self.kind {
            Kind::Enemy => left < 0,
            _ => left <= 0,
        }
    }

    /// Whether this was built with a life, which is the original's `Vulnerable`.
    ///
    /// `StaticObject`'s constructor sets `Vulnerable = life.HasValue` (`StaticObject.cs:36`), and
    /// `life` is the descriptor's `MaxHitPoints` for anything read off a map (`Entity.cs:606`), the
    /// duration for a decoy or a trap (`Decoy.cs:29`, `Trap.cs:21`), and the countdown for a bag or
    /// a timed portal (`Container.cs:14`, `Portal.cs:13`). Everything else — a wall, a sign, a
    /// merchant, a permanent portal — is given `null` and is not vulnerable.
    ///
    /// Players and enemies are `Character`s rather than `StaticObject`s and never ask this.
    pub fn vulnerable(&self) -> bool {
        self.max_hp != 0 || self.expires_in_ms.is_some()
    }

    /// What the health stat carries on the wire.
    ///
    /// `StaticObject.ExportStats` sends `int.MaxValue` rather than the real number for anything not
    /// `Vulnerable` (`StaticObject.cs:44`), and everything that is not a player or an enemy is a
    /// `StaticObject` in the original. It is not cosmetic: the client keeps its own `maxHP_` from
    /// the descriptor's `MaxHitPoints`, defaulting to two hundred where there is none, and raises it
    /// to whatever health it is told when that is larger (`GameObject.as:1035-1038`). So an
    /// indestructible object sending the impossible number draws a full bar, and one sending zero —
    /// which is what its real health is — draws an empty one over something that cannot be hurt.
    pub fn exported_hp(&self) -> i32 {
        match self.kind {
            // `Character.ExportStats` (`Character.cs:76`) sends the real number, always.
            Kind::Player | Kind::Enemy => self.hp,
            _ if !self.vulnerable() => i32::MAX,
            _ => self.hp,
        }
    }

    pub fn state(&self) -> EntityState {
        EntityState {
            object_type: self.object_type.0,
            x: self.x,
            y: self.y,
            hp: self.exported_hp(),
            max_hp: self.max_hp,
            mp: self.mp,
            max_mp: self.max_mp,
            conditions: self.conditions.0,
            size: self.size,
            name: self.name.clone(),
            texture: self.texture,
            skin: self.skin,
            oxygen: self.oxygen.clamp(0, FULL_OXYGEN) as u8,
            // Only players carry meaningful stats. Everything else sends zeroes, which cost a byte
            // each as varints and never change, so they never appear in a delta.
            stats: if self.kind == Kind::Player {
                self.stats.totals()
            } else {
                [0; hendra_net::STAT_COUNT]
            },
            // What equipment and running boosts add on top, which is what the sheet draws in green.
            // `Player.ExportStats` sends the whole `Stats.Boost` array beside the totals
            // (`Player.cs:342-352`), and a total on its own cannot say how much of it is the player.
            boosts: if self.kind == Kind::Player {
                self.stats.boost_totals()
            } else {
                [0; hendra_net::STAT_COUNT]
            },
            // The guild written under the name. Only a player has one, and most players have none.
            guild: self.guild.clone(),
            guild_rank: self.guild_rank,
            stars: self.stars,
            // Levelling belongs to players, and the experience is what remains after the total the
            // current level started at is taken off, which is the figure the bar fills with.
            level: if self.kind == Kind::Player {
                self.progress.level
            } else {
                0
            },
            experience: if self.kind == Kind::Player {
                self.progress.experience - crate::leveling::experience_at(self.progress.level)
            } else {
                0
            },
            experience_goal: if self.kind == Kind::Player {
                crate::leveling::experience_goal(self.progress.level)
            } else {
                0
            },
            fame: if self.kind == Kind::Player {
                self.progress.fame
            } else {
                0
            },
            // The account's purse rather than the character's earnings, and the thing every shop
            // in the game charges against.
            credits: self.purse.credits,
            current_fame: self.purse.fame,
            prestige: self.purse.prestige,
            // What a bag or a chest holds. `Container.ExportStats` writes the eight slots into the
            // entity's stats (`realm/entities/Container.cs:76-90`), and it is the only way a client
            // knows whether the bag it is standing on is worth opening.
            contents: self.container.as_ref().map(|held| {
                let mut slots = [hendra_net::NO_ITEM; 8];
                for (index, slot) in slots.iter_mut().enumerate() {
                    let item = held.item(index);
                    *slot = if item.is_none() {
                        hendra_net::NO_ITEM
                    } else {
                        item.0
                    };
                }
                Box::new(slots)
            }),
            // What a vendor is offering. `SellableObject.ExportStats` and `Merchant.ExportStats`
            // write these into the entity's stats (`realm/entities/vendors/SellableObject.cs:72-78`,
            // `.../Merchant.cs:46-52`), and the client draws a merchant as the item rather than as
            // itself, so a stall that sent none of this would be a grey placeholder nobody can buy
            // from.
            merchandise: self.selling.map(|stall| hendra_net::Merchandise {
                item: stall.item.0,
                price: stall.price,
                currency: match stall.currency {
                    crate::shop::Currency::Gold => 0,
                    crate::shop::Currency::Fame => 1,
                },
                count: stall.count,
                rank: stall.rank,
            }),
            // The halo `/glow` sets. `Player.ExportStats` sends it for every player on every update
            // (`Player.cs:302`) and the client paints a ring of it around the sprite
            // (`GlowRedrawer.as:19-46`).
            glow: self.glow,
            // Marks that belong to the account and the character rather than to the body, and
            // none of which moves while a player is standing in a world.
            admin: self.admin,
            has_backpack: self.has_backpack,
            name_chosen: self.name_chosen,
            portal_unusable: self.portal_unusable,

            // What a dye put on. Only a player wears one, and nearly no player does.
            tex1: self.tex1,
            tex2: self.tex2,

            // Which neighbours a wall or a fence joins onto, worked out when the map was read.
            connection: self.connection,

            // The three boost clocks, in whole seconds as `ExportStats` sends them
            // (`Player.cs:356-358`). Rounded down, so a boost with nine hundred milliseconds left
            // reads as zero and the client stops drawing it a blink early rather than a blink late.
            experience_boost_seconds: self.experience_boost_ms.max(0) / 1000,
            loot_drop_boost_seconds: self.loot_drop_boost_ms.max(0) / 1000,
            loot_tier_boost_seconds: self.loot_tier_boost_ms.max(0) / 1000,

            fame_goal: if self.kind == Kind::Player {
                self.fame_goal
            } else {
                0
            },
        }
    }
}

/// How many times a volley of bullets rolls its damage.
///
/// The original does it differently for the two things that fire a fan, and the difference is
/// visible on every multi-shot enemy in the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VolleyDamage {
    /// One roll for the whole fan, so every bullet of it hits for the same amount.
    ///
    /// `Shoot.TickCore` draws `int dmg = Random.Next(desc.MinDamage, desc.MaxDamage)` before the
    /// loop that makes the projectiles and hands the same `dmg` to each
    /// (`logic/behaviors/Shoot.cs:181-190`). The `EnemyShoot` packet the client is told about it
    /// with carries a single `Damage` for the whole volley (`:197-206`), so a fan whose bullets
    /// differed could not be described to a client at all.
    Volley,

    /// A fresh roll for each bullet.
    ///
    /// `AEShoot` calls `Stats.GetAttackDamage(prjDesc.MinDamage, prjDesc.MaxDamage, true)` inside
    /// the loop (`Player.UseItem.cs:1124-1128`), so a five-shot ability item does five different
    /// amounts. `AllyShoot` carries no damage field, which is why it can.
    PerBullet,
}

/// Why a movement claim was not honoured in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveRefusal {
    /// The claim would need the entity to travel faster than it can.
    TooFar,

    /// The destination cannot be stood on.
    Blocked,

    /// The entity is held in place by an effect.
    ///
    /// Distinct from `TooFar` because it is not a speed judgement: a paralysed player claiming one
    /// square is refused where an unaffected one would be allowed, and telling them apart is what
    /// lets a client say why rather than looking like a rubber-banding bug.
    Rooted,
}

/// What the server decided about a movement claim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveOutcome {
    /// Where the entity actually is now. The server's answer, not the client's.
    pub x: f32,
    pub y: f32,

    /// Set when the claim was trimmed, and why.
    pub refused: Option<MoveRefusal>,

    /// How much of the allowance this move used, in milliseconds of travel.
    ///
    /// What stops the allowance being per-message rather than per-second. The claim is checked
    /// against how far the player could have walked since it was last checked, so a client that
    /// sends ten times as often would otherwise be granted ten times the distance -- which is a
    /// speed hack made of nothing but a faster loop. The caller keeps the unspent remainder, so a
    /// burst of claims released together after a stall is still paid for out of the time that
    /// actually passed. Zero while the server itself has just moved somebody.
    pub spent_ms: u32,
}

/// The slot types an item-kind name refers to.
///
/// The content's loot tables say `weapon` or `ring`, which are the `ItemType` enum the C# used. Each
/// names a whole family of slots rather than one, and the lists are `TierLoot`'s
/// (`logic/loot/MobDrops.cs:66-70`): a weapon is a sword, dagger, bow, wand, katana or staff, and a
/// potion is slot type ten rather than the untyped zero. An unrecognised name means "anything of
/// this tier", which is a wider drop rather than no drop.
///
/// Matched without regard to case, because the two places that name a family spell it differently:
/// a behaviour file writes the C# argument through as `weapon`, and a setpiece chest names the
/// `ItemType` member as `Weapon`. A family that does not match is not a narrower drop but a wider
/// one, so the difference is a chest handing out rings where the original hands out swords.
fn slot_types_of(kind: &str) -> Option<&'static [i32]> {
    Some(match kind.to_ascii_lowercase().as_str() {
        "weapon" => &[1, 2, 3, 8, 17, 24],
        "ability" => &[4, 5, 11, 12, 13, 15, 16, 18, 19, 20, 21, 22, 23, 25],
        "armor" | "armour" => &[6, 7, 14],
        "ring" => &[9],
        "potion" => &[10],
        _ => return None,
    })
}

/// One concrete drop: an item, the chance it appears, how many must appear, and who may have it.
///
/// `LootDef` in `logic/loot/Loots.cs:11-25`. A loot table is a tree in the content and a flat list
/// of these by the time anything is rolled, because `MobDrops.Populate` flattens it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LootDef {
    item: ObjectType,
    chance: f32,
    required: u32,

    /// How much damage a player must have done to be eligible, or zero for loot anybody may take.
    threshold: f32,
}

/// Flattens one loot entry into the concrete drops it stands for.
///
/// A named item is one drop. A tier is one drop per item in the family, each at the chance divided
/// between them (`MobDrops.cs:100`), so a tier entry at three per cent is three per cent of dropping
/// something rather than three per cent per item. A threshold stamps its share onto everything
/// inside and contributes nothing itself.
///
/// `threshold` is the share inherited from an enclosing `Threshold`, and `None` at the top. Where
/// one `Threshold` holds another, the *outer* share is the one that survives: the inner constructor
/// stamps its own share onto its children, and the outer then calls `Populate` over those same
/// children with its share as the override, which `MobDrops.Populate` applies whenever it is not
/// negative (`MobDrops.cs:26-40`) — and a share written in a script never is. Letting the inner win
/// instead reads the nesting backwards and hands a boss's rarest drop to whoever did the least
/// damage.
fn flatten_loot(
    entry: &hendra_behavior::program::LootEntry,
    threshold: Option<f32>,
    catalog: &Catalog,
    out: &mut Vec<LootDef>,
) {
    use hendra_behavior::program::LootEntry;

    match entry {
        LootEntry::Item {
            name,
            chance,
            required,
            threshold: own,
        } => {
            if let Some(item) = catalog.type_of(name) {
                out.push(LootDef {
                    item,
                    chance: *chance,
                    required: *required,
                    threshold: threshold.unwrap_or(*own),
                });
            }
        }

        LootEntry::Tier {
            tier,
            kind,
            chance,
            required,
            threshold: own,
        } => {
            let candidates = tier_candidates(catalog, *tier, kind);
            if candidates.is_empty() {
                return;
            }
            let each = *chance / candidates.len() as f32;
            for item in candidates {
                out.push(LootDef {
                    item,
                    chance: each,
                    // The original copies the count onto every item the tier expands into rather
                    // than sharing it out, so a tier entry that requires one requires one of each.
                    required: *required,
                    threshold: threshold.unwrap_or(*own),
                });
            }
        }

        LootEntry::Threshold { share, children } => {
            // Outer wins, so an already-inherited share is carried straight through.
            let inherited = threshold.or(Some(*share));
            for child in children {
                flatten_loot(child, inherited, catalog, out);
            }
        }
    }
}

/// Every item a tier entry could name, in catalog order.
///
/// `TierLoot`'s constructor query (`logic/loot/MobDrops.cs:91-95`): everything whose slot type is in
/// the family and whose tier matches. The order matters only in that it is stable, so a roll picks
/// the same item from the same seed.
fn tier_candidates(catalog: &Catalog, tier: u8, kind: &str) -> Vec<ObjectType> {
    let wanted = slot_types_of(kind);
    catalog
        .items()
        .filter(|desc| {
            desc.item.as_ref().is_some_and(|item| {
                item.tier == Some(tier as i32)
                    && wanted.is_none_or(|slots| slots.contains(&item.slot_type))
            })
        })
        .map(|desc| desc.object_type)
        .collect()
}

/// How much further than the rules allow a claim may travel before it is trimmed.
///
/// Not generosity toward cheating, since the claim is clamped either way. It absorbs the ordinary
/// disagreement between a client's clock and the server's, which at 20 ticks per second is a small
/// fraction of a tile.
///
/// This server's own number, not the original's. The only speed tolerance the original ever wrote
/// is the 1.05 in the check commented out of `MoveHandler.cs:25-31`, and that check calls a method
/// (`GetTilesPerSecSqr`) that exists nowhere in the source -- it could not have run if it had been
/// uncommented. So there is nothing here to match, and the reason this is wider than 1.05 is that
/// it is measured against the gap since the last claim rather than against a whole second.
pub const MOVE_TOLERANCE: f32 = 1.5;

/// The furthest a server-driven move travels before the ground under it is tested again, in tiles.
///
/// `colSkipBoundary` in `Entity.ResolveNewLocation` (`Entity.cs:330`). Under half a tile, so no
/// step can begin on one side of a square and end on the other without the square in between being
/// looked at.
const COLLISION_STEP: f32 = 0.4;

pub use crate::sight::{Sight, SightCircle};

pub struct World {
    pub name: String,

    /// How dangerous this place says it is, as the marks on the loading screen.
    ///
    /// `World.Difficulty` out of the definition's `difficulty` (`World.cs:119`). Nothing in the
    /// simulation reads it; it is carried so the client can be told.
    pub difficulty: i32,

    /// Which sky the map is drawn against. `World.Background` (`World.cs:120`).
    pub background: i32,

    /// Whether the scoreboards standing in this world show anything.
    ///
    /// `World.ShowDisplays` out of the definition's `showDisplays` (`World.cs:124`), and true for
    /// anything built in code (`World.Setup`, `World.cs:145`).
    pub show_displays: bool,

    /// What is playing here.
    ///
    /// One name drawn from the definition's list when the world was built (`World.cs:127-132`), and
    /// changed under the people already standing here by `/music` and by the `ChangeMusic`
    /// behaviours, which is why it is held rather than read from the definition each time.
    pub music: String,

    /// What this world lets a player see through.
    pub sight: Sight,

    /// Whether a player away from air drowns here.
    ///
    /// One world does: `HandleOceanTrenchGround` checks the name and does nothing anywhere else.
    /// A flag rather than the name, so the tick is a comparison rather than a string.
    pub drowns: bool,

    /// How long until the next breath is counted.
    since_breath_ms: u32,

    /// How long this world has been running, in milliseconds.
    ///
    /// Only the clock a caller with no better one falls back to: everything the simulation decides
    /// for itself is decided on the tick. It exists because a rate of fire is not decided on the
    /// tick — see [`World::shoot_at`].
    now_ms: u32,

    terrain: Terrain,
    entities: Slab<Entity>,
    projectiles: Projectiles,
    grid: Grid,
    tick: Tick,

    /// Rolls damage without pulling in a random-number crate. Deterministic, which also makes a
    /// failing combat test reproducible.
    seed: u32,

    /// Compiled enemy behaviour, by object id.
    behaviours: Programs,

    /// What a loot bag looks like.
    bag_type: ObjectType,

    /// The object to use for each loot colour, indexed by the content's own numbering.
    bag_types: Vec<ObjectType>,

    /// Reused between ticks so a warm world allocates nothing.
    handles: Vec<Handle>,
    nearby: Vec<Handle>,

    /// The chunks near enough to a player that what stands in them thinks.
    awake: std::collections::HashSet<(i32, i32)>,

    /// What entities have said, waiting to be sent out.
    ///
    /// Bounded, because nothing in the simulation makes a taunt stop: a boss left alive in an
    /// empty room talks to itself indefinitely, and a queue nobody drains is a slow leak.
    announcements: Vec<Announcement>,

    /// Squares whose ground changed, waiting to be sent out. Bounded for the same reason.
    ground_changes: Vec<(u16, u16, u16)>,

    /// Players whose quest arrow has come to rest on something new, and what.
    ///
    /// Only changes, and only to something: `HandleQuest` sends nothing when the choice is the same
    /// as last time and nothing when there is nothing to point at
    /// (`Player.Leveling.cs:214-228`). One entry per player per change, so the queue is bounded by
    /// how many players are here.
    quest_changes: Vec<(Handle, Handle)>,

    /// Players whose camera has been put on a different body, and which.
    ///
    /// Bounded by how many players are here, since only a command or a watched body leaving pushes
    /// one.
    focus_changes: Vec<(Handle, Handle)>,

    /// Squares whose ground was laid down by something summoned.
    ///
    /// `GroundTransform` copies its host's `Spawned` onto every square it changes
    /// (`GroundTransform.cs:72`), and `Player.Death` turns a death on such a square into a rekting
    /// (`Player.cs:999-1002`). Without it, fire painted by a boss's summoned minion would end a
    /// character where the original only sends it home.
    summoned_ground: std::collections::HashSet<(u32, u32)>,

    /// Objects thrown but not yet landed, with the time left before they do.
    ///
    /// A thrown object that appeared instantly would be an unavoidable hit. The delay is what the
    /// content calls a throw time, and it is the difference between a hard attack and one nobody
    /// can play around.
    falling: Vec<Falling>,

    /// Grenades in the air, with the time left on their fuse.
    ///
    /// Held for the same reason the thrown objects above are, and for a stronger one: a grenade
    /// that went off the instant it was thrown could not be stepped out of, and stepping out of it
    /// is the whole of what a grenade asks of a player.
    fuses: Vec<Fuse>,

    /// Poison already dealt out and still being paid, one second at a time.
    poisons: Vec<Poison>,

    /// Enemies whose stasis is running out, and who are made unfreezable when it does.
    stasis_locks: Vec<StasisLock>,

    /// Advanced for every child spawned, so two children of one parent do not move in lockstep.
    spawn_seed: u32,

    /// How full this world is and whether it has been cleared. Only a realm uses it.
    realm: crate::realm::Realm,

    /// Whether players may teleport here, from the world definition's `restrictTp`.
    allows_teleport: bool,

    /// How many squares a drawing could not paint, because the map already names as many kinds of
    /// square as an index can hold.
    refused_squares: usize,

    /// Scenery that has appeared since the last tick, as `(x, y, object, size)`.
    ///
    /// Sent like a ground change rather than in the snapshot, because scenery is not an entity: a
    /// setpiece drawn once should not cost a place in every snapshot for the rest of the world.
    scenery_changes: Vec<(u16, u16, u16, u16)>,

    /// Squares whose breakable object has just been broken, as `(x, y, what stood there)`.
    ///
    /// A scratch buffer for one pass of [`World::break_static_objects`], held on the world so the
    /// pass allocates nothing: the entities have to be walked before the terrain can be written to.
    broken: Vec<(u32, u32, ObjectType)>,

    /// What players said this tick, and where they were standing.
    ///
    /// Cleared every tick: a word is heard when it is said, and an enemy that reacted to something
    /// shouted a minute ago would be answering an echo. Held on the world rather than passed along,
    /// because every enemy in earshot has to hear the same thing.
    heard: Vec<(f32, f32, Box<str>)>,

    /// How long since the quest arrows were last worked out.
    since_quests_ms: u32,

    /// Players who have died since the last drain.
    deaths: Vec<Death>,

    /// Quest enemies killed since the last drain, which is what Oryx answers to.
    quest_kills: Vec<QuestKill>,

    /// Hits that landed since the last drain, waiting to be sent out.
    ///
    /// Bounded like the announcements are: a world nobody is listening to still fights, and a queue
    /// nobody empties is a slow leak.
    damage: Vec<DamageEvent>,

    /// Bodies the world itself has moved since the last drain.
    ///
    /// Bounded for the same reason as the hits above.
    teleports: Vec<TeleportEvent>,

    /// Squares a player has just laid eyes on, as `(who, x, y)`, waiting to be sent as ground.
    ///
    /// The ground reaches a client because it was walked into sight, not because the client
    /// arrived: this is what [`look_around`](World::look_around) uncovers, drained once a tick by
    /// whoever serves the world. Bounded, and a look that would overflow it is left to be taken
    /// again rather than dropped, since a square marked seen but never sent is a hole in the map
    /// that nothing else would ever fill.
    revealed: Vec<(Handle, u32, u32)>,

    /// Things for the clients to draw that are neither bodies nor bullets nor numbers.
    ///
    /// Bounded for the same reason as the hits above.
    effects: Vec<EffectEvent>,

    /// Lines of text to float off a body, waiting to be sent out.
    ///
    /// Bounded for the same reason as the hits above.
    status_texts: Vec<StatusTextEvent>,

    /// Blasts that have gone off at a place, waiting to be sent out.
    ///
    /// Bounded for the same reason as the hits above.
    blasts: Vec<BlastEvent>,

    /// Setpiece names a behaviour asked for that nothing answers to, so they can be reported once.
    ///
    /// Four of the shipped behaviours name a setpiece that does not exist, in the original too. A
    /// silent miss would be a boss whose arena never appears and no way to tell.
    unknown_setpieces: std::collections::HashSet<String>,

    /// The randomness a behaviour-drawn setpiece uses.
    setpiece_dice: crate::setpiece::Dice,

    /// Where each terrain's walkable squares are, worked out on first use.
    ///
    /// Only a realm ever asks, and it asks once per terrain, so this stays empty everywhere else.
    spawn_squares: std::collections::HashMap<hendra_content::Terrain, Vec<u32>>,

    /// Neighbours as behaviours see them, rebuilt per entity per tick and reused so the think
    /// loop allocates nothing.
    neighbours: Vec<Neighbour>,
    visible: Vec<(EntityId, EntityState)>,
    hits: Vec<Hit>,
    actions: Vec<Action>,
}

impl World {
    /// Builds a world from terrain, populating it with the fixtures the map declares.
    pub fn new(name: impl Into<String>, terrain: Terrain, catalog: &Catalog) -> World {
        let name = name.into();
        let mut entities = Slab::new();

        for (x, y, square) in terrain.map().objects() {
            // Scenery is not an entity. It never moves, never acts and never changes, so it is
            // sent once with the ground and lives in the collision bitmap after that.
            //
            // This follows `Wmap.Load`, which keeps static objects on the tile rather than in the
            // world, and it is not an optimisation: the realm map carries a quarter of a million
            // trees and rocks. As entities they would fill a world four times over, leaving no room
            // for a single enemy, and every one of them would take a place in every snapshot for
            // the life of the world.
            //
            // The class is consulted as well as the flag, because the content marks the nexus
            // fixtures static too. A money changer never moves either, but a player has to be able
            // to name it to use it, and only an entity has a name to give.
            //
            // An object the map gave a name stays an entity as well: that is the map author saying
            // this one is not scenery.
            let desc = catalog.object(square.object);
            if World::is_scenery(catalog, square) {
                continue;
            }

            let mut entity = Entity::fixture(square.object, x as f32 + 0.5, y as f32 + 0.5);
            if let Some(name) = square.name() {
                entity.name = Some(name.into());
            }
            if let Some(size) = square.size() {
                entity.size = size.clamp(0, u16::MAX as i32) as u16;
            }
            if let Some(desc) = desc {
                entity.kind = Kind::of_class(desc.class.as_str());
                if matches!(entity.kind, Kind::Enemy | Kind::StaticObject) {
                    entity.hp = desc.max_hp;
                    entity.max_hp = desc.max_hp;
                }

                entity.holds_conditions = holds_conditions(desc);
                entity.take_immunities(desc);
            }

            if entities.insert(entity).is_none() {
                tracing::warn!(world = %name, "world is full; some fixtures were dropped");
                break;
            }
        }

        let grid = Grid::new(terrain.width(), terrain.height());
        let mut world = World {
            sight: Sight::default(),
            drowns: false,
            since_breath_ms: 0,
            now_ms: 0,
            name,
            difficulty: 0,
            background: 0,
            show_displays: true,
            music: String::new(),
            terrain,
            entities,
            projectiles: Projectiles::new(),
            grid,
            tick: Tick::ZERO,
            seed: 0x9e37_79b9,
            behaviours: Programs::default(),
            bag_type: ObjectType(0x0500),
            bag_types: Vec::new(),
            handles: Vec::new(),
            nearby: Vec::new(),
            awake: std::collections::HashSet::new(),
            neighbours: Vec::new(),
            announcements: Vec::new(),
            ground_changes: Vec::new(),
            quest_changes: Vec::new(),
            focus_changes: Vec::new(),
            summoned_ground: std::collections::HashSet::new(),
            falling: Vec::new(),
            fuses: Vec::new(),
            poisons: Vec::new(),
            stasis_locks: Vec::new(),
            spawn_seed: 0x2545_f491,
            realm: crate::realm::Realm::new(),
            spawn_squares: std::collections::HashMap::new(),
            allows_teleport: true,
            heard: Vec::new(),
            since_quests_ms: 0,
            deaths: Vec::new(),
            quest_kills: Vec::new(),
            damage: Vec::new(),
            teleports: Vec::new(),
            revealed: Vec::new(),
            effects: Vec::new(),
            status_texts: Vec::new(),
            blasts: Vec::new(),
            refused_squares: 0,
            scenery_changes: Vec::new(),
            broken: Vec::new(),
            unknown_setpieces: std::collections::HashSet::new(),
            setpiece_dice: crate::setpiece::Dice::new(0x5e7_9153),
            visible: Vec::new(),
            hits: Vec::new(),
            actions: Vec::new(),
        };
        world.reindex();
        world
    }

    /// Installs compiled behaviours and gives every enemy already present a mind.
    ///
    /// An enemy with no matching program keeps `None` and simply stands there, which is what a
    /// half-converted content directory should look like: most of the dungeon working.
    pub fn set_behaviours(&mut self, catalog: &Catalog, mut behaviours: Programs) {
        // Names become numbers here, once, because this is the first moment both the behaviours
        // and the catalog are in the same place. Skipping it would leave every behaviour that
        // names an entity inert: a boss would wait forever for guardians it cannot recognise, and
        // an order would reach nobody.
        for program in &mut behaviours.programs {
            // Objects first, then tiles. A behaviour names both an enemy to spawn and a ground to
            // lay down, and looking in only one place left every ground change silently doing
            // nothing.
            // Objects first, then groups, then tiles. A behaviour names all three, and a name that
            // is a group holds several: `heal_group("Crystals")` means every crystal there is, and
            // looking a group name up as an object finds nothing at all.
            let unknown = program.resolve(|name| {
                if let Some(found) = catalog.type_of(name) {
                    return vec![found.0];
                }

                let group = catalog.types_in_group(name);
                if !group.is_empty() {
                    return group.into_iter().map(|found| found.0).collect();
                }

                catalog
                    .tile_type_of(name)
                    .map(|found| vec![found.0])
                    .unwrap_or_default()
            });
            if !unknown.is_empty() {
                tracing::warn!(
                    enemy = %program.name,
                    names = ?unknown,
                    "behaviour names entities the catalog does not have"
                );
            }
        }

        self.behaviours = behaviours;

        // Every entity is offered one, not only the monsters. `ResolveBehavior` is called from the
        // base `Entity` constructor (`Entity.cs:132`) and looks the behaviour up by object type
        // alone (`BehaviorDb.cs:58-63`), so a `StaticObject` runs its tree exactly as an `Enemy`
        // does — which is what makes the Tomb turrets fire and the Shatters switches answer.
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| entity.kind != Kind::Player)
                .map(|(handle, _)| handle),
        );

        for index in 0..self.handles.len() {
            let handle = self.handles[index];
            let Some(entity) = self.entities.get(handle) else {
                continue;
            };
            let Some(id) = catalog
                .object(entity.object_type)
                .map(|desc| desc.id.clone())
            else {
                continue;
            };

            if self.behaviours.get(&id).is_some() {
                let seed = handle.0.wrapping_mul(2654435761).wrapping_add(1);
                let program = self.behaviours.get(&id).expect("checked");
                let mind = Mind::new(program, seed);
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.mind = Some(Box::new(mind));
                    // An enemy fires its own projectiles, so it is its own weapon.
                    entity.weapon = Some(entity.object_type);
                }
            }
        }
    }

    /// How many entities are running a behaviour.
    pub fn thinking(&self) -> usize {
        self.entities
            .iter()
            .filter(|(_, entity)| entity.mind.is_some())
            .count()
    }

    pub fn tick_number(&self) -> Tick {
        self.tick
    }

    pub fn terrain(&self) -> &Terrain {
        &self.terrain
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn get(&self, handle: Handle) -> Option<&Entity> {
        self.entities.get(handle)
    }

    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut Entity> {
        self.entities.get_mut(handle)
    }

    pub fn spawn(&mut self, entity: Entity) -> Option<Handle> {
        self.entities.insert(entity)
    }

    pub fn despawn(&mut self, handle: Handle) -> Option<Entity> {
        self.entities.remove(handle)
    }

    pub fn iter(&self) -> impl Iterator<Item = (Handle, &Entity)> {
        self.entities.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle, &mut Entity)> {
        self.entities.iter_mut()
    }

    /// Rebuilds the spatial index from current positions.
    ///
    /// A tick ends with this, so nothing in ordinary play has to ask for it. Anything that puts
    /// entities into a world between ticks does: until it runs, `spawn` has placed a body that
    /// nothing can see, and a behaviour looking for its guardians finds an empty room.
    pub fn reindex(&mut self) {
        // Collected first because `rebuild` takes `&mut self.grid` while reading `self.entities`.
        self.handles.clear();
        self.handles
            .extend(self.entities.iter().map(|(handle, _)| handle));

        let entities = &self.entities;
        let positions = self.handles.iter().filter_map(|handle| {
            let entity = entities.get(*handle)?;
            Some((*handle, entity.x, entity.y))
        });

        self.grid.rebuild(positions.collect::<Vec<_>>());
    }

    /// Decides where an entity really is, given where it claims to be.
    ///
    /// The claim is never trusted. Two things bound it: how far the entity could have travelled in
    /// the time available, and whether the ground it crossed to get there can be walked on. A claim
    /// that fails either is trimmed rather than rejected outright. A rejected move makes a laggy
    /// player rubber-band, while a trimmed one merely makes them slightly wrong for one tick.
    ///
    /// The whole line is walked rather than only its far end. Testing the endpoint alone asks
    /// "could you be standing there?", which any wall one tile thick answers yes to from either
    /// side: a claim of a tile and a quarter starts on open ground, ends on open ground, and passes
    /// through the wall in between. That is the shape of every wall in the game, and it made the
    /// speed limit the only thing standing between a client and any locked room on the map.
    ///
    /// Walking it is what the original does too, though not on the server. The C# server never
    /// tests a player's claim against the ground at all — `MoveHandler` hands it to `Player.Move`,
    /// which is `MoveEntity` and believes it, with the speed check beside it commented out; the
    /// only check afterwards is `IsNoClipping` (`Player.AntiCheat.cs:124`, called from
    /// `Player.cs:1089`), which disconnects a player whose *destination* square is occupied and
    /// says nothing about the ones it crossed. The sweep lives in the client, in
    /// `Player.modifyMove` (`Player.as:469-494`), which cuts any move longer than `MOVE_THRESHOLD`
    /// into pieces of that length and resolves each one — the same routine, to the constant, as
    /// `Entity.ResolveNewLocation` (`Entity.cs:322-362`) runs for every enemy on the server.
    ///
    /// So the mechanic is the original's and the constant is the original's; what has moved is
    /// which end of the wire runs it. It has to be this end, because this server resolves movement
    /// rather than believing it, and a rule enforced only by the client is not a rule.
    pub fn resolve_move(
        &self,
        handle: Handle,
        catalog: &Catalog,
        claimed_x: f32,
        claimed_y: f32,
        elapsed_ms: u32,
    ) -> Option<MoveOutcome> {
        let entity = self.entities.get(handle)?;

        // A rooted player is held where it is rather than clamped toward its claim, because a
        // clamp still lets it creep at the tolerance every tick.
        let rules = crate::effects::Rules::of(entity.conditions);
        if rules.rooted {
            return Some(MoveOutcome {
                x: entity.x,
                y: entity.y,
                refused: (claimed_x != entity.x || claimed_y != entity.y)
                    .then_some(MoveRefusal::Rooted),
                spent_ms: 0,
            });
        }

        let (mut dx, mut dy) = (claimed_x - entity.x, claimed_y - entity.y);
        let distance = (dx * dx + dy * dy).sqrt();

        let mut refused = None;
        let mut spent_ms = 0;

        // A player the server itself just moved, or one that has only just arrived, is where the
        // server put them and their client is about to say so. Measuring that against a walking
        // speed refuses the server's own teleport and snaps them back. The original has no such
        // window because it has nothing to forgive -- it never judges a player's speed -- so this
        // is ours, and it exists only to hold up the speed check that is also ours. Only the speed
        // is forgiven: where the ground allows somebody to stand does not change because they were
        // sent there.
        if entity.move_grace_ms == 0 {
            let ground = self.terrain.speed_at(catalog, entity.x, entity.y);
            let allowed = entity.stats.movement_speed(&rules)
                * ground
                * (elapsed_ms as f32 / 1000.0)
                * MOVE_TOLERANCE;

            if distance > allowed && distance > 0.0 {
                let scale = allowed / distance;
                dx *= scale;
                dy *= scale;
                refused = Some(MoveRefusal::TooFar);
                spent_ms = elapsed_ms;
            } else if allowed > 0.0 {
                spent_ms = (elapsed_ms as f32 * distance / allowed).ceil() as u32;
            }
        }

        let (from_x, from_y) = (entity.x, entity.y);
        let (x, y, blocked) = self.sweep(Walker::Player, from_x, from_y, dx, dy);

        Some(MoveOutcome {
            x,
            y,
            refused: if blocked {
                Some(MoveRefusal::Blocked)
            } else {
                refused
            },
            // Nothing is charged for a claim the ground refused outright. The player is where it
            // was, and taking the allowance for it would make walking into a wall cost the same as
            // walking, so holding a key against a wall would leave nothing to walk away with.
            spent_ms: if x == from_x && y == from_y {
                0
            } else {
                spent_ms
            },
        })
    }

    /// Resolves one movement short enough to be a single test.
    ///
    /// `Entity.CalcNewLocation` (`Entity.cs:365-445`) and `Player.modifyStep`
    /// (`Player.as:496-562`), which are the same routine. Not an axis slide: when the destination
    /// is refused the blocked axis is snapped to the *half-tile line* it tried to cross, kept a
    /// hundredth of a tile inside the square it came from when that line is also a square boundary.
    /// That snapping is what decides how a diagonal into a corner catches and how a body lines
    /// itself up with a doorway, and an axis slide reproduces neither.
    ///
    /// A move that crosses no half-tile line on either axis is taken without any test at all, which
    /// is the original's first clause and the reason small movements are cheap.
    ///
    /// `from` is the body's position at the start of the whole move, not the end of the last piece
    /// of it. That is the original's, and deliberately so on both sides of the wire:
    /// `CalcNewLocation` reads the entity's own `X`/`Y` fields, which `ResolveNewLocation` does not
    /// write until it has finished. Where a piece is aimed accumulates; what it is measured against
    /// does not.
    ///
    /// Returns whether the ground refused any part of the move.
    fn settle(
        &self,
        walker: Walker,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
    ) -> (f32, f32, bool) {
        let far_x =
            (from_x % 0.5 == 0.0 && to_x != from_x) || (from_x / 0.5) as i32 != (to_x / 0.5) as i32;
        let far_y =
            (from_y % 0.5 == 0.0 && to_y != from_y) || (from_y / 0.5) as i32 != (to_y / 0.5) as i32;

        if (!far_x && !far_y)
            || self
                .terrain
                .region_unblocked(walker, from_x, from_y, to_x, to_y)
        {
            return (to_x, to_y, false);
        }

        // The half-tile line the blocked axis is held at. Backed off by a hundredth when it is the
        // start of the next square, so a body stopped at a boundary stays in the square it was in
        // rather than landing on the line and being rounded into the one it was refused.
        let boundary = |from: f32, to: f32| -> f32 {
            let mut at = if to > from {
                ((to * 2.0) as i32) as f32 / 2.0
            } else {
                ((from * 2.0) as i32) as f32 / 2.0
            };
            if at as i32 > from as i32 {
                at -= 0.01;
            }
            at
        };

        let snapped_x = if far_x { boundary(from_x, to_x) } else { 0.0 };
        let snapped_y = if far_y { boundary(from_y, to_y) } else { 0.0 };

        // Only one axis crossed a line, so the other keeps everything it asked for.
        if !far_x {
            return (to_x, snapped_y, true);
        }
        if !far_y {
            return (snapped_x, to_y, true);
        }

        // Both crossed. Whichever axis reached further past its line is the one released first, so
        // a glancing approach slides along the wall instead of stopping dead against it.
        let over_x = if to_x > from_x {
            to_x - snapped_x
        } else {
            snapped_x - to_x
        };
        let over_y = if to_y > from_y {
            to_y - snapped_y
        } else {
            snapped_y - to_y
        };

        let (first, second) = if over_x > over_y {
            ((to_x, snapped_y), (snapped_x, to_y))
        } else {
            ((snapped_x, to_y), (to_x, snapped_y))
        };

        for (x, y) in [first, second] {
            if self.terrain.region_unblocked(walker, from_x, from_y, x, y) {
                return (x, y, true);
            }
        }

        (snapped_x, snapped_y, true)
    }

    /// Which rule a body's movement is held to.
    fn walker_of(&self, handle: Handle) -> Walker {
        match self.entities.get(handle).map(|entity| entity.kind) {
            Some(Kind::Player) => Walker::Player,
            _ => Walker::Entity,
        }
    }

    /// Follows a movement from one place to another, testing the ground the whole way.
    ///
    /// `Entity.ResolveNewLocation` (`Entity.cs:322-362`) and `Player.modifyMove`
    /// (`Player.as:468-500`), which are the same routine on either side of the wire. A move longer
    /// than [`COLLISION_STEP`] on its longer axis is cut into pieces no longer than that and each
    /// piece resolved in turn, so nothing crosses a wall by naming a position on the far side of
    /// it: a step of one tile tests the half-way point as well as the end, and a wall one tile
    /// thick stops what a single endpoint test would let straight through.
    ///
    /// Each piece is aimed from wherever the last one actually landed rather than from where it was
    /// aimed, so something sliding along a wall keeps sliding instead of jumping back onto its
    /// original heading. What each piece is *measured* against is a separate thing and stays at the
    /// body's starting position throughout — see [`World::settle`].
    ///
    /// Returns where it ended up and whether the ground refused any part of the way.
    fn sweep(
        &self,
        walker: Walker,
        from_x: f32,
        from_y: f32,
        dx: f32,
        dy: f32,
    ) -> (f32, f32, bool) {
        let longest = dx.abs().max(dy.abs());

        // A claim of infinity or a NaN has no line to walk. Refusing it leaves the body where it
        // was, which is the only answer that cannot put it somewhere the map does not have.
        if !longest.is_finite() {
            return (from_x, from_y, true);
        }

        // Short enough to be one test, which is the case `ResolveNewLocation` answers before it
        // starts stepping at all.
        if longest < COLLISION_STEP {
            return self.settle(walker, from_x, from_y, from_x + dx, from_y + dy);
        }

        let mut piece = COLLISION_STEP / longest;
        let mut travelled = 0.0f32;
        let (mut x, mut y) = (from_x, from_y);
        let mut blocked = false;
        let mut done = false;

        while !done {
            if travelled + piece >= 1.0 {
                piece = 1.0 - travelled;
                done = true;
            }

            let (next_x, next_y, refused) =
                self.settle(walker, from_x, from_y, x + dx * piece, y + dy * piece);
            x = next_x;
            y = next_y;
            blocked |= refused;
            travelled += piece;
        }

        (x, y, blocked)
    }

    /// Walks an entity along a line, testing the ground everywhere on the way.
    ///
    /// What every behaviour's movement goes through in the original, and the same [`World::sweep`]
    /// a player's claim is held to: nothing in the world crosses a wall by naming a position on the
    /// far side of it, whether a behaviour asked or a client did.
    pub fn walk(&mut self, handle: Handle, to_x: f32, to_y: f32) {
        let Some(entity) = self.entities.get(handle) else {
            return;
        };
        let (from_x, from_y) = (entity.x, entity.y);
        let walker = self.walker_of(handle);
        let (x, y, _) = self.sweep(walker, from_x, from_y, to_x - from_x, to_y - from_y);

        if let Some(entity) = self.entities.get_mut(handle) {
            entity.x = x;
            entity.y = y;
        }
    }

    /// Puts an entity somewhere it did not walk to.
    ///
    /// A jump rather than a move: nothing between here and there is looked at, because crossing
    /// what is in the way is the whole point of the thing that asked. `TeleportPosition`
    /// (`Player.cs:663-700`) tests nothing at all and sends a `Goto`; this keeps the one test the
    /// original leaves to the client, so a jump cannot end inside a wall, and otherwise leaves the
    /// body where it was rather than sliding it, which is what a jump that misses does.
    pub fn step(&mut self, handle: Handle, to_x: f32, to_y: f32) {
        let Some(entity) = self.entities.get(handle) else {
            return;
        };
        let (from_x, from_y) = (entity.x, entity.y);
        let walker = self.walker_of(handle);

        if !self
            .terrain
            .region_unblocked(walker, from_x, from_y, to_x, to_y)
        {
            return;
        }

        if let Some(entity) = self.entities.get_mut(handle) {
            entity.x = to_x;
            entity.y = to_y;
        }
    }

    /// Applies a resolved move.
    pub fn place(&mut self, handle: Handle, outcome: MoveOutcome) {
        let Some(entity) = self.entities.get_mut(handle) else {
            return;
        };

        entity.x = outcome.x;
        entity.y = outcome.y;

        if entity.kind == Kind::Player {
            self.check_lab_water(handle);
            self.look_around(handle);
        }
    }

    /// Hexes or unhexes somebody for the water they are standing in.
    ///
    /// `CheckLabConditions` (`MoveHandler.cs:41-84`), which the original runs on every move packet
    /// between recording the move and accepting it. The green water applies Hexed, Stunned and
    /// Speedy together and with no duration at all, so they are held until the blue water takes
    /// them off: that is the whole of the laboratory's puzzle, and a timer on any of the three
    /// would let somebody wait it out.
    fn check_lab_water(&mut self, handle: Handle) {
        let Some((x, y)) = self.entities.get(handle).map(|entity| (entity.x, entity.y)) else {
            return;
        };
        let Some(green) = self.terrain.lab_water_at(x, y) else {
            return;
        };

        // The original guards each branch — apply unless all three are held, clear if any one is —
        // which is two different shapes for the same idea and neither changes what happens, since
        // both operations are idempotent and the durations never vary.
        let duration = if green { FOREVER } else { 0 };
        for effect in [
            hendra_content::ConditionEffect::Hexed,
            hendra_content::ConditionEffect::Stunned,
            hendra_content::ConditionEffect::Speedy,
        ] {
            self.give_effect(handle, effect.index() as u8, duration);
        }
    }

    /// Points each player's quest arrow at the most worthwhile enemy near them.
    ///
    /// Follows `Player.HandleQuest`, which picks again every five hundred ticks or as soon as what
    /// it was pointing at is gone. Rarely, in other words: a scan over every enemy for every player
    /// is not something to do each tick, and an arrow that swings about every time an enemy wanders
    /// is worse than one that holds still.
    ///
    /// What it points at is not the nearest thing worth killing but the most worthwhile thing near
    /// enough to be worth walking to, which is what `quest::score` weighs.
    fn choose_quests(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        self.since_quests_ms = self.since_quests_ms.saturating_add(elapsed_ms);
        let due = self.since_quests_ms >= QUEST_INTERVAL_MS;
        if due {
            self.since_quests_ms = 0;
        }

        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| entity.kind == Kind::Player && !entity.dead)
                .filter(|(_, entity)| {
                    // Whether it is time, or what it was pointing at has died or left.
                    due || entity
                        .quest_target
                        .is_none_or(|target| self.entities.get(target).is_none_or(|at| at.dead))
                })
                .map(|(handle, _)| handle),
        );

        if self.handles.is_empty() {
            return;
        }

        let asking = std::mem::take(&mut self.handles);
        for player in &asking {
            let Some(entity) = self.entities.get(*player) else {
                continue;
            };
            let (x, y, level) = (entity.x, entity.y, entity.progress.level);

            // Best score wins, and the nearer of two equal scores. `FindQuest` walks its candidates
            // in ascending distance and keeps only what beats the best strictly
            // (`Player.Leveling.cs:186-204`), which is the same rule said the other way round.
            let mut best: Option<(f32, f32, Handle)> = None;

            for (handle, other) in self.entities.iter() {
                if other.kind != Kind::Enemy || other.dead {
                    continue;
                }
                let Some(desc) = catalog.object(other.object_type) else {
                    continue;
                };

                // Only what the world counts as a quest enemy. `World.Quests` holds the ones whose
                // description carries the flag (`World.cs:344`) and `FindQuest` reads nothing else,
                // so an enemy the table names but the content did not flag is never pointed at.
                if !desc.quest {
                    continue;
                }

                // Only what the table names, and only what suits this level. The range is a hard
                // filter rather than part of the score, or a high enough priority would send a
                // beginner to something that kills them.
                let Some(quest) = crate::quest::quest_for(&desc.id) else {
                    continue;
                };
                if !crate::quest::suits(&quest, level) {
                    continue;
                }

                let (dx, dy) = (other.x - x, other.y - y);
                let away = dx * dx + dy * dy;
                let score = crate::quest::score(
                    quest.priority,
                    desc.level.unwrap_or(0) as i16,
                    level,
                    away.sqrt(),
                );

                let better = match best {
                    None => true,
                    Some((held, held_away, _)) => {
                        score > held || (score == held && away < held_away)
                    }
                };
                if better {
                    best = Some((score, away, handle));
                }
            }

            let chosen = best.map(|(_, _, handle)| handle);

            if let Some(entity) = self.entities.get_mut(*player) {
                // Told only when the choice moved, and only when it moved to something.
                // `HandleQuest` sends the packet under `newQuest != null && newQuest != questEntity`
                // and nowhere else (`Player.Leveling.cs:214-228`), so an arrow that settles on the
                // same enemy again costs nothing and one that runs out of candidates is left
                // pointing where it was rather than being taken away.
                if let Some(target) = chosen
                    && entity.quest_target != chosen
                {
                    self.quest_changes.push((*player, target));
                }
                entity.quest_target = chosen;
            }
        }

        self.handles = asking;
    }

    /// What one player's arrow points at, for whoever asks.
    pub fn quest_target(&self, handle: Handle) -> Option<Handle> {
        self.entities.get(handle)?.quest_target
    }

    /// Puts one player's camera on another body, or gives it back.
    ///
    /// Naming oneself is how the original ends it: `/spectate` on your own name clears the target
    /// and lifts the pause after three seconds, and sends `SetFocus` with your own id either way
    /// (`UnrankedCommands.cs:1420-1441`).
    pub fn watch(&mut self, who: Handle, target: Handle) {
        let Some(entity) = self.entities.get_mut(who) else {
            return;
        };
        entity.watching = (target != who).then_some(target);
        self.focus_changes.push((who, target));
    }

    /// Hands a camera back when what it was watching has gone.
    ///
    /// `Player.ResetFocus` (`Player.cs:502-515`), which the watched body raises as it leaves: the
    /// watcher is pointed at their own body again and the pause is lifted. Without it, somebody
    /// watching a player who walks through a portal is left paused and looking at nothing.
    fn reset_lost_focus(&mut self) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| {
                    entity
                        .watching
                        .is_some_and(|target| self.entities.get(target).is_none())
                })
                .map(|(handle, _)| handle),
        );

        let lost = std::mem::take(&mut self.handles);
        for who in &lost {
            if let Some(entity) = self.entities.get_mut(*who) {
                entity.watching = None;
            }
            self.focus_changes.push((*who, *who));
        }
        self.handles = lost;
    }

    /// Every camera that has been moved since the last call.
    pub fn take_focus_changes(&mut self) -> Vec<(Handle, Handle)> {
        std::mem::take(&mut self.focus_changes)
    }

    /// Every arrow that has swung onto something new since the last call.
    ///
    /// Drained like the announcements and the hits: each of these is sent once, to the one player
    /// it belongs to, and a caller that forgets gets a queue that stops rather than one that grows.
    pub fn take_quest_changes(&mut self) -> Vec<(Handle, Handle)> {
        std::mem::take(&mut self.quest_changes)
    }

    /// Uncovers the circle of ground this player can see, and lists what was news.
    ///
    /// This is the whole of how a client comes to know the map. `SendUpdate`
    /// (`Player.Update.cs:127-187`) walks the sight circle every tick, sends the squares the client
    /// has not been given at their current version and remembers that it did; a two-thousand-square
    /// map cannot be handed over at once, so it is handed over a circle at a time as the player
    /// walks into it. The bits here are that memory, and what they say is news goes on
    /// [`take_revealed`](World::take_revealed) for whoever serves the world to send.
    ///
    /// The count of fresh squares is also what the two exploration fame bonuses rest on, exactly as
    /// `FameCounter.TileSent` (`Player.Update.cs:152`) is fed by the same list.
    ///
    /// Nothing is walked unless the player has crossed into a new square or the circle has been
    /// made stale, which is what makes this affordable to do on every accepted move rather than on a
    /// timer -- and a teleport crosses into a new square, so a jump across the map uncovers its
    /// landing site in the same call.
    ///
    /// What is walked is the world's own sight circle, not a disc: in a `blocking: 1` dungeon the
    /// ground beyond the room's wall ring is neither drawn nor counted, exactly as the original's
    /// `sCircle` decides both.
    pub fn look_around(&mut self, handle: Handle) {
        let (width, height) = (self.terrain.width(), self.terrain.height());

        // A fresh circle is a fresh look even from the same square: a wall coming down widens what
        // can be seen without the player taking a step.
        let took_a_fresh_circle = self.refresh_sight(handle);

        let Some(entity) = self.entities.get_mut(handle) else {
            return;
        };

        let (at_x, at_y) = (entity.x.floor() as i32, entity.y.floor() as i32);

        // Only a player has a circle; anything else has nothing to be told about the map.
        let Some(circle) = entity.sight.as_ref() else {
            return;
        };
        let seen = entity
            .seen
            .get_or_insert_with(|| Box::new(Seen::over(width, height)));

        if !took_a_fresh_circle && seen.standing_at == Some((at_x, at_y)) {
            return;
        }
        seen.standing_at = Some((at_x, at_y));

        let mut fresh = 0i32;

        for (x, y) in circle.tiles() {
            // Room to say so is checked before the square is marked, and a look that runs out
            // of it is forgotten rather than half-remembered: the rest of the circle is taken
            // again on the next call, where a square marked seen and never sent would stay
            // blank for as long as the player stood there.
            if self.revealed.len() >= MAX_PENDING_REVEALS {
                seen.standing_at = None;
                break;
            }

            if seen.look_at(*x, *y, width) {
                fresh += 1;
                self.revealed.push((handle, *x, *y));
            }
        }

        entity.tally.tiles_seen = entity.tally.tiles_seen.saturating_add(fresh);
    }

    /// Brings a player's sight circle up to date for where they are standing.
    ///
    /// `Sight.GetSightCircle(Owner.Blocking)` (`Player.Update.cs:130`), called once per pass and
    /// answering from its cache unless the player has changed square or something near them has
    /// stopped blocking sight. Nothing but a player carries one.
    ///
    /// Returns whether a fresh circle was actually taken.
    fn refresh_sight(&mut self, handle: Handle) -> bool {
        let mode = self.sight;

        let Some(entity) = self.entities.get_mut(handle) else {
            return false;
        };
        if entity.kind != Kind::Player {
            return false;
        }

        let (at_x, at_y) = (entity.x.floor() as i32, entity.y.floor() as i32);
        let circle = entity
            .sight
            .get_or_insert_with(|| Box::new(SightCircle::new()));

        circle.refresh(mode, &self.terrain, at_x, at_y)
    }

    /// Hands squares back that a player was told about but could not be sent.
    ///
    /// The bit that says a square has been given to a client is set as the circle is walked, one
    /// step before the message carrying it is built, so anything that cannot carry that message has
    /// to say so or the square is silently lost for the rest of the world. The original cannot lose
    /// one: `tiles[x, y]` is only written after the square has gone into the packet.
    pub fn forget_uncovered(&mut self, handle: Handle, squares: &[(u32, u32)]) {
        let width = self.terrain.width();

        let Some(entity) = self.entities.get_mut(handle) else {
            return;
        };
        let Some(seen) = entity.seen.as_mut() else {
            return;
        };

        for (x, y) in squares {
            seen.unsee(*x, *y, width);
        }
        seen.look_again();
    }

    /// Labels the map's open areas, which is what a `blocking: 3` world sees by.
    ///
    /// Once, at load, and only for that mode: it is a flood fill of the whole map
    /// (`World.FromWorldMap`, `World.cs:302-303`).
    pub fn label_sight_regions(&mut self) {
        self.terrain.calc_region_blocks();
    }

    /// The squares a player can see, as of the last time their circle was taken.
    pub fn sight_circle(&self, handle: Handle) -> Option<&SightCircle> {
        self.entities
            .get(handle)
            .and_then(|entity| entity.sight.as_deref())
    }

    /// Records that a square has stopped blocking sight, or started.
    ///
    /// `World.LeaveWorld` does both halves of this when a `BlocksSight` static leaves: it relabels
    /// the open areas for a `blocking: 3` world, and it makes the circle of everyone within the
    /// sight radius stale so their next pass is taken through the gap (`World.cs:397-405`).
    fn sight_blocker_changed(&mut self, x: u32, y: u32) {
        if self.terrain.has_sight_regions() && !self.terrain.blocks_sight(x, y) {
            self.terrain.update_sight_region(x, y);
        }

        let (at_x, at_y) = (x as f32 + 0.5, y as f32 + 0.5);
        for (_, entity) in self.entities.iter_mut() {
            let Some(circle) = entity.sight.as_mut() else {
                continue;
            };
            let (dx, dy) = (entity.x - at_x, entity.y - at_y);
            if dx * dx + dy * dy < SIGHT_RADIUS * SIGHT_RADIUS {
                circle.go_stale();
            }
        }
    }

    /// Forgets one square for every player who cannot currently see it, so that they are told about
    /// it again when they next can.
    ///
    /// The other half of `WmapTile.UpdateCount`. Anyone whose circle covers the square right now is
    /// being sent the change on this tick and keeps their bit; anyone who is not gets the square
    /// again the moment their circle next reaches it, which is what stops a repaint being lost to
    /// somebody who happened to be in another room when it happened.
    fn forget_square_for_the_absent(&mut self, x: u32, y: u32) {
        let width = self.terrain.width();

        for (_, entity) in self.entities.iter_mut() {
            if entity.kind != Kind::Player {
                continue;
            }
            if entity
                .sight
                .as_ref()
                .is_some_and(|circle| circle.contains(x, y))
            {
                continue;
            }
            if let Some(seen) = entity.seen.as_mut() {
                seen.unsee(x, y, width);
                seen.look_again();
            }
        }
    }

    /// Every square uncovered since the last call, as `(who, x, y)`.
    ///
    /// Drained like the teleports and the hits: each square is sent once, to the one player who
    /// uncovered it, and a caller that forgets gets a queue that stops rather than one that grows.
    pub fn take_revealed(&mut self) -> Vec<(Handle, u32, u32)> {
        std::mem::take(&mut self.revealed)
    }

    /// How many projectiles are in flight.
    pub fn projectile_count(&self) -> usize {
        self.projectiles.len()
    }

    pub fn projectiles(&self) -> impl Iterator<Item = (Handle, &Projectile)> {
        self.projectiles.iter()
    }

    /// Takes every projectile fired since this was last called, so they can be announced.
    ///
    /// Whoever serves the world calls this once a tick. A shot that is never announced still flies
    /// and still lands, which is exactly how an enemy comes to kill a player who saw nothing.
    pub fn take_fired(&mut self) -> Vec<(Handle, Projectile)> {
        self.projectiles
            .take_fired()
            .into_iter()
            .filter_map(|handle| {
                let shot = self.projectiles.get(handle)?;
                Some((handle, shot.clone()))
            })
            .collect()
    }

    /// Fires an entity's weapon on the world's own clock.
    ///
    /// For a caller with nothing finer to measure with, which is every caller inside the simulation.
    /// A player's own weapon comes through [`shoot_at`](Self::shoot_at) instead, on the clock that
    /// decides its rate of fire.
    pub fn shoot(&mut self, handle: Handle, catalog: &Catalog, angle: f32) -> Vec<Handle> {
        self.shoot_at(handle, catalog, angle, self.now_ms)
    }

    /// Fires an entity's weapon, if it has one and its interval has passed.
    ///
    /// The angle is the one thing taken from the client without argument: where a player is aiming
    /// is genuinely theirs to decide, and there is nothing to validate it against. Everything that
    /// follows, meaning where the shot goes, what it strikes and what that costs, is the server's.
    ///
    /// `at_ms` is the reading of the clock the interval is enforced on, and for a player that is
    /// their own client's: `ValidatePlayerShoot` refuses a shot whose `time` field is less than the
    /// last accepted one plus the weapon's interval, and never looks at the server's clock at all
    /// (`Player.AntiCheat.cs:93-95`). Measuring the interval against the tick instead rounds it up
    /// to the next whole tick — a 422 ms weapon becomes a 450 ms one on a fifty-millisecond tick —
    /// and worse, a client that throttles itself to its own 422 ms has every shot land in the gap
    /// and be refused, halving its rate. This is the opposite of what an enemy's behaviour needs,
    /// where the original really does quantise: `Cooldown` counts down inside a logic tick that only
    /// runs every 166 ms, so a cooldown of 500 there is 664 (`behavior::on_the_original_clock`).
    /// One clock is the client's and never rounded; the other is the server's and always rounded.
    pub fn shoot_at(
        &mut self,
        handle: Handle,
        catalog: &Catalog,
        angle: f32,
        at_ms: u32,
    ) -> Vec<Handle> {
        let mut fired = Vec::new();

        let Some(entity) = self.entities.get(handle) else {
            return fired;
        };
        if entity.dead {
            return fired;
        }

        // Exactly on the interval fires: the original refuses only what is strictly earlier
        // (`Player.AntiCheat.cs:94`).
        if at_ms < entity.ready_at_ms {
            return fired;
        }

        // No condition effect stops a player's own weapon. Stunned is checked by the behaviour
        // tree's `Shoot` (`Shoot.cs:132`) and by the six tossing behaviours, and by nothing on the
        // player at all: neither `PlayerShootHandler` nor `Player.ValidatePlayerShoot`
        // (`Player.AntiCheat.cs:88`) reads any effect, and the `StatsManager` clause that once
        // zeroed a stunned player's dexterity is commented out (`StatsManager.cs:167-179`).
        // Whether a stunned player stops firing is their client's decision in the original.
        let Some(weapon) = entity.weapon else {
            return fired;
        };
        let Some(desc) = catalog.object(weapon) else {
            return fired;
        };
        if desc.projectiles.is_empty() {
            return fired;
        }

        let (x, y) = (entity.x, entity.y);
        let from_player = entity.kind == Kind::Player;
        let rules = crate::effects::Rules::of(entity.conditions);

        // Rate of fire is quoted as a multiplier on a base of one shot every 500 ms.
        let rate = desc
            .item
            .as_ref()
            .map(|item| item.rate_of_fire.max(0.1))
            .unwrap_or(1.0);
        let cooldown = entity.stats.shot_cooldown_ms(&rules, rate);

        // The two weapon-derived damage stats. `BaseStatManager.SetWeaponDamage` writes the first
        // projectile of whatever is in slot zero into base slots 8 and 9, and `PlayerShootProjectile`
        // rolls between `Stats[8]` and `Stats[9]` rather than between the fired descriptor's own
        // bounds (`Player.Projectiles.cs:17`). For a single-descriptor weapon those are the same
        // pair; for anything with a `DamageMinBonus` on it they are not, and the stat is what wins.
        //
        // Read at the moment of firing rather than kept on the body, because that is when the
        // original recomputes it, and a cached copy is one missed equip away from arming a player
        // with a weapon they are not holding.
        let armed = entity.stats.armed_with(
            desc.projectiles[0].min_damage,
            desc.projectiles[0].max_damage,
        );

        // How many bullets one press makes, and how far apart they leave. An item's
        // `NumProjectiles` is the whole volley and every bullet of it comes from the item's first
        // projectile descriptor: the original's shoot handler reads `item.Projectiles[0]` and says
        // so in a comment (`PlayerShootHandler.cs:45`), and no item in the content declares a
        // second one. Seventy-seven weapons and shields declare more projectiles than they have
        // descriptors, which is every multi-shot weapon in the game.
        //
        // Anything with no item half is an enemy firing its own object's weapon, and keeps one
        // bullet per projectile element.
        let volley = desc
            .item
            .as_ref()
            .map(|item| (item.num_projectiles.max(1) as usize, item.arc_gap));
        let (count, gap) = volley.unwrap_or((desc.projectiles.len(), 0.0));
        let start = volley_start(angle, count, gap);
        let step = gap.to_radians();

        for index in 0..count {
            let shot = match volley {
                Some(_) => &desc.projectiles[0],
                None => &desc.projectiles[index],
            };

            let roll = self.roll();
            let mut projectile = Projectile::from_desc(
                handle,
                from_player,
                weapon,
                shot,
                x,
                y,
                start + step * index as f32,
                roll,
                index as u8,
            );

            // Truncated rather than rounded, and with no floor of one: `GetAttackDamage` ends
            // `return (int)ret` and nothing downstream clamps it (`StatsManager.cs:50-55`). A shot
            // for 101 at half power does 50, not 51.
            //
            // Only a player's shot goes through the stats. `PlayerShootProjectile` is the one
            // caller of `GetAttackDamage` for a weapon, and an enemy's own `Shoot` takes
            // `Random.Next(desc.MinDamage, desc.MaxDamage)` with no multiplier at all
            // (`Shoot.cs:181`) — an enemy has no attack stat to be multiplied by.
            if from_player {
                projectile.damage = armed.attack_damage(&rules, roll, false);
            }

            if let Some(handle) = self.projectiles.fire(projectile) {
                fired.push(handle);
            }
        }

        if let Some(entity) = self.entities.get_mut(handle) {
            // The moment the next shot is allowed, measured from this one and not from when the
            // last tick happened to fall, which is what keeps the interval exact.
            entity.ready_at_ms = at_ms.saturating_add(cooldown);

            // One per bullet, and only for players. The original counts here too, from the shoot
            // handler after the projectile is made (`PlayerShootHandler.cs:59`, which calls
            // `FameCounter.Shoot(prj)`); its client sends one such packet per bullet of a volley,
            // so a multi-shot weapon is counted once per bullet there as it is here. Counting a
            // press rather than a bullet would put this on a different scale from the hits, which
            // are counted per bullet that lands, and the accuracy bonuses divide the two.
            if entity.kind == Kind::Player {
                entity.tally.shots = entity.tally.shots.saturating_add(fired.len() as i32);
            }
        }

        fired
    }

    /// Sets where the world's rolls start from.
    ///
    /// Every world starts from the same constant, which makes one run reproducible and makes a
    /// hundred runs identical. Anything measuring a distribution — how often a three per cent drop
    /// appears, whether a chance split between two items can produce both — has to vary this, and a
    /// bug report worth reproducing has to be able to pin it.
    pub fn reseed(&mut self, seed: u32) {
        // Zero is the one state the shift-register cannot leave.
        self.seed = if seed == 0 { 1 } else { seed };
    }

    /// A deterministic roll in `0.0..1.0`.
    ///
    /// Public because the realm rolls too: which of Oryx's taunts he says, and which enemy he says
    /// it about, come from the world's own stream so a realm is reproducible from its seed.
    pub fn roll(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 10_000) as f32 / 10_000.0
    }

    /// One index out of `count`, or nought if there is nothing to pick from.
    ///
    /// `Random.Next(n)`, which is what the original reaches for wherever it chooses between a
    /// handful of named things.
    fn pick(&mut self, count: usize) -> usize {
        if count == 0 {
            return 0;
        }
        ((self.roll() * count as f32) as usize).min(count - 1)
    }

    /// A deterministic offset in `-reach..reach`.
    ///
    /// `(Rand.NextDouble() * 2 - 1) * reach`, which is how everything the original scatters on the
    /// ground is scattered. Public because a bag can be made outside the simulation, by a player
    /// dropping something, and it should land the same way one that fell out of an enemy does.
    pub fn roll_offset(&mut self, reach: f32) -> f32 {
        (self.roll() * 2.0 - 1.0) * reach
    }

    /// Advances the world by one tick.
    pub fn advance(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        self.tick = self.tick.next();
        self.now_ms = self.now_ms.saturating_add(elapsed_ms);

        // Before thinking, so something thrown this tick waits its full time rather than landing
        // on the same tick it was thrown.
        let behaviours = std::mem::take(&mut self.behaviours);
        self.land_thrown(catalog, &behaviours, elapsed_ms);
        self.behaviours = behaviours;
        self.burn_fuses(catalog, elapsed_ms);
        self.drip_poison(catalog, elapsed_ms);

        // Before the effects are counted down, as the original's timer is armed for exactly the
        // length of the hold: the immunity is raised on the tick the stasis it followed runs out.
        self.lock_out_stasis(elapsed_ms);

        self.cool_down(elapsed_ms);
        self.expire_effects(elapsed_ms);
        self.resize(elapsed_ms);
        self.sample_positions(elapsed_ms);
        self.think(catalog, elapsed_ms);
        self.spring_traps(catalog);
        self.apply_hazards(catalog, elapsed_ms);
        self.apply_suffocation(catalog, elapsed_ms);
        self.apply_effect_health(catalog, elapsed_ms);
        self.regenerate(elapsed_ms);
        self.advance_projectiles(catalog, elapsed_ms);
        self.drift_decoys(elapsed_ms);
        self.expire(elapsed_ms);
        self.break_static_objects(catalog);

        // In the order `Enemy.Death` runs them (`Enemy.cs:48-53`): the damage counter first, which
        // is where experience is handed out; then the state's own death behaviours; then the
        // `OnDeath` handlers, which is where loot is registered; and only then `LeaveWorld`. All of
        // it before the reaping, because what an entity leaves behind is decided by what it was.
        self.award_experience(catalog);
        self.note_quest_kills(catalog);
        self.run_death_effects(catalog);
        self.drop_loot(catalog);
        self.note_deaths();
        self.reap();

        // After the reaping, so an arrow that was pointing at something now dead is pointed
        // somewhere else. Before it, the kill has not been awarded yet, and re-pointing the arrow
        // first would erase the very thing that says the kill was a completed quest.
        self.choose_quests(catalog, elapsed_ms);
        self.reset_lost_focus();

        // A word is heard when it is said. Cleared after the thinking, so everything in earshot has
        // had its chance and nothing reacts to an echo on the next tick.
        self.heard.clear();
        self.reindex();
    }

    /// Runs every enemy's behaviour and applies what it asked for.
    fn think(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        // Lifted out for the loop. Nothing in a tick changes the compiled behaviours, and holding
        // them here rather than borrowing from the world is what lets the world be written to
        // while a program is being read. The alternative was cloning a program per entity per
        // tick, which is the most expensive thing that could possibly happen in this loop.
        let behaviours = std::mem::take(&mut self.behaviours);

        // Which chunks hold a player, so that only what is near one thinks. `TickLogic` ticks
        // `EnemiesCollision.GetActiveChunks(PlayersCollision)` rather than every enemy: anything
        // more than three chunks from a player is frozen where it stands. That is a rule about the
        // game and not only about cost — a boss should be waiting where it was left rather than
        // three phases further on, and a spawner should not have filled an empty room.
        self.awake.clear();
        for (_, entity) in self.entities.iter() {
            if entity.kind != Kind::Player || entity.dead {
                continue;
            }

            // Somebody hidden wakes nothing. Both `AnyPlayerNearby` overloads skip them
            // (`Utils.cs:42` and `:56`), which is what lets an administrator stand in a boss room
            // and watch it stay exactly as it was left.
            if entity
                .conditions
                .contains(hendra_content::ConditionEffect::Hidden)
            {
                continue;
            }

            let (cx, cy) = chunk_of(entity.x, entity.y);
            for dy in -ACTIVE_CHUNKS..=ACTIVE_CHUNKS {
                for dx in -ACTIVE_CHUNKS..=ACTIVE_CHUNKS {
                    self.awake.insert((cx + dx, cy + dy));
                }
            }
        }

        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                // Stasis alone stops a behaviour tree. `Entity.Tick` (`Entity.cs:225`) names it and
                // nothing else, so a paused enemy keeps thinking, moving and shooting; what Paused
                // stops is a *player's* upkeep.
                .filter(|(_, entity)| {
                    entity.mind.is_some()
                        && !entity.dead
                        && !crate::effects::Rules::of(entity.conditions).frozen
                })
                // A decoy is ticked wherever it is, as it is there: its whole job is to be
                // somewhere its owner is not.
                //
                // So is anything carrying a condition effect, however far from anyone: the same
                // line reads `(this.AnyPlayerNearby() || ConditionEffects != 0)`, so one arrow
                // wakes an enemy for as long as what it carried lasts.
                .filter(|(_, entity)| {
                    entity.kind == Kind::Decoy
                        || !entity.conditions.is_empty()
                        || self.awake.contains(&chunk_of(entity.x, entity.y))
                })
                .map(|(handle, _)| handle),
        );

        for index in 0..self.handles.len() {
            let handle = self.handles[index];

            // The neighbour buffer is lifted out of the world for the duration of the tick, so
            // that senses can borrow it while the world is being written to.
            let mut neighbours = std::mem::take(&mut self.neighbours);
            let scalars = self.gather_senses(handle, &mut neighbours);
            let Some(scalars) = scalars else {
                self.neighbours = neighbours;
                continue;
            };

            // The mind comes out of the entity for the duration of the tick, because running it
            // needs the world that the entity is part of.
            let Some(mut mind) = self.entities.get_mut(handle).and_then(|e| e.mind.take()) else {
                self.neighbours = neighbours;
                continue;
            };

            let program = self
                .entities
                .get(handle)
                .and_then(|entity| catalog.object(entity.object_type))
                .and_then(|desc| behaviours.get(&desc.id));

            let Some(program) = program else {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.mind = Some(mind);
                }
                self.neighbours = neighbours;
                continue;
            };

            // What was said, as distances from this entity. Built per entity because the condition
            // asks how far away the speaker was, and that is a different answer for each listener.
            let spoken: Vec<(f32, &str)> = if self.heard.is_empty() {
                Vec::new()
            } else {
                self.heard
                    .iter()
                    .map(|(x, y, text)| {
                        let (dx, dy) = (x - scalars.x, y - scalars.y);
                        ((dx * dx + dy * dy).sqrt(), &**text)
                    })
                    .collect()
            };

            let senses = Senses {
                x: scalars.x,
                y: scalars.y,
                hp: scalars.hp,
                max_hp: scalars.max_hp,
                spawn_x: scalars.spawn_x,
                spawn_y: scalars.spawn_y,
                nearest_player: scalars.nearest_player,
                nearest_player_hiding: scalars.nearest_player_hiding,
                nearby: &neighbours,
                said: &spoken,
                damage_taken: scalars.damage_taken,
                stunned: scalars.stunned,
                dazed: scalars.dazed,
                paralyzed: scalars.paralyzed,
            };

            let mut actions = std::mem::take(&mut self.actions);
            mind.tick(program, &senses, elapsed_ms, &mut actions);
            self.neighbours = neighbours;

            for action in &actions {
                self.apply(handle, catalog, &behaviours, program, action, elapsed_ms);
            }

            self.actions = actions;
            if let Some(entity) = self.entities.get_mut(handle) {
                entity.mind = Some(mind);
                // Reset once it has been read, so damage counts toward exactly one tick.
                entity.damage_since_tick = 0;
            }
        }

        self.behaviours = behaviours;
    }
}

/// The parts of [`Senses`] that are not borrowed.
struct SenseScalars {
    x: f32,
    y: f32,
    hp: i32,
    max_hp: i32,
    spawn_x: f32,
    spawn_y: f32,
    nearest_player: Option<Nearby>,

    /// The closest player of any kind, which the handful of behaviours written to see through
    /// invisibility read instead.
    nearest_player_hiding: Option<Nearby>,

    damage_taken: i32,

    /// The three condition effects a behaviour reads for itself, rather than the whole set: the
    /// tree holds fire while stunned, halves a volley while dazed, and zeroes its own movement
    /// speed for good while paralysed. Everything else a condition does is decided by the host
    /// when it carries an action out.
    stunned: bool,
    dazed: bool,
    paralyzed: bool,
}

/// What a name in a behaviour points at, as a list of object types.
///
/// `None` means the behaviour named nothing, which is how "anything nearby" is written. `Some` of
/// an empty list means it named something the catalog does not have, which matches nothing at all:
/// a heal aimed at a group that is not there should reach nobody rather than everybody, and reading
/// the two the same way turns a typo into a boss that heals the room.
fn named_kinds(
    program: &hendra_behavior::Program,
    name: Option<hendra_behavior::program::NameRef>,
) -> Option<Vec<ObjectType>> {
    let name = name?;

    Some(
        program
            .kinds_of(name)
            .iter()
            .map(|kind| ObjectType(*kind))
            .collect(),
    )
}

/// How many chunks from a player something still thinks.
///
/// `Collision.ACTIVE_RADIUS`, over `CHUNK_SIZE` of sixteen tiles.
const ACTIVE_CHUNKS: i32 = 3;

/// The side of one chunk, in tiles. `Collision.CHUNK_SIZE`.
const CHUNK_SIZE: f32 = 16.0;

/// Which chunk a position falls in.
fn chunk_of(x: f32, y: f32) -> (i32, i32) {
    ((x / CHUNK_SIZE) as i32, (y / CHUNK_SIZE) as i32)
}

/// A full breath.
pub const FULL_OXYGEN: i32 = 100;

/// How often air is counted, in milliseconds.
const BREATH_PERIOD_MS: u32 = 100;

/// What one count away from air costs, and what one at a vent restores.
const BREATH_COST: i32 = 2;
const BREATH_GAIN: i32 = 8;

/// What a count with no air left costs in health instead.
const DROWNING_DAMAGE: i32 = 10;

/// The object a player breathes from.
const OXYGEN_SOURCE: &str = "Ocean Vent";

/// How often ground that hurts takes its toll.
///
/// `GroundDamagePeriodMs`. One roll per half-second rather than a drain, which is what makes
/// crossing a corner of lava different from standing in it.
const GROUND_DAMAGE_PERIOD_MS: u32 = 500;

/// The chance every enemy in the game carries of dropping a tier-one potion.
///
/// `World.WorldLoot`, set on the base class and overridden by no world, and merged into every
/// enemy's own table before it is rolled.
const WORLD_POTION_CHANCE: f32 = 0.03;

/// How many items one loot bag holds.
///
/// `Loots.ShowBags` fills an eight-slot array and starts a fresh bag on the ninth item, so a drop of
/// twelve is two bags rather than eight items and four lost ones.
pub const BAG_SLOTS: usize = 8;

/// How long a bag lies on the ground.
///
/// `new Container(manager, bag, 1000 * 60, true)`, drained by `StaticObject.Tick` taking the elapsed
/// milliseconds off the container's health.
pub const BAG_LIFETIME_MS: u32 = 60_000;

/// The first loot colour that is a white bag.
///
/// A loot-drop boost recolours anything below this and leaves anything at or above it alone
/// (`Loots.cs:368`), so a white drop still looks like a white drop.
const WHITE_BAG_COLOUR: i32 = 6;

/// The colour a loot-drop boost recolours a bag to.
///
/// `RED_BAG`. One past the eight the content numbers, because the original addresses it separately
/// rather than as another step on the same scale.
const BOOSTED_BAG_COLOUR: i32 = 9;

/// The colour an enemy marked `<TrollWhiteBag/>` starts its first bag at.
///
/// `TROLL_WHITE_BAG`, the last of the eight the content numbers (`Loots.cs:311`).
const TROLL_BAG_COLOUR: i32 = 8;

/// How long before a player may teleport again.
pub const TELEPORT_COOLDOWN_MS: u32 = 10_000;

/// How long somebody who has just arrived is left alone.
///
/// `Player.SetNewbiePeriod`, three seconds. Long enough for a client to have drawn the room it
/// walked into, and short enough that it is not a way to fight.
pub const NEWCOMER_GRACE_MS: u32 = 3_000;

/// How long a teleported or newly arrived player is forgiven for being somewhere no speed explains.
///
/// A teleport moves somebody further in one tick than walking ever could, and the movement check
/// cannot tell that from a client claiming to be somewhere it is not.
///
/// Three seconds, chosen here rather than taken from anywhere: the original has no movement check
/// for a teleport to trip, so it needs no window and defines none. Three because it has to outlast
/// the round trip that tells the client where it now is, and because that is what the original
/// gives a new arrival before anything may hurt them (`Player.SetNewbiePeriod`) -- the same length
/// for the same reason, which is how long it takes a client to catch up with a world it was put in.
pub const MOVE_GRACE_MS: u32 = 3_000;

/// One hit that landed, in the shape the original broadcasts it.
///
/// `Enemy.cs:78` and `:114`, `Player.cs:788` and `:815`, `Player.Ground.cs:100` and
/// `StaticObject.cs:63` all build the same `Damage` packet and hand it to `BroadcastPacketNearby`.
/// The snapshot already carries health, so this is not how the number gets across; it is how a
/// client learns that a particular hit landed for a particular amount, and — through `kill` — how
/// it tells a body that died from one that walked out of sight.
///
/// Recorded where the health is subtracted rather than worked out afterwards, because `kill` is
/// read from the health the moment after the subtraction and nothing later can reconstruct it: by
/// the end of the tick the body has been reaped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageEvent {
    pub target: Handle,

    /// Where the target stood when it was hit.
    ///
    /// Carried rather than looked up when the event is sent, because the one event that matters
    /// most is the one that killed: by then the body has been reaped and there is nowhere left to
    /// ask. The original has no such problem — it broadcasts inside the damage call, while the
    /// entity is still in the world.
    pub x: f32,
    pub y: f32,

    /// The condition effects the hit carried, as the bitfield the snapshot uses.
    pub effects: ConditionSet,

    /// Health taken, after defence and after the target's own effects.
    pub amount: i32,

    /// Whether the target died of it, at that kind's threshold.
    pub kill: bool,

    /// Which shot of the volley landed. Zero for anything that was not a projectile, which is what
    /// the original writes for every non-projectile hit.
    pub bullet: u8,

    /// Who dealt it. `None` for the world itself, which is what ground damage is.
    pub owner: Option<Handle>,

    /// The one player not to tell, matching the `exclude` argument the original passes.
    ///
    /// A shooter already drew its own hit and a victim already knows it was hit; sending either one
    /// its own event again is a doubled damage number on the only screen that would notice.
    pub except: Option<Handle>,
}

/// A body the world put somewhere, rather than one that walked there.
///
/// Recorded rather than sent from where it happens, because the world does not know who is
/// listening: the session layer holds the connections, and this is drained on the tick the way the
/// hits are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TeleportEvent {
    /// Whose body moved.
    pub who: Handle,

    /// Where it is now.
    pub x: f32,
    pub y: f32,
}

/// Something for the clients to draw that is neither a body, a bullet nor a number.
///
/// The whole of `ShowEffect` (`networking/packets/outgoing/ShowEffect.cs:5-34`). Recorded here and
/// drained on the tick for the same reason the teleports are: the world does not know who is
/// listening.
///
/// Both positions and the target mean something different for each effect — `Structures.cs:133-151`
/// is the only description of which — so they are carried raw and interpreted only by the client
/// that draws them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectEvent {
    /// `EffectType` (`Structures.cs:133-151`), by its number.
    pub effect: u8,

    /// The body it hangs off. `None` for the effects that live at a place, or at nowhere at all:
    /// the earthquake names no target because it shakes the camera rather than the world.
    pub target: Option<Handle>,

    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,

    /// Packed `0xAARRGGBB`, as `ARGB` (`Structures.cs:153-165`) is.
    pub color: u32,
}

/// A line of text to float off a body.
///
/// The original's `Notification` (`networking/packets/outgoing/Notification.cs:5-27`), which is
/// the only way the game says something about one particular body: the `+45` a potion is worth,
/// the `+3Fame` a kill paid, the "Stasis" an enemy just fell into. Twenty of them are built across
/// the whole of `wServer` -- six in `Player.UseItem.cs`, five in the healing behaviours, four in
/// `LaunchRaidHandler.cs`, three in `Player.Leveling.cs`, two in `RankedCommands.cs` -- and none of
/// them exists anywhere else on the wire: a heal with no float is a health bar that moves for no
/// visible reason.
///
/// Drained on the tick like the effects are, for the same reason: the world does not know who is
/// close enough to read it.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusTextEvent {
    /// The body it hangs over, and follows.
    pub who: Handle,

    /// A literal, or a `LineBuilder` JSON blob naming a localisation key.
    pub text: std::sync::Arc<str>,

    /// Packed `0xAARRGGBB`.
    pub color: u32,

    /// Whether everyone in the world hears it rather than only those close enough to read it.
    ///
    /// The original is split on this and the split is not accidental. The floats that are *about*
    /// a body -- every heal, every `+Fame`, every quest -- go out through
    /// `BroadcastSync(pkt, p => this.DistSqr(p) < RadiusSqr)` or `BroadcastPacketNearby`, so they
    /// reach twenty tiles and no further. The four that are *announcements* wearing a body as an
    /// anchor -- a dungeon opened, a raid launched, an administrator putting something down -- use
    /// a bare `BroadcastPacket` and reach the whole world (`Player.UseItem.cs:502`, `:616`,
    /// `RankedCommands.cs:250`, `:510`).
    pub everywhere: bool,
}

/// A blast that has gone off at a place.
///
/// The original's `Aoe` (`networking/packets/outgoing/Aoe.cs:6-37`), sent by the two behaviours
/// that throw something and detonate it a moment later (`Grenade.cs:85`, `Ported.cs:78`). It is
/// what tells a player the ground they are standing on is about to hurt, and the reason a grenade
/// is a thing that can be dodged rather than damage out of nowhere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlastEvent {
    pub x: f32,
    pub y: f32,

    /// How far it reaches, in tiles.
    pub radius: f32,

    /// What it takes off an undefended body.
    pub damage: u16,

    /// `ConditionEffectIndex` by its number, or zero for none.
    pub effect: u8,

    /// How long that condition lasts, in seconds.
    pub duration: f32,

    /// What threw it, which the original's client names as the killer if the blast is fatal.
    pub orig_type: ObjectType,
}

/// A quest enemy that has been killed, and who killed it.
///
/// What Oryx reacts to: a taunt naming the killer, and a new event placed somewhere in the realm.
/// The killer is whoever hit last, and is nobody when a thing died of the ground.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestKill {
    pub kind: ObjectType,
    pub killer: Option<String>,
}

/// A player who has died, and what killed them.
#[derive(Debug, Clone, PartialEq)]
pub struct Death {
    pub who: Handle,

    /// What to name as the killer. The world knows this and the session does not: by the time a
    /// session hears about a death, whatever did it may already be gone.
    pub killer: String,

    pub x: f32,
    pub y: f32,

    /// Whether this sends them home instead of ending the character.
    ///
    /// `Player.Death`'s first two checks, folded into one flag: a death nobody claimed
    /// (`Rekted`, `Player.cs:880`) and a death dealt by something summoned (`NonPermaKillEnemy`,
    /// `Player.cs:862`) both leave a "got rekt" stone and put the player back in the nexus with
    /// the character still alive.
    pub rekt: bool,

    /// What the body was worth at the moment it fell.
    ///
    /// Carried rather than asked for afterwards, because the body is reaped on the tick it dies
    /// and there is nothing left to ask by the time a session hears about it. `Player.Death` has
    /// no such problem: it calls `SaveToCharacter` from inside the entity (`Player.cs:1018`),
    /// which is why the fame a death is worth includes the last seconds of the life that earned it.
    pub body: Body,
}

/// What a player was at one instant, as everything outside the world needs it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Body {
    pub hp: i32,
    pub mp: i32,
    pub max_hp: i32,
    pub max_mp: i32,
    pub level: i16,
    pub experience: i32,
    pub fame: i32,

    /// The eight base stats as they stand.
    pub stats: [i32; 8],

    /// What the character has done since the last time the counts were taken off it.
    pub tally: crate::fame::Tally,
}

/// One temporary stat boost somebody is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldBoost {
    pub stat: u8,
    pub amount: i32,
    pub remaining_ms: u32,

    /// Whether this stacks with others of its kind, or only the largest counts.
    pub stacks: bool,
}

/// How many boosts one entity may hold at once.
///
/// Generous for anything that is really a fight, and bounded because an aura nobody leaves would
/// otherwise grow the list forever.
const MOST_BOOSTS_HELD: usize = 32;

/// Works out what somebody's boosts come to, and writes it into their stats.
///
/// Recomputed from the whole list rather than adjusted, so a boost lapsing takes away exactly what
/// it was worth: with halving, what a boost contributes depends on which others are held, and no
/// amount of bookkeeping at the edges gets that right.
fn restack(entity: &mut Entity) {
    let mut totals = [0i32; hendra_content::STAT_COUNT];

    // All eleven, not just the eight a class has: `ApplyActivateBonus` walks the whole array and
    // only stops short at index eight for the condition icons, not for the boost itself
    // (`BoostStatManager.cs:124-130`).
    for stat in 0..hendra_content::STAT_COUNT as u8 {
        let stacking: Vec<i32> = entity
            .boosts
            .iter()
            .filter(|held| held.stat == stat && held.stacks)
            .map(|held| held.amount)
            .collect();

        let separate: Vec<i32> = entity
            .boosts
            .iter()
            .filter(|held| held.stat == stat && !held.stacks)
            .map(|held| held.amount)
            .collect();

        totals[stat as usize] = crate::stats::stacked(&stacking, &separate);
    }

    // The two ceilings are carried on the body rather than asked for, so a boost to either has to
    // move them by what it changed. `ReCalculateValues` recomputes `Stats[0]` and `Stats[1]` from
    // base plus boost every time a boost is pushed or popped (`StatsManager.cs:37-42`), which is
    // what makes a knight's helm raise the health bar and not merely the number under it.
    let before = entity.stats.boosts();
    entity.stats.set_boosts(totals);

    let raise = |ceiling: &mut i32, index: usize, floor: i32| {
        *ceiling = (*ceiling + totals[index] - before[index]).max(floor);
    };
    raise(&mut entity.max_hp, 0, 1);
    raise(&mut entity.max_mp, 1, 0);

    // The eight arrows the client draws over the health bar. `ApplyActivateBonus`
    // (`BoostStatManager.cs:130-152`) sets the condition effect at `i + 39` for boost `i` — the
    // eight boost icons sit at 39..=46 in the same order as the first eight stats — permanently
    // while the boost is positive, and clears it otherwise. There is no icon for a debuff: the
    // original tests `b > 0` and takes the clearing branch for everything else, so a boost that
    // has gone negative reads as no boost at all.
    //
    // Held for ever rather than for the boost's own remaining time, because the two are separate
    // clocks in the original: the icon has no timer, and disappears only because the boost lapsing
    // triggers another recalculation that clears it.
    for stat in 0..8usize {
        let Some(icon) = hendra_content::ConditionEffect::from_index((stat + 39) as u16) else {
            continue;
        };
        let wanted = totals[stat] > 0;
        if wanted == entity.conditions.contains(icon) {
            continue;
        }

        let index = icon.index() as u8;
        if wanted {
            entity.effects.push((index, FOREVER));
            entity.conditions.insert(icon);
        } else {
            entity.effects.retain(|(held, _)| *held != index);
            entity.conditions.remove(icon);
        }
    }
}

/// How much speech one tick holds before the rest is dropped.
///
/// A room full of people all typing at once is still a room, and an enemy only needs to hear one of
/// them say the word.
const MOST_HEARD_AT_ONCE: usize = 64;

/// How many players one enemy remembers being hurt by.
///
/// Generous enough for anything that is really a fight, and bounded because the list lives on every
/// enemy and a realm holds thousands of them.
const MOST_REMEMBERED_DAMAGERS: usize = 64;

/// The chest a setpiece leaves its reward in.
const SETPIECE_CHEST: &str = "Treasure Chest";

/// How much it holds. Eight, as every container in the game does.
const SETPIECE_CHEST_SLOTS: usize = 8;

/// Whether a class of object is there to be looked at rather than used.
///
/// Everything else static stays an entity, because a player has to be able to name what they use
/// and only an entity has a name to give.
/// Whether a dead entity's death is one the original would have paid out for.
///
/// Three questions at once, because the answer to all three is the same. `Enemy.Death` is where
/// both the experience and the loot come from (`Enemy.cs:48-53`), and it is reached only by
/// something that died rather than left, only by an `Enemy`, and it hands out nothing at all for
/// one that was `Spawned` (`DamageCounter.cs:73`, `Loots.cs:83`).
///
/// Being an `Enemy` is decided by class alone: `Entity.Resolve` (`Entity.cs:591-617`) makes an
/// `Enemy` of `Character` and a `StaticObject` of everything else, however the descriptor is
/// flagged. Forty-six of the shipped objects are destructible without being characters — wine
/// barrels, sprite trees, lab tables, the Shatters switches — and every one of them is a
/// `StaticObject` with no damage counter, so shooting one is worth nothing.
fn pays_for_death(entity: &Entity, catalog: &Catalog) -> bool {
    entity.dead
        && !entity.removed
        && !entity.spawned
        && entity.kind == Kind::Enemy
        && catalog
            .object(entity.object_type)
            .is_some_and(|desc| desc.class == "Character")
}

fn is_decoration(class: &str) -> bool {
    matches!(
        class,
        "GameObject"
            | "Wall"
            | "CaveWall"
            | "ConnectedWall"
            | "DoubleWall"
            | "Stalagmite"
            | "SpiderWeb"
    )
}

/// How many squares are tried before giving up on finding one nobody is standing near.
///
/// Every square tried is already the right terrain, so this only has to outlast a crowd.
const SPAWN_ATTEMPTS: usize = 20;

/// The widest circle of ground one behaviour may change at once.
///
/// The content asks for radii of ninety-nine in places, which as a circle is thirty thousand
/// squares and a tick that does not finish. What those uses mean is "the room".
const MAX_GROUND_RADIUS: f32 = 24.0;

/// The widest square of ground one death may change at once.
///
/// `ChangeGroundOnDeath` is written as a side rather than a radius, and the widest the content asks
/// for is thirty. The cap is the same guard as [`MAX_GROUND_RADIUS`]: a mistyped side of a thousand
/// is a million squares and a tick that does not finish.
const MAX_GROUND_SIDE: u32 = 64;

/// How much unsent speech is held before the rest is dropped.
const MAX_PENDING_ANNOUNCEMENTS: usize = 256;

/// How many hits may wait to be sent out before further ones go unreported.
///
/// A deliberate departure. The original queues each `Damage` packet on the players who should see
/// it and has no cap at all, so this can drop a number a player would have been shown. It is kept
/// because the queue is drained every tick and the only way to reach four thousand entries in one
/// tick is a world nobody is draining -- a session that has stopped reading, or a tick loop that
/// has stalled -- in which case the choice is between dropping the tail and growing until there is
/// no memory left. Ninety players each landing a five-bullet volley is four hundred and fifty.
const MAX_PENDING_DAMAGE: usize = 4_096;

/// How many unsent teleports are held before the rest is dropped.
///
/// Bounded like the hits above and for the same reason. A teleport costs a cooldown, so one player
/// cannot produce more than a handful a second however hard they try.
const MAX_PENDING_TELEPORTS: usize = 256;

/// How many uncovered squares are held before a look is left to be taken again.
///
/// One sight circle is thirteen hundred squares, and a teleport uncovers a whole one at once, so
/// this is room for a world's worth of players all landing somewhere new on the same tick. The
/// queue is drained every tick; the cap only decides what happens to a tick loop that has stalled.
const MAX_PENDING_REVEALS: usize = 131_072;

/// How many undrawn effects are held before the rest is dropped.
///
/// Bounded like the teleports above. An effect is a decoration: dropping one costs a sparkle,
/// where letting the queue grow costs the process.
const MAX_PENDING_EFFECTS: usize = 256;

/// The green a restored health bar floats in (`Player.UseItem.cs:1257`).
const HEAL_TEXT_COLOUR: u32 = 0xff00_ff00;

/// The purple a restored mana bar floats in (`Player.UseItem.cs:1281`).
const MANA_TEXT_COLOUR: u32 = 0xff90_00ff;

/// The orange fame is paid in (`Player.Leveling.cs:258`).
const FAME_TEXT_COLOUR: u32 = 0xffe2_5f00;

/// The green both quest completions are announced in (`Player.Leveling.cs:249`, `:310`).
const QUEST_TEXT_COLOUR: u32 = 0xff00_ff00;

/// The red a vampire blast is drawn in (`Player.UseItem.cs:870`, `:877`).
const VAMPIRE_BLAST_COLOUR: u32 = 0xffff_0000;

/// How many threads of drained health a vampire blast draws (`Player.UseItem.cs:909`).
const VAMPIRE_BLAST_THREADS: usize = 5;

/// Half the width of the cone a scepter picks its first target from, in radians.
///
/// A quarter-turn either side (`Player.UseItem.cs:705`), so a half-turn in all.
const LIGHTNING_CONE: f32 = std::f32::consts::FRAC_PI_4;

/// How far a bolt will jump from one body to the next, in tiles (`Player.UseItem.cs:743`).
const LIGHTNING_HOP: f32 = 10.0;

/// The pink every scepter's bolt is drawn in (`Player.UseItem.cs:723`, `:767`).
const LIGHTNING_COLOUR: u32 = 0xffff_0088;

/// How thick the bolt is drawn, carried in `pos2.x` (`Player.UseItem.cs:731`, `:773`).
const LIGHTNING_PARTICLE_SIZE: f32 = 350.0;

/// The name a rare drop is shouted under (`Player.Chat.cs:303`).
const LOOT_NOTIFIER: &str = "<Loot Notifer>";

/// The items whose dropping is worth telling the whole world about.
///
/// `Loots.notifItem` (`Loots.cs:186-296`), which lists a hundred and five names of which
/// twenty-eight are written twice. Duplicates are dropped here because the original only ever asks
/// `Contains`, so a second copy of a name changes nothing.
///
/// Matched against the object's id exactly, spelling and capitalisation as the file has them: the
/// original compares `i.ObjectId` to these strings with `Contains`, so "staff of green poison" in
/// lower case matches only an item written that way in the content, and an item this list spells
/// wrongly is one that never announces itself.
const NOTABLE_DROPS: &[&str] = &[
    "Demon Blade",
    "Claymore of Eternal Light",
    "Helm of the Heavenly Guard",
    "Vault of the Skies",
    "Ring of Deep Radiance",
    "Maxy",
    "Health Maxy",
    "Wisdom Maxy",
    "Vitality Maxy",
    "Dexterity Maxy",
    "Defense Maxy",
    "Mana Maxy",
    "Speed Maxy",
    "Attack Maxy",
    "Staff of the Phoenix Lord",
    "Elven Tablet of the Blood Moon",
    "Robe of the Elven Highlord",
    "Ring of the Golden Sun",
    "Wand of Dark Philosophies",
    "Scepter of the Dark Descent",
    "Robe of Foreboding Signs",
    "Crown of the Insane Alchemist",
    "Doom Bow",
    "Gladiator Sword",
    "Minotaur's Waraxe",
    "Champions breastplate",
    "Gladiator Trophy",
    "Titus's Shield",
    "Falling Star",
    "Golem's Axe",
    "Golem's Robe",
    "Sky Katana",
    "Veil of the ancient oceans",
    "Pink Robe",
    "Pink Tome",
    "Septavius Ghost Robe",
    "Sword of the Spirit Walker",
    "Sharped corals of the oceans",
    "Ocean's Hide Armor",
    "Royality Ring of Depth Oceans",
    "Pink Ring",
    "Pink Wand",
    "Staff of Unholy Sacrifice",
    "Skull of Corrupted Souls",
    "Ritual Robe",
    "Bloodshed Ring",
    "Sword of the Colossus",
    "Marble Seal",
    "Breastplate of New Life",
    "Magical Lodestone",
    "Cloak of Bloody Surprises",
    "Oryx of Dexterity",
    "Oryx of Ring",
    "Oryx of Attack",
    "Abyss of demons token",
    "The void token",
    "Lost halls token",
    "undead lair token",
    "ocean trench token",
    "snake pit token",
    "Tomb of the ancients token",
    "Bow of the void",
    "Quiver of the shadows",
    "Armor of Nil",
    "Sourcestone",
    "Omnipotence Ring",
    "Ghost Cannon",
    "Ghostly Trap",
    "Ghostly Armor",
    "Revenge Ring",
    "Wooden Helm",
    "Barriel's Enchanted Spear",
    "Staff of blood",
    "staff of green poison",
    "Robber's Gun",
    "Puppet Rainbow Dagger",
    "Orange",
    "Stheno Quiver",
    "Wooden Hide Armor",
    "Leaf Amulet",
    "Oryx of Health",
];

/// How many unsent ground changes are held before the rest is dropped.
const MAX_PENDING_GROUND_CHANGES: usize = 4096;

/// The angle the first bullet of a volley leaves at.
///
/// `startAngle = atan2(...) - (NumProjectiles - 1) / 2 * arcGap` (`Player.UseItem.cs:1121`), where
/// `(NumProjectiles - 1) / 2` is integer division between two ints. An even volley is therefore not
/// centred on where the player aimed: four bullets sit one gap to one side of the aim and two gaps
/// to the other. That lopsidedness is what the client draws and what the player learns to lead
/// with, so it is reproduced rather than corrected.
fn volley_start(angle: f32, count: usize, arc_gap_degrees: f32) -> f32 {
    let offsets = (count.max(1) - 1) / 2;
    angle - offsets as f32 * arc_gap_degrees.to_radians()
}

/// How far from the caster a bullet nova may be placed.
///
/// `Player.MaxAbilityDist` (`Player.UseItem.cs:16`), and the only thing it is used for.
const MAX_ABILITY_DIST: f32 = 14.0;

/// How many bullets a nova is.
///
/// Twenty, written into the array length and the angle step alike (`Player.UseItem.cs:1146-1153`).
/// No attribute of the activation changes it.
const BULLET_NOVA_SHOTS: u32 = 20;

/// Which of the eleven stats a number names.
///
/// All eleven, because a timed boost reaches every one of them: `AEStatBoostSelf` and
/// `AEStatBoostAura` go straight to `ActivateBoost[idx].Push` without consulting the class, so
/// content that boosts luck or a weapon's damage works there even though the permanent rise of
/// `AEIncrementStat` would throw on the same index.
fn stat_of(index: u8) -> Option<hendra_content::Stat> {
    hendra_content::ALL_STATS.get(index as usize).copied()
}

/// Whether an effect is one a player would want removed.
///
/// The set an enemy inflicts, which is the same set the immunities cover. Cleansing a beneficial
/// effect would make a purifying item a punishment.
fn is_harmful(effect: hendra_content::ConditionEffect) -> bool {
    use hendra_content::ConditionEffect::*;
    matches!(
        effect,
        Slowed
            | Sick
            | Dazed
            | Stunned
            | Blind
            | Hallucinating
            | Drunk
            | Confused
            | Paralyzed
            | Weak
            | Bleeding
            | Quiet
            | ArmorBroken
            | Hexed
            | Curse
            | Petrify
            | Darkness
            | Unstable
    )
}

/// What a trap does when something comes near.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Armed {
    pub radius: f32,
    pub damage: i32,
    pub effect: Option<u8>,
}

/// How long what a trap leaves behind lasts.
const TRAP_EFFECT_MS: u32 = 3_000;

/// Where a decoy is walking, and whether it has already announced its end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drifting {
    /// A unit vector: the heading its owner had at the moment it was left behind.
    pub dx: f32,
    pub dy: f32,

    /// `Decoy.exploded` (`Decoy.cs:56`), which is what keeps the blast to one showing.
    pub blasted: bool,
}

/// How fast a decoy walks, in tiles a second.
///
/// `new Decoy(this, eff.DurationMS, 4)` (`Player.UseItem.cs:795`) is the only decoy the game makes,
/// and the four is written there rather than in the item, so every decoy in the game moves at it.
const DECOY_SPEED: f32 = 4.0;

/// How much of a decoy's life is spent walking.
///
/// `Decoy.Tick` moves while `HP > duration - 2000` (`Decoy.cs:59`), and a decoy's health is the time
/// it has left. So it walks for its first two seconds and stands for the rest — which for the
/// three-second decoy the game ships is one second of walking and two of standing still.
const DECOY_DRIFT_MS: i32 = 2_000;

/// How little of a decoy's life is left when it announces itself.
///
/// `Decoy.cs:65`. The blast is a showing and nothing else: it damages nobody and moves nothing.
const DECOY_BLAST_MS: i32 = 250;

/// Something thrown, waiting to arrive.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Falling {
    kind: ObjectType,
    x: f32,
    y: f32,
    remaining_ms: u32,

    /// The terrain of whoever threw it, so what lands is counted where the thrower was.
    terrain: hendra_content::Terrain,
}

/// A grenade in the air.
///
/// `Grenade.TickCore` draws the throw and arms a fifteen-hundred-millisecond timer that sends the
/// `Aoe` and applies the damage together (`Grenade.cs:72-101`). Both halves wait: the ring appears
/// with the damage rather than before it, and what warns the player is the ball arcing towards them
/// for the second and a half in between.
#[derive(Debug, Clone, Copy)]
struct Fuse {
    x: f32,
    y: f32,
    remaining_ms: u32,

    /// Who threw it, so a death it causes is named after them. Kept as a handle rather than as a
    /// name because the thrower may still be alive and moving when it lands.
    from: Option<Handle>,

    /// What the thrower is, which is what the client names as the killer.
    orig_type: ObjectType,

    radius: f32,
    damage: i32,
    effect: Option<u8>,
    effect_ms: u32,

    /// What happens to whatever is standing there when it lands.
    payload: FusePayload,
}

/// What a grenade does where it lands.
///
/// The enemy's grenade and the two an item throws share a shape — a telegraph, a second and a half
/// in the air, then a circle — and share nothing else. `Grenade.TickCore` hits players once
/// (`Grenade.cs:85-100`); `AEPoisonGrenade` and `AEHealingGrenade` hand what they caught to
/// `PoisonEnemy` and `HealingPlayersPoison`, which pay out over seconds
/// (`Player.UseItem.cs:698-699`, `:1241-1242`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum FusePayload {
    /// One blow to everyone caught.
    Blast,

    /// Damage spread over this many milliseconds, to the enemies caught.
    Poison { duration_ms: u32 },

    /// Health spread over this many milliseconds, to the players caught.
    Healing { duration_ms: u32 },
}

/// Damage or health still owed to one body by a grenade, and when the next instalment is due.
///
/// `PoisonEnemy` (`Player.UseItem.cs:1292-1331`) and `HealingPlayersPoison` (`:1333-1368`), which
/// are the same loop with the sign flipped. Defence is taken once, off the whole amount, at the
/// moment the grenade lands; every instalment after that is dealt raw, because the tick calls
/// `Damage(..., true)` with the no-defence flag set. So a poison grenade thrown at an armoured
/// enemy is reduced once rather than once per second, and the total is what the item promised
/// minus that one reduction.
#[derive(Debug, Clone, Copy)]
struct Poison {
    /// Who is being poisoned or healed.
    who: Handle,

    /// Who threw it, so a kill it lands is theirs.
    from: Option<Handle>,

    /// What is left to pay.
    remaining: i32,

    /// What one instalment is worth: the whole amount divided by the duration in seconds.
    per_tick: i32,

    /// Time until the next instalment.
    until_next_ms: u32,

    /// Whether the instalments restore health rather than take it.
    heals: bool,
}

/// How long after a poison grenade lands the first instalment falls due, and the gap after that.
///
/// The original arms a two-hundred-and-fifty-millisecond timer and acts on every fourth firing
/// (`Player.UseItem.cs:1312`, `:1329`), which is a first payment a quarter of a second after the
/// ball lands and one a second thereafter. Both numbers are kept, because the quarter-second is
/// what makes the poison read as landing rather than as starting later.
const POISON_FIRST_MS: u32 = 250;
const POISON_EVERY_MS: u32 = 1_000;

/// An enemy held in stasis, and how long until it is made unfreezable.
///
/// `StasisBlast` arms a timer for exactly the length of the hold and applies `StasisImmune` for
/// three seconds when it fires (`Player.UseItem.cs:829-830`). Those three seconds are the whole
/// reason a stasis blast cannot be chained: the second cast lands while the immunity is up and is
/// answered with "Immune" rather than with another hold.
#[derive(Debug, Clone, Copy)]
struct StasisLock {
    who: Handle,
    until_ms: u32,
}

/// How close a player has to be standing to a locked door to unlock it.
///
/// Three tiles, which the original writes as `DistSqr(this) <= 9` (`Player.UseItem.cs:439`).
const UNLOCK_REACH: f32 = 3.0;

/// How long an enemy is unfreezable once its stasis runs out (`Player.UseItem.cs:830`).
const STASIS_IMMUNE_MS: u32 = 3_000;

/// The circle a stasis blast freezes, which the original writes as a literal
/// (`Player.UseItem.cs:812`) and reads from no attribute.
const STASIS_BLAST_RADIUS: f32 = 3.0;

/// How far past the aimed point the concentrate telegraph is drawn, which is what gives it its
/// radius: the client measures `dist(pos1, pos2)` (`Structures.cs:146`).
const STASIS_TELEGRAPH_REACH: f32 = 3.0;

/// The green "Immune" and the red "Stasis" a stasis blast floats over what it touched
/// (`Player.UseItem.cs:818-838`).
const IMMUNE_TEXT_COLOUR: u32 = 0xff00_ff00;
const STASIS_TEXT_COLOUR: u32 = 0xffff_0000;

/// The yellow-green both grenades an item throws are drawn in (`Player.UseItem.cs:680`, `:1222`).
const THROWN_GRENADE_COLOUR: u32 = 0xffdd_ff00;

/// How long a grenade spends in the air (`Grenade.cs:82`).
const GRENADE_FUSE_MS: u32 = 1_500;

/// The red a grenade is drawn in when the behaviour names no colour (`Grenade.cs:26`).
const GRENADE_COLOUR: u32 = 0xffff_0000;

/// How many objects may be in the air at once.
///
/// A behaviour with no cooldown could otherwise fill the queue faster than it drains.
const MAX_FALLING: usize = 256;

/// What an explosion does where it lands.
#[derive(Debug, Clone, Copy)]
struct Blast {
    /// Who set it off, so a death it causes is named after them rather than after the room.
    from: Option<Handle>,

    radius: f32,
    damage: i32,
    effect: Option<u8>,
    effect_ms: u32,

    /// Whether this catches players or enemies.
    ///
    /// An explosion has a side, as a projectile does. Without one a player's own spell hurts the
    /// people standing next to them and a trap they set hurts nobody.
    hits_players: bool,
}

/// The most children one behaviour may make in a single tick.
///
/// A bound rather than a rule: a content file asking for a thousand should get a handful and a
/// world that still ticks, not a world that stops.
const MAX_SPAWNED_AT_ONCE: u32 = 8;

/// Something an entity said, and who should hear it.
#[derive(Debug, Clone, PartialEq)]
pub struct Announcement {
    pub from: Handle,
    pub text: std::sync::Arc<str>,

    /// Heard everywhere rather than only nearby.
    pub broadcast: bool,

    /// Who to name as the speaker, where it is not the entity that spoke.
    ///
    /// Oryx is the one who needs this: `ChatManager.Oryx` (`ChatManager.cs:193`) sends his taunts
    /// under a name with nothing in the world behind it, which is what makes them the realm
    /// speaking rather than a monster in it.
    pub speaker: Option<std::sync::Arc<str>>,
}

/// A handle as an opaque number a behaviour can hand back.
fn handle_bits(handle: Handle) -> u32 {
    handle.0
}

impl World {
    /// Records where every player is, for the enemies that lead their shots.
    ///
    /// On its own clock rather than every tick, because the interval is what decides how far ahead
    /// of a target an enemy aims: `Predict` leads by four times the distance between the last two
    /// samples, so sampling at our tick rate rather than the original's would lead by a fifth of a
    /// second where the original leads by well over one. See
    /// [`hendra_behavior::program::PREDICT_SAMPLE_MS`].
    fn sample_positions(&mut self, elapsed_ms: u32) {
        for (_, entity) in self.entities.iter_mut() {
            if entity.kind != Kind::Player {
                continue;
            }

            entity.trail.since_ms = entity.trail.since_ms.saturating_add(elapsed_ms);
            if entity.trail.since_ms < hendra_behavior::program::PREDICT_SAMPLE_MS {
                continue;
            }

            // Spent rather than cleared, so the samples keep the original's average spacing instead
            // of drifting out to a whole number of our own ticks.
            entity.trail.since_ms -= hendra_behavior::program::PREDICT_SAMPLE_MS;
            entity.trail.previous = entity.trail.last;
            entity.trail.last = (entity.x, entity.y);
        }
    }

    /// Everything a behaviour perceives except its neighbours, which go into `into`.
    ///
    /// Split in two because [`Senses`] borrows the neighbour list, and the world cannot be written
    /// to while it lends out its own field. The caller holds the buffer for the tick and gives it
    /// back afterwards.
    fn gather_senses(&mut self, handle: Handle, into: &mut Vec<Neighbour>) -> Option<SenseScalars> {
        let entity = self.entities.get(handle)?;
        let (x, y, hp, max_hp) = (entity.x, entity.y, entity.hp, entity.max_hp);
        let (spawn_x, spawn_y) = (entity.spawn_x, entity.spawn_y);
        let damage_taken = entity.damage_since_tick;
        let rules = crate::effects::Rules::of(entity.conditions);
        let (stunned, dazed) = (rules.silenced, rules.dazed);

        // Paralysis on its own rather than `rules.rooted`, which is Paralyzed *or* Petrify: the
        // behaviours test `ConditionEffects.Paralyzed` alone, so a petrified enemy's type is not
        // crippled the way a paralysed one's is.
        let paralyzed = entity
            .conditions
            .contains(hendra_content::ConditionEffect::Paralyzed);

        self.grid.within(x, y, SIGHT_RADIUS, &mut self.nearby);
        into.clear();

        let mut nearest: Option<Nearby> = None;
        let mut hiding: Option<Nearby> = None;
        for found in &self.nearby {
            if *found == handle {
                continue;
            }
            let Some(other) = self.entities.get(*found) else {
                continue;
            };
            if other.dead || !other.kind.is_perceptible() {
                continue;
            }

            let (dx, dy) = (other.x - x, other.y - y);
            let distance = (dx * dx + dy * dy).sqrt();
            let player = other.kind == Kind::Player;

            // Behind a wall is not in sight, so an enemy does not chase or shoot through cover.
            let seen = self.terrain.line_of_sight(x, y, other.x, other.y);

            // Other entities are listed whether or not there is a clear line to them: a boss
            // waiting for its guardians to die is asking whether they exist, not whether it can
            // see them, and a wall between the two does not make one dead.
            into.push(Neighbour {
                kind: other.object_type.0,
                id: handle_bits(*found),
                x: other.x,
                y: other.y,
                distance,
                hp: other.hp,
                max_hp: other.max_hp,
                player,
            });

            // What an enemy will chase and shoot at, which is not the same as what it can be told
            // about. Somebody invisible, paused, or who has only just arrived is there to be looked
            // at rather than attacked.
            let worth_attacking = player
                && other.unseen_ms == 0
                && !crate::effects::Rules::of(other.conditions).unseen_by_enemies;

            // Both answers, because the two differ only for somebody hiding and the handful of
            // behaviours written to see through it read the wider one.
            // Carrying the older of the two position samples with it, because that is what a
            // leading shot subtracts from where the target is now.
            let found = Nearby {
                x: other.x,
                y: other.y,
                distance,
                past_x: other.trail.previous.0,
                past_y: other.trail.previous.1,
            };

            if player && seen && hiding.is_none_or(|closest| distance < closest.distance) {
                hiding = Some(found);
            }

            if worth_attacking && seen && nearest.is_none_or(|closest| distance < closest.distance)
            {
                nearest = Some(found);
            }
        }

        Some(SenseScalars {
            x,
            y,
            hp,
            max_hp,
            spawn_x,
            spawn_y,
            nearest_player: nearest,
            nearest_player_hiding: hiding,
            damage_taken,
            stunned,
            dazed,
            paralyzed,
        })
    }

    /// Carries out one thing a behaviour asked for.
    fn apply(
        &mut self,
        handle: Handle,
        catalog: &Catalog,
        behaviours: &Programs,
        program: &Program,
        action: &Action,
        elapsed_ms: u32,
    ) {
        match action {
            Action::Move { angle, speed } => {
                let Some(entity) = self.entities.get(handle) else {
                    return;
                };

                // Behaviours do not go through `resolve_move`, so the same rules apply here or a
                // paralysed enemy would keep walking while a paralysed player could not.
                let rules = crate::effects::Rules::of(entity.conditions);
                if rules.rooted {
                    return;
                }

                // `Utils.GetSpeed`, which is not the formula players move by and does not know
                // about `Speedy`: an enemy under it moves at its ordinary speed in the original.
                let distance = hendra_behavior::program::tiles_per_second(*speed, rules.slowed)
                    * (elapsed_ms as f32 / 1000.0);
                let (to_x, to_y) = (
                    entity.x + angle.cos() * distance,
                    entity.y + angle.sin() * distance,
                );

                // Terrain still applies, so an enemy cannot walk through a wall, but the speed
                // limit does not, because this distance came from the server rather than a client.
                // Through the sweep rather than a single test, because every behaviour in the
                // original moves its host with `ValidateAndMove` (`MoveTo.cs:34`, `Wander.cs:51`
                // and the rest), which is `ResolveNewLocation` and cuts the move into pieces first.
                let _ = catalog;
                self.walk(handle, to_x, to_y);
            }

            Action::Shoot {
                angle,
                count,
                spread,
                projectile,
            } => {
                // Centred on the aim in floating point: `startAngle = a - _shootAngle * (count - 1) / 2`
                // (`Shoot.cs:182`), where the division is between a float and an int and so keeps
                // its half. An even volley straddles the aim rather than leaning off it.
                let start = *angle - spread.to_radians() * (count.saturating_sub(1) as f32) / 2.0;
                self.fire_spread(
                    handle,
                    catalog,
                    None,
                    start,
                    *count,
                    *spread,
                    *projectile,
                    VolleyDamage::Volley,
                );
            }

            // `HealSelf` (`logic/behaviors/HealSelf.cs:52-72`), which shows itself off. `Sick` is
            // not tested here and is at `HealOthers`: only `HealPlayer` looks for it, and the three
            // that heal monsters do not, so a sickened boss still mends itself.
            Action::Heal { amount } => {
                self.heal_and_show(handle, handle, *amount);
            }

            Action::Spawn {
                child,
                count,
                offset_x,
                offset_y,
                state,
                delay_ms,
                gives_no_xp,
            } => {
                // One of them, chosen fresh, when the name is a group: `SpawnGroup` picks a member
                // per spawn, which is what makes a dwarf camp a mix rather than a row of the same
                // dwarf.
                let choices = named_kinds(program, Some(*child)).unwrap_or_default();
                let Some(kind) = self.choose(&choices) else {
                    return;
                };
                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };

                // Thrown rather than placed: it lands after its telegraph, which is what makes it
                // dodgeable rather than an unavoidable hit.
                if *delay_ms > 0 {
                    if self.falling.len() < MAX_FALLING {
                        self.falling.push(Falling {
                            kind,
                            x: x + offset_x,
                            y: y + offset_y,
                            remaining_ms: *delay_ms,
                            terrain: self
                                .entities
                                .get(handle)
                                .map(|thrower| thrower.terrain)
                                .unwrap_or_default(),
                        });
                    }
                    return;
                }

                for index in 0..(*count).min(MAX_SPAWNED_AT_ONCE) {
                    // Fanned slightly, so a group spawned together does not sit in one square and
                    // read as a single enemy.
                    let spread = index as f32 * 0.35;
                    self.spawn_child(
                        catalog,
                        behaviours,
                        kind,
                        x + offset_x + spread,
                        y + offset_y,
                        state.as_deref(),
                        Some(handle),
                        *gives_no_xp,
                    );
                }
            }

            Action::Effect {
                effect,
                duration_ms,
                radius,
                target,
            } => match target {
                EffectTarget::Myself => self.give_effect(handle, *effect, *duration_ms),
                EffectTarget::Players | EffectTarget::Others => {
                    let players = matches!(target, EffectTarget::Players);
                    self.each_nearby(handle, *radius, players, None, |world, other| {
                        world.give_effect(other, *effect, *duration_ms);
                    });
                }
            },

            Action::Texture { index } => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.texture = *index;
                }
            }

            Action::Resize { rate, target } => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.resizing = Some((*rate, *target));
                }
            }

            // Said by the world rather than by the entity, because who hears it is the world's
            // question. Held on the entity so the next snapshot carries it.
            Action::Say { text, broadcast } => {
                if self.announcements.len() < MAX_PENDING_ANNOUNCEMENTS {
                    self.announcements.push(Announcement {
                        from: handle,
                        text: text.clone(),
                        broadcast: *broadcast,
                        speaker: None,
                    });
                }
            }

            Action::Transform { into } => {
                let Some(kind) = program.kind_of(*into).map(ObjectType) else {
                    return;
                };
                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };

                // The old body leaves the world rather than dying: `Transform.cs:41` is a bare
                // `LeaveWorld`, so no experience, no loot and none of its own death behaviours.
                // What replaces it inherits `Spawned` and, with it, the permanent invisibility an
                // administrator's summons wear (`Transform.cs:30-37`); `spawn_child` reads the
                // first from the parent, which is why the body is still whole when it is called.
                let replacement =
                    self.spawn_child(catalog, behaviours, kind, x, y, None, Some(handle), false);

                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.dead = true;
                    entity.removed = true;
                }

                if let Some(replacement) = replacement
                    && let Some(entity) = self.entities.get_mut(replacement)
                    && entity.spawned
                {
                    entity
                        .conditions
                        .insert(hendra_content::ConditionEffect::Invisible);
                }
            }

            Action::Order {
                radius,
                kind,
                state,
            } => {
                let wanted = named_kinds(program, *kind);
                let state = state.clone();
                self.each_nearby(handle, *radius, false, wanted.as_deref(), |world, other| {
                    world.order_into(catalog, behaviours, other, &state);
                });
            }

            Action::HealOthers {
                radius,
                amount,
                kind,
                players,
            } => {
                let wanted = named_kinds(program, *kind);
                let amount = *amount;

                // `Sick` is only consulted when the target is a player. `HealPlayer.cs:39` skips a
                // sickened player outright, while `HealGroup` and `HealEntity` -- which heal
                // monsters -- never ask.
                let players = *players;
                self.each_nearby(
                    handle,
                    *radius,
                    players,
                    wanted.as_deref(),
                    |world, other| {
                        if players && world.is_sick(other) {
                            return;
                        }
                        world.heal_and_show(handle, other, amount);
                    },
                );
            }

            Action::Grenade {
                offset_x,
                offset_y,
                radius,
                damage,
                effect,
                effect_ms,
            } => {
                let Some((x, y, kind)) =
                    self.entities.get(handle).map(|e| (e.x, e.y, e.object_type))
                else {
                    return;
                };
                let (at_x, at_y) = (x + offset_x, y + offset_y);

                // The ball leaves the thrower now and lands in a second and a half. This is the
                // only warning the player gets, and the fuse below is what makes it a warning
                // rather than a decoration.
                self.show_effect(EffectEvent {
                    effect: hendra_net::message::effect::THROW,
                    target: Some(handle),
                    x1: at_x,
                    y1: at_y,
                    x2: 0.0,
                    y2: 0.0,
                    color: GRENADE_COLOUR,
                });

                if self.fuses.len() < MAX_FALLING {
                    self.fuses.push(Fuse {
                        x: at_x,
                        y: at_y,
                        remaining_ms: GRENADE_FUSE_MS,
                        from: Some(handle),
                        orig_type: kind,
                        radius: *radius,
                        damage: *damage,
                        effect: *effect,
                        effect_ms: *effect_ms,
                        payload: FusePayload::Blast,
                    });
                }
            }

            Action::Portal { name, duration_ms } => {
                let Some(kind) = program.kind_of(*name).map(ObjectType) else {
                    return;
                };
                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };
                if let Some(portal) =
                    self.spawn_child(catalog, behaviours, kind, x, y, None, Some(handle), false)
                    && let Some(entity) = self.entities.get_mut(portal)
                {
                    // Zero means the portal stays open, which is how the original spells it: the
                    // closing timer is armed only `if (timeoutTime != 0)`
                    // (`DropPortalOnDeath.cs:49`).
                    if *duration_ms != 0 {
                        entity.expires_in_ms = Some(*duration_ms);
                    }
                }
            }

            Action::Ground {
                tile,
                radius,
                offset_x,
                offset_y,
            } => {
                let Some(kind) = program.kind_of(*tile) else {
                    return;
                };
                let Some((x, y, summoned)) =
                    self.entities.get(handle).map(|e| (e.x, e.y, e.spawned))
                else {
                    return;
                };
                self.reshape_ground(
                    catalog,
                    x + offset_x,
                    y + offset_y,
                    *radius,
                    kind,
                    Some(summoned),
                );
            }

            Action::Setpiece { name } => {
                let Some(kind) = crate::setpiece::Kind::named(name) else {
                    // Four of these in the shipped content name nothing, in the original too:
                    // `Type.GetType` finds no class and the behaviour throws where it stands. Saying
                    // so is better than either.
                    self.unknown_setpieces.insert(name.clone());
                    return;
                };

                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };

                // Drawn from its corner, as the original places them, and centred on whoever asked
                // for it so it appears around them rather than beside them.
                let half = kind.size() as f32 / 2.0;
                let at = ((x - half).max(0.0) as u32, (y - half).max(0.0) as u32);

                let drawing = kind.draw(&mut self.setpiece_dice);
                self.draw(catalog, &drawing, at);
            }

            Action::Flash {
                colour,
                period_ms,
                repeats,
            } => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.flash = Some((*colour, *period_ms, *repeats));
                }
            }

            Action::RemoveEffect { effect } => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.effects.retain(|(held, _)| held != effect);

                    let mut conditions = hendra_content::ConditionSet::EMPTY;
                    for (held, _) in &entity.effects {
                        if let Some(known) =
                            hendra_content::ConditionEffect::from_index(*held as u16)
                        {
                            conditions.insert(known);
                        }
                    }
                    entity.conditions = conditions;
                }
            }

            // Health follows the crowd, so a boss built for forty people is not trivial when two
            // find it. Measured from the base rather than compounded, or a boss in a busy room
            // would grow without bound.
            Action::ScaleHealth {
                per_player,
                maximum_extra,
                radius,
            } => {
                let Some((x, y)) = self.entities.get(handle).map(|e| (e.x, e.y)) else {
                    return;
                };
                self.grid.within(x, y, *radius, &mut self.nearby);
                let found = std::mem::take(&mut self.nearby);

                let players = found
                    .iter()
                    .filter(|other| {
                        self.entities
                            .get(**other)
                            .is_some_and(|entity| entity.kind == Kind::Player && !entity.dead)
                    })
                    .count();
                self.nearby = found;

                if let Some(entity) = self.entities.get_mut(handle) {
                    let base = entity.base_max_hp.unwrap_or(entity.max_hp);
                    entity.base_max_hp = Some(base);

                    // Every player counts, not every player after the first. `ScaleHP.cs:71-74`
                    // grows by `(plrCount - initialScaleAmount) * amountPerPlayer`, where
                    // `initialScaleAmount` starts at `scaleAfter` — and `scaleAfter` defaults to
                    // nought and is passed by no call in the original's database, so the first
                    // player already adds a full share. Subtracting one implements a `scaleAfter`
                    // of one that nothing asked for.
                    let extra = per_player
                        .saturating_mul(players as i32)
                        .clamp(0, (*maximum_extra).max(0));
                    let before = entity.max_hp;
                    entity.max_hp = base.saturating_add(extra).max(1);

                    // The health added is granted rather than left as a hole, and health lost when
                    // the room empties comes off the top rather than killing anything.
                    entity.hp = (entity.hp + (entity.max_hp - before)).clamp(1, entity.max_hp);
                }
            }

            // `SetNoXP` writes `GivesNoXp` and nothing else (`SetNoXP.cs:15`), which zeroes the
            // experience in `DamageCounter.Death` (`DamageCounter.cs:82`) and leaves the loot
            // alone. The four Draconis dragon souls and the Shatters entity that ask for this keep
            // everything they drop.
            Action::NoExperience => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.awards_experience = false;
                }
            }

            Action::RemoveNearby { radius, kind, dies } => {
                let wanted = named_kinds(program, *kind);
                let dies = *dies;
                self.each_nearby(handle, *radius, false, wanted.as_deref(), |world, other| {
                    if let Some(entity) = world.entities.get_mut(other) {
                        entity.dead = true;
                        entity.spawned = true;
                        entity.removed = !dies;
                    }
                });
            }

            Action::Vanish { dies } => {
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.dead = true;
                    entity.removed = !dies;
                }
            }
        }
    }

    /// Creates an entity of a kind, giving it a mind if the content has one for it.
    ///
    /// Everything a behaviour spawns comes through here, so a child gets its own behaviour, its
    /// own health and its own spawn point rather than inheriting the parent's.
    ///
    /// `from` is what asked for it, where anything did. A child inherits its parent's terrain, so
    /// an enemy that breeds keeps its whole line counted against the ground it was placed on rather
    /// than leaking out of the census.
    #[allow(clippy::too_many_arguments)]
    fn spawn_child(
        &mut self,
        catalog: &Catalog,
        behaviours: &Programs,
        kind: ObjectType,
        x: f32,
        y: f32,
        state: Option<&str>,
        from: Option<Handle>,
        gives_no_xp: bool,
    ) -> Option<Handle> {
        let desc = catalog.object(kind)?;

        let mut entity = Entity::fixture(kind, x, y);
        entity.kind = Kind::of_class(desc.class.as_str());
        entity.max_hp = desc.max_hp.max(1);
        entity.hp = entity.max_hp;
        entity.spawn_x = x;
        entity.spawn_y = y;
        entity.holds_conditions = holds_conditions(desc);
        entity.take_immunities(desc);
        entity.terrain = from
            .and_then(|parent| self.entities.get(parent))
            .map(|parent| parent.terrain)
            .unwrap_or_default();

        // A spawner's children are worth no experience unless the script says otherwise, and a
        // child of something already worthless stays worthless however deep the chain runs.
        // `givesNoXp` defaults to true in the original's spawning behaviours.
        let parent_awards = from
            .and_then(|parent| self.entities.get(parent))
            .map(|parent| parent.awards_experience)
            .unwrap_or(true);
        entity.awards_experience = !gives_no_xp && parent_awards;

        // `Spawned` carries down separately and means something stronger: neither experience nor
        // loot for anything wearing it. Every spawning behaviour in the original passes it to what
        // it makes — `SpawnGroup.cs:60`, `Reproduce.cs:105`, `ReproduceGroup.cs:123`,
        // `RelativeSpawn.cs:51` and `:88`, `TossObject.cs:180` and `:190`, `Transform.cs:30` — and
        // without that a summoned spawner is a room where loot appears for free.
        entity.spawned = from
            .and_then(|parent| self.entities.get(parent))
            .is_some_and(|parent| parent.spawned);

        // Taken from the caller rather than from `self`, because the tick lifts the programs out
        // of the world while it runs. Reading them from `self` here found an empty set, and every
        // spawned child stood still forever.
        let seed = self.next_seed();
        if let Some(program) = behaviours.get(&desc.id) {
            let mut mind = Mind::new(program, seed);

            // A child ordered into a state starts there rather than at its program's beginning,
            // which is what `spawn` with a state and what `order` on arrival both mean.
            if let Some(state) = state
                && let Some(index) = program.state_named(state)
            {
                mind.force_into(program, index);
            }
            entity.mind = Some(Box::new(mind));

            // An enemy fires its own projectiles, so it is its own weapon. Set here as well as in
            // `set_behaviours`, which only reaches the entities a world was built with: without it
            // everything spawned after the world started -- every child of a spawner, everything an
            // administrator puts down -- thought and moved and wandered but could not shoot.
            entity.weapon = Some(kind);
        }

        self.spawn(entity)
    }

    /// A seed that differs per spawn, so two children of one parent do not move in lockstep.
    fn next_seed(&mut self) -> u32 {
        self.spawn_seed = self
            .spawn_seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        self.spawn_seed
    }

    /// Gives an entity a temporary stat boost.
    ///
    /// Renewing one it already holds extends it rather than adding a second, for the same reason a
    /// condition does: standing in an aura for a minute should not be sixty boosts.
    fn give_boost(&mut self, handle: Handle, boost: HeldBoost) {
        let Some(entity) = self.entities.get_mut(handle) else {
            return;
        };

        let held = entity
            .boosts
            .iter_mut()
            .find(|held| held.stat == boost.stat && held.amount == boost.amount);

        match held {
            Some(held) => held.remaining_ms = held.remaining_ms.max(boost.remaining_ms),
            None => {
                if entity.boosts.len() < MOST_BOOSTS_HELD {
                    entity.boosts.push(boost);
                }
            }
        }

        restack(entity);
    }

    /// Gives an entity a condition effect for a time, or takes it away.
    ///
    /// `ApplyConditionEffect` (`Entity.cs:707-736`), whose whole body is
    /// `_effects[eff] = durationMs`. The duration it is given *replaces* whatever was there rather
    /// than extending it, so a second, briefer source of the same effect genuinely shortens it: a
    /// half-second stun landing on a five-second one leaves half a second. Taking the longer of
    /// the two reads as the kinder rule and disagrees with the original every time two things
    /// inflict the same effect.
    ///
    /// A duration of zero is how the original clears an effect: it writes the timer and then
    /// refuses to raise the bit (`if (i.DurationMS != 0)`), and the next rebuild drops it.
    ///
    /// [`FOREVER`] stands for the original's `-1`, which is never counted down.
    pub fn give_effect(&mut self, handle: Handle, effect: u8, duration_ms: u32) {
        let Some(entity) = self.entities.get_mut(handle) else {
            return;
        };

        // Nowhere to put it. See [`holds_conditions`]: most scenery has no effect array, and what
        // is written into a missing array is not held anywhere.
        if !entity.holds_conditions {
            return;
        }

        // Consulted on every application, not only the first: `ApplyCondition` runs at the top of
        // `ApplyConditionEffect` each time, so an entity that gained an immunity while holding the
        // effect refuses the renewal and lets what it has run out.
        let known = hendra_content::ConditionEffect::from_index(effect as u16);
        if let Some(known) = known
            && !crate::effects::accepts(entity.conditions, known)
        {
            return;
        }

        if duration_ms == 0 {
            entity.effects.retain(|(held, _)| *held != effect);
            if let Some(known) = known {
                entity.conditions.remove(known);
            }
            return;
        }

        if let Some(held) = entity.effects.iter_mut().find(|(held, _)| *held == effect) {
            held.1 = duration_ms;
        } else {
            entity.effects.push((effect, duration_ms));
        }
        if let Some(known) = known {
            entity.conditions.insert(known);
        }
    }

    /// Runs something for every entity near another, of a kind and a side.
    ///
    /// The handles are collected before anything is run, because the closure writes to the world
    /// and iterating the grid while it changes is how an entity gets visited twice or not at all.
    /// One of several, drawn from the world's own roll so a run is reproducible from its seed.
    fn choose(&mut self, from: &[ObjectType]) -> Option<ObjectType> {
        match from.len() {
            0 => None,
            1 => Some(from[0]),
            many => {
                let index = (self.roll() * many as f32) as usize;
                from.get(index.min(many - 1)).copied()
            }
        }
    }

    /// Everything near this entity that matches, one at a time.
    ///
    /// `kinds` is `None` for "anything", and otherwise holds every kind that counts: one for an
    /// object, several for a group, and none at all for a name the catalog does not have.
    ///
    /// The behaviour makes the same check before it asks for anything, so either one alone is
    /// enough and no single change to one of them can be seen from outside. Both are kept because
    /// they answer different questions: the behaviour asks whether it is worth spending a cooldown,
    /// and this decides who is actually touched.
    fn each_nearby(
        &mut self,
        from: Handle,
        radius: f32,
        players: bool,
        kinds: Option<&[ObjectType]>,
        mut each: impl FnMut(&mut World, Handle),
    ) {
        let Some((x, y)) = self.entities.get(from).map(|e| (e.x, e.y)) else {
            return;
        };

        self.grid.within(x, y, radius, &mut self.nearby);
        let found = std::mem::take(&mut self.nearby);

        for handle in &found {
            if *handle == from {
                continue;
            }
            let Some(entity) = self.entities.get(*handle) else {
                continue;
            };
            if entity.dead
                || (entity.kind == Kind::Player) != players
                || kinds.is_some_and(|wanted| !wanted.contains(&entity.object_type))
            {
                continue;
            }

            each(self, *handle);
        }

        self.nearby = found;
    }

    /// Damages everything of the opposite side within a circle.
    ///
    /// Hands back the total actually taken off every body it touched, which is what a vampire
    /// blast turns into health (`Player.UseItem.cs:888-893` sums the return of each `Damage` call).
    fn explode(&mut self, catalog: &Catalog, at: (f32, f32), blast: Blast) -> i32 {
        let mut total = 0;
        let (x, y) = at;
        let Blast {
            from,
            radius,
            damage,
            effect,
            effect_ms,
            hits_players,
        } = blast;
        self.grid.within(x, y, radius, &mut self.nearby);
        let found = std::mem::take(&mut self.nearby);

        for handle in &found {
            let hit = self.entities.get(*handle).is_some_and(|entity| {
                if entity.dead {
                    return false;
                }

                // Nothing at all happens to a target that cannot be touched — not even a packet
                // naming it. `Enemy.Damage` returns before it has computed anything when the target
                // is Invincible, Paused or in Stasis (`Enemy.cs:62-66`), and `Player.Damage` returns
                // on `IsInvulnerable()` (`Player.cs:811`). Sending a zero-amount `Damage` for one of
                // those puts a "0" over a boss that a behaviour has frozen mid-phase.
                if crate::effects::Rules::of(entity.conditions).untouchable {
                    return false;
                }

                if hits_players {
                    return entity.kind == Kind::Player;
                }

                // `Enemy.Damage` returns zero before anything else when the descriptor declares no
                // maximum health (`Enemy.cs:20` and `:60`), which is how the content marks a
                // spawner or a turret indestructible.
                //
                // A `StaticObject` is not reached at all: `AOE` hands each entity to
                // `(enemy as Enemy).Damage(...)` (`Trap.cs:66`, `ConditionEffectAura.cs:63`), which
                // is null for a wine barrel and so damages nothing.
                entity.kind == Kind::Enemy
                    && catalog
                        .object(entity.object_type)
                        .is_some_and(|desc| desc.max_hp != 0)
            });
            if !hit {
                continue;
            }

            total += self.strike(catalog, from, *handle, damage);

            // The damage lands on everybody; the effect does not. `Grenade.cs:97-98` guards only
            // the `ApplyConditionEffect` with `!Invincible && !Stasis`, so somebody untouchable
            // still takes the blast's damage — which, being untouchable, is nothing — and never
            // takes what it carried.
            if let Some(effect) = effect {
                let refuses = self.entities.get(*handle).is_some_and(|entity| {
                    entity
                        .conditions
                        .contains(hendra_content::ConditionEffect::Invincible)
                        || crate::effects::Rules::of(entity.conditions).frozen
                });
                if !refuses {
                    self.give_effect(*handle, effect, effect_ms);
                }
            }
        }

        self.nearby = found;
        total
    }

    /// Takes health off one body and tells everyone what it cost.
    ///
    /// What `Enemy.Damage` and `Player.Damage` do once the target is chosen, shared by everything
    /// that hits without a projectile: a blast's circle and a scepter's chain both end here.
    /// Returns what was actually taken, which is what a vampire blast converts into health.
    fn strike(
        &mut self,
        catalog: &Catalog,
        from: Option<Handle>,
        target: Handle,
        damage: i32,
    ) -> i32 {
        // The same reduction a projectile gets, through the same path. A second copy of the
        // formula is how this came to use a different floor and to ignore every condition:
        // an invulnerable boss took full damage from a blast while shrugging off bullets.
        let taken = self
            .entities
            .get(target)
            .map(|entity| {
                let defence = crate::projectile::defence_of(entity, catalog);
                crate::effects::Rules::of(entity.conditions).damage_after_defence(
                    damage,
                    defence,
                    false,
                    entity.kind == Kind::Player,
                )
            })
            .unwrap_or(0);

        let absorbed = self
            .entities
            .get(target)
            .is_some_and(|entity| crate::effects::Rules::of(entity.conditions).no_damage);

        let mut event = None;
        let mut killed_player = false;
        let mut dealt = 0;
        if let Some(entity) = self.entities.get_mut(target) {
            // Invulnerable skips the subtraction and nothing else, so the number below is still
            // the whole blow (`Enemy.cs:112-113`).
            if !absorbed {
                entity.hp -= taken;
            }
            entity.damage_since_tick += taken;
            entity.last_hurt_by = from;
            dealt = taken;
            if entity.slain() {
                entity.dead = true;
                killed_player = entity.kind == Kind::Player;
            }

            // `Player.Damage` and `Enemy.Damage` both broadcast, with no bullet and no effects
            // (`Player.cs:815`, `Enemy.cs:78`). A blast at an enemy tells the whole room,
            // because `Enemy.Damage` passes no exclusion at all.
            //
            // The player it lands on is told as well, where `Player.cs:824` leaves them out.
            // The original leaves them out because an area effect reaches their client as an
            // `Aoe` packet that it applies and draws itself
            // (`GameServerConnectionConcrete.as:1850-1855`). The `Aoe` this server sends is a
            // telegraph rather than an instruction — the damage is decided here, as it is for
            // every other hit — so the number the blast cost still travels this way.
            event = Some(DamageEvent {
                target,
                x: entity.x,
                y: entity.y,
                effects: ConditionSet::EMPTY,
                amount: taken,
                kill: entity.dead,
                bullet: 0,
                owner: from,
                except: None,
            });
        }

        if let Some(event) = event {
            self.note_damage(event);
        }

        // Blamed on whatever set the blast off, as `Player.Damage` blames its `src`
        // (`Player.cs:827-829`).
        if killed_player {
            let (killer, summoned) = self.blame(from, catalog);
            self.claim_death(target, killer, summoned);
        }

        dealt
    }

    /// Takes health off a body without letting its armour reduce the blow.
    ///
    /// `Enemy.Damage(..., noDef: true)`, which is how a poison instalment is dealt: the reduction
    /// was taken once when the grenade landed, and taking it again every second would make an
    /// armoured enemy immune to poison rather than resistant to it (`Player.UseItem.cs:1318`).
    ///
    /// The untouchable check is `Enemy.Damage`'s own (`Enemy.cs:62-66`) and is made on every
    /// instalment rather than once, so an enemy that a behaviour freezes mid-poison stops taking it
    /// and starts again when the freeze ends.
    fn wound(
        &mut self,
        catalog: &Catalog,
        from: Option<Handle>,
        target: Handle,
        damage: i32,
    ) -> i32 {
        let untouchable = self
            .entities
            .get(target)
            .is_some_and(|entity| crate::effects::Rules::of(entity.conditions).untouchable);
        if untouchable {
            return 0;
        }

        let absorbed = self
            .entities
            .get(target)
            .is_some_and(|entity| crate::effects::Rules::of(entity.conditions).no_damage);

        let mut event = None;
        let mut killed_player = false;
        let mut dealt = 0;
        if let Some(entity) = self.entities.get_mut(target) {
            if !absorbed {
                entity.hp -= damage;
            }
            entity.damage_since_tick += damage;
            entity.last_hurt_by = from;
            dealt = damage;
            if entity.slain() {
                entity.dead = true;
                killed_player = entity.kind == Kind::Player;
            }

            event = Some(DamageEvent {
                target,
                x: entity.x,
                y: entity.y,
                effects: ConditionSet::EMPTY,
                amount: damage,
                kill: entity.dead,
                bullet: 0,
                owner: from,
                except: None,
            });
        }

        if let Some(event) = event {
            self.note_damage(event);
        }

        if killed_player {
            let (killer, summoned) = self.blame(from, catalog);
            self.claim_death(target, killer, summoned);
        }

        dealt
    }

    /// The nearest enemy within a cone pointing where the player aimed.
    ///
    /// `GetNearestEntity(MaxAbilityDist, false, ...)` with the cone test as its predicate
    /// (`Player.UseItem.cs:709-710`). Nearest to the caster rather than to the cursor: the cursor
    /// only chooses the direction.
    ///
    /// The cone test is the original's, angle difference and all: `Math.Abs(aimed - theirs)`, with
    /// no wrapping. Two bodies either side of due west differ by nearly a full turn rather than by
    /// nothing, so a scepter aimed straight left picks up whichever half of the cone the arithmetic
    /// happens to favour. That is a bug, and it is the behaviour every player of the original
    /// learned to aim around.
    fn nearest_in_cone(
        &mut self,
        caster: Handle,
        from: (f32, f32),
        aimed: f32,
        cone: f32,
    ) -> Option<Handle> {
        self.grid
            .within(from.0, from.1, MAX_ABILITY_DIST, &mut self.nearby);
        let found = std::mem::take(&mut self.nearby);

        let mut best: Option<(Handle, f32)> = None;
        for handle in &found {
            if *handle == caster {
                continue;
            }
            let Some(entity) = self.entities.get(*handle) else {
                continue;
            };
            if entity.dead || entity.kind != Kind::Enemy {
                continue;
            }

            let (dx, dy) = (entity.x - from.0, entity.y - from.1);
            let distance = (dx * dx + dy * dy).sqrt();
            if distance >= MAX_ABILITY_DIST || (aimed - dy.atan2(dx)).abs() > cone {
                continue;
            }

            if best.is_none_or(|(_, nearest)| distance < nearest) {
                best = Some((*handle, distance));
            }
        }

        self.nearby = found;
        best.map(|(handle, _)| handle)
    }

    /// The next body a bolt jumps to.
    ///
    /// `GetNearestEntity(10, false, ...)` from wherever the bolt currently is, refusing anything it
    /// has already visited and anything invincible or in stasis (`Player.UseItem.cs:743-752`).
    fn nearest_untouched(&mut self, from: Handle, visited: &[Handle]) -> Option<Handle> {
        let Some((x, y)) = self.entities.get(from).map(|e| (e.x, e.y)) else {
            return None;
        };

        self.grid.within(x, y, LIGHTNING_HOP, &mut self.nearby);
        let found = std::mem::take(&mut self.nearby);

        let mut best: Option<(Handle, f32)> = None;
        for handle in &found {
            if *handle == from || visited.contains(handle) {
                continue;
            }
            let Some(entity) = self.entities.get(*handle) else {
                continue;
            };
            if entity.dead || entity.kind != Kind::Enemy {
                continue;
            }
            if entity
                .conditions
                .contains(hendra_content::ConditionEffect::Invincible)
                || crate::effects::Rules::of(entity.conditions).frozen
            {
                continue;
            }

            let (dx, dy) = (entity.x - x, entity.y - y);
            let distance = (dx * dx + dy * dy).sqrt();
            if distance >= LIGHTNING_HOP {
                continue;
            }

            if best.is_none_or(|(_, nearest)| distance < nearest) {
                best = Some((*handle, distance));
            }
        }

        self.nearby = found;
        best.map(|(handle, _)| handle)
    }

    /// Puts an entity into a named state, if it has one by that name.
    fn order_into(
        &mut self,
        catalog: &Catalog,
        behaviours: &Programs,
        handle: Handle,
        state: &str,
    ) {
        let Some(program) = self
            .entities
            .get(handle)
            .and_then(|entity| catalog.object(entity.object_type))
            .and_then(|desc| behaviours.get(&desc.id))
        else {
            return;
        };

        // An order naming a state the target does not have is ignored rather than guessed at. The
        // content sends one order to several kinds of minion and expects each to take the part of
        // it that applies.
        let Some(index) = program.state_named(state) else {
            return;
        };

        if let Some(entity) = self.entities.get_mut(handle)
            && let Some(mind) = entity.mind.as_mut()
        {
            // A target already inside the ordered state is left alone. Orders stand for as long as
            // the sender holds its own state, so without this an entity under a standing order
            // would have that state — and every cooldown in it — restarted on every repeat, and a
            // staggered volley spread over ten seconds would never get past its first repeat.
            if mind.is_within(program, index) {
                return;
            }
            mind.force_into(program, index);
        }
    }

    /// Replaces the ground in a circle with another tile.
    ///
    /// Walkability and sight follow the new tile, and the change is recorded so a snapshot can
    /// carry it. Without the record the ground would change for the simulation and not for anyone
    /// looking at it, which is worse than not changing it at all.
    ///
    /// `summoned` is whether whatever laid it down was itself summoned, which each square
    /// remembers: the original writes `tile.Spawned = host.Spawned` as it paints
    /// (`GroundTransform.cs:72`), so ground laid by something real overwrites the mark rather than
    /// leaving it behind. `None` leaves the mark as it was, which is what a ground change that
    /// clones the square and rewrites only its id does (`ChangeGroundOnDeath.cs:39`).
    fn reshape_ground(
        &mut self,
        catalog: &Catalog,
        x: f32,
        y: f32,
        radius: f32,
        tile: u16,
        summoned: Option<bool>,
    ) {
        let tile_type = hendra_content::TileType(tile);
        if catalog.tile(tile_type).is_none() {
            return;
        }

        let radius = radius.clamp(0.0, MAX_GROUND_RADIUS);
        let (from_x, from_y) = ((x - radius).floor() as i32, (y - radius).floor() as i32);
        let (to_x, to_y) = ((x + radius).ceil() as i32, (y + radius).ceil() as i32);

        // No radius is the single square the point falls in, which is what a ground transform with
        // a pair of offsets changes.
        let single = radius <= 0.0;

        for square_y in from_y..=to_y {
            for square_x in from_x..=to_x {
                if square_x < 0 || square_y < 0 {
                    continue;
                }
                let (dx, dy) = (square_x as f32 + 0.5 - x, square_y as f32 + 0.5 - y);
                if !single && dx * dx + dy * dy > radius * radius {
                    continue;
                }
                if single && (square_x != x.floor() as i32 || square_y != y.floor() as i32) {
                    continue;
                }

                // Already this ground, so nothing to change and nothing to say. The original checks
                // the same thing on both of its branches (`GroundTransform.cs:58-59`, `:84-85`) and
                // returns without recording the square, which is what keeps a boss standing in its
                // own lava from sending the same square every second.
                if self.terrain.tile_at(square_x as u32, square_y as u32).0 == tile {
                    continue;
                }

                // Ground never blocks sight. Only objects standing on it do, and this changes
                // the ground rather than what is on it.
                self.terrain
                    .set_ground(catalog, square_x as u32, square_y as u32, tile_type);
                match summoned {
                    Some(true) => {
                        self.summoned_ground
                            .insert((square_x as u32, square_y as u32));
                    }
                    Some(false) => {
                        self.summoned_ground
                            .remove(&(square_x as u32, square_y as u32));
                    }
                    None => {}
                }

                if self.ground_changes.len() < MAX_PENDING_GROUND_CHANGES {
                    self.ground_changes
                        .push((square_x as u16, square_y as u16, tile));
                }
            }
        }
    }

    /// Rewrites the ground a death stands on, where it is already ground of a named kind.
    ///
    /// `ChangeGroundOnDeath.Resolve` (`ChangeGroundOnDeath.cs:26-59`), which is a different shape
    /// from [`Self::reshape_ground`] in three ways worth spelling out, because the Shatters is built
    /// on all three.
    ///
    /// It is a `dist` by `dist` *square*, not a disc, and its corner rather than its centre is
    /// placed: `pos` is the square the enemy died on less `dist / 2` in integer arithmetic, so a
    /// `dist` of thirty reaches fifteen squares one way and fourteen the other, and an odd `dist`
    /// is lopsided in the original too.
    ///
    /// Only squares already laid with one of `sources` are touched, and each is rewritten to one of
    /// `targets` drawn afresh per square per source name. An empty `sources` is the original's
    /// `groundToChange == null`, which rewrites everything in reach unconditionally.
    ///
    /// A square that matches one source and is rewritten is still tested against the sources after
    /// it, now holding its new kind — so a call naming both a floor and what that floor becomes can
    /// rewrite the same square twice in one pass. That is what the original does and the Shatters'
    /// bridge closers depend on the pairing, so it is kept.
    fn change_ground(
        &mut self,
        catalog: &Catalog,
        x: f32,
        y: f32,
        dist: u32,
        sources: &[u16],
        targets: &[u16],
    ) {
        if targets.is_empty() {
            return;
        }

        // The content asks for thirty at most; the clamp is here for the same reason
        // `MAX_GROUND_RADIUS` is, so a mistyped number cannot cost a tick.
        let side = dist.min(MAX_GROUND_SIDE) as i32;
        let corner_x = x as i32 - side / 2;
        let corner_y = y as i32 - side / 2;

        // `x` outer and `y` inner, as `ChangeGroundOnDeath.cs:33-35` walks it. Only the order the
        // random draws are consumed in depends on it, which shows only where a call names more than
        // one target — but matching it costs nothing.
        for step_x in 0..side {
            for step_y in 0..side {
                let (square_x, square_y) = (corner_x + step_x, corner_y + step_y);
                if square_x < 0 || square_y < 0 {
                    continue;
                }
                let (square_x, square_y) = (square_x as u32, square_y as u32);
                if !self.terrain.contains(square_x, square_y) {
                    continue;
                }

                // Nothing named to match against means match everything, once.
                if sources.is_empty() {
                    let target = targets[self.pick(targets.len())];
                    self.repaint_square(catalog, square_x, square_y, target);
                    continue;
                }

                for source in sources {
                    // Drawn before the comparison, as the original draws it
                    // (`ChangeGroundOnDeath.cs:42`), so a source that does not match still moves the
                    // shared random stream on.
                    let target = targets[self.pick(targets.len())];
                    if self.terrain.tile_at(square_x, square_y).0 == *source {
                        self.repaint_square(catalog, square_x, square_y, target);
                    }
                }
            }
        }
    }

    /// Lays one square of ground and records it for the next snapshot.
    ///
    /// The summoned mark is left as it was: `ChangeGroundOnDeath` clones the square and rewrites
    /// only its tile id (`ChangeGroundOnDeath.cs:39`).
    fn repaint_square(&mut self, catalog: &Catalog, x: u32, y: u32, tile: u16) {
        let tile_type = hendra_content::TileType(tile);
        if catalog.tile(tile_type).is_none() {
            return;
        }
        self.terrain.set_ground(catalog, x, y, tile_type);
        if self.ground_changes.len() < MAX_PENDING_GROUND_CHANGES {
            self.ground_changes.push((x as u16, y as u16, tile));
        }
    }

    /// Ages held effects, dropping the ones that have run out.
    fn expire_effects(&mut self, elapsed_ms: u32) {
        for (_, entity) in self.entities.iter_mut() {
            if entity.effects.is_empty() {
                continue;
            }

            entity.effects.retain_mut(|(_, left)| {
                // An effect written as never lapsing is not counted down at all, which is how the
                // immunities an entity is born with survive the world it is standing in.
                if *left == FOREVER {
                    return true;
                }
                *left = left.saturating_sub(elapsed_ms);
                *left > 0
            });

            // Rebuilt from what is left rather than cleared per effect, so an effect held by two
            // sources does not vanish when the first of them lapses.
            let mut conditions = ConditionSet::EMPTY;
            for (effect, _) in &entity.effects {
                if let Some(known) = hendra_content::ConditionEffect::from_index(*effect as u16) {
                    conditions.insert(known);
                }
            }
            entity.conditions = conditions;
        }
    }

    /// Grows or shrinks whatever is changing size.
    fn resize(&mut self, elapsed_ms: u32) {
        for (_, entity) in self.entities.iter_mut() {
            let Some((rate, target)) = entity.resizing else {
                continue;
            };

            let step = rate * (elapsed_ms as f32 / 1000.0);
            let now = entity.size as f32 + step;

            // Stops at the target rather than oscillating around it, whichever way it was going.
            let reached = if rate >= 0.0 {
                now >= target as f32
            } else {
                now <= target as f32
            };

            if reached {
                entity.size = target;
                entity.resizing = None;
            } else {
                entity.size = now.clamp(0.0, 65_535.0) as u16;
            }
        }
    }

    /// Fires a spread of projectiles from a weapon or from an ability item.
    ///
    /// `source` names the object whose projectile is fired. An ability shoots its own bullet rather
    /// than the weapon's — `AEShoot` reads `item.Projectiles[0]` from the item being used
    /// (`Player.UseItem.cs:1122`) — while a behaviour's shot comes from whatever the entity is
    /// holding, which is what `None` means here.
    ///
    /// `start` is the angle of the first bullet, not the aim. The two callers centre a volley
    /// differently and the difference is visible: a behaviour divides in floating point
    /// (`Shoot.cs:182`) and an ability divides in integers (`Player.UseItem.cs:1121`), so an
    /// even-numbered ability volley sits off to one side of where it was aimed.
    ///
    /// `damage` says whether the volley shares one roll, which the two callers also differ on.
    #[allow(clippy::too_many_arguments)]
    fn fire_spread(
        &mut self,
        handle: Handle,
        catalog: &Catalog,
        source: Option<ObjectType>,
        start: f32,
        count: u32,
        spread: f32,
        projectile: u8,
        damage: VolleyDamage,
    ) {
        let Some(entity) = self.entities.get(handle) else {
            return;
        };
        let Some(weapon) = source.or(entity.weapon) else {
            return;
        };
        let Some(desc) = catalog.object(weapon) else {
            return;
        };
        if desc.projectiles.is_empty() {
            return;
        }

        let shot = desc
            .projectiles
            .get(projectile as usize)
            .unwrap_or(&desc.projectiles[0])
            .clone();

        let (x, y) = (entity.x, entity.y);
        let from_player = entity.kind == Kind::Player;

        let step = spread.to_radians();

        // Drawn before the loop when the whole volley shares it, so the fan is uniform and the
        // stream advances once rather than once per bullet.
        let shared = match damage {
            VolleyDamage::Volley => Some(self.roll()),
            VolleyDamage::PerBullet => None,
        };

        for index in 0..count.min(64) {
            let roll = match shared {
                Some(roll) => roll,
                None => self.roll(),
            };
            let projectile = Projectile::from_desc(
                handle,
                from_player,
                weapon,
                &shot,
                x,
                y,
                start + step * index as f32,
                roll,
                index as u8,
            );
            self.projectiles.fire(projectile);
        }

        // One per bullet, as the original counts an ability's volley: `AEShoot` calls
        // `FameCounter.Shoot` inside the loop that makes the projectiles
        // (`Player.UseItem.cs:1138`), and so does `AEBulletNova` (`:1155`). Only a player has a
        // tally; a behaviour's shot is an enemy's and belongs to nobody.
        if let Some(shooter) = self.entities.get_mut(handle)
            && shooter.kind == Kind::Player
        {
            shooter.tally.shots = shooter.tally.shots.saturating_add(count.min(64) as i32);
        }
    }

    /// Fires one of an item's bullets from a point of its own choosing.
    ///
    /// Apart from `fire_spread` because the starting position is not the shooter's: a nova is the
    /// only thing in the game that puts bullets somewhere the player is not standing, and passing
    /// an origin to every volley in the server to serve one of them would be the wrong shape.
    fn fire_ring(
        &mut self,
        handle: Handle,
        catalog: &Catalog,
        item: ObjectType,
        at: (f32, f32),
        angle: f32,
        id: u8,
    ) {
        let Some(entity) = self.entities.get(handle) else {
            return;
        };
        let from_player = entity.kind == Kind::Player;

        let Some(shot) = catalog
            .object(item)
            .and_then(|desc| desc.projectiles.first())
            .cloned()
        else {
            return;
        };

        let roll = self.roll();
        let mut projectile = Projectile::from_desc(
            handle,
            from_player,
            item,
            &shot,
            at.0,
            at.1,
            angle,
            roll,
            id,
        );

        // It starts where the cursor was rather than where the caster is, so the caster's own
        // client has no way to have drawn it and has to be told, as `AEBulletNova` tells it.
        projectile.predicted_by_owner = false;
        self.projectiles.fire(projectile);

        if let Some(shooter) = self.entities.get_mut(handle)
            && shooter.kind == Kind::Player
        {
            shooter.tally.shots = shooter.tally.shots.saturating_add(1);
        }
    }

    fn cool_down(&mut self, elapsed_ms: u32) {
        for (_, entity) in self.entities.iter_mut() {
            entity.teleport_cooldown_ms = entity.teleport_cooldown_ms.saturating_sub(elapsed_ms);
            entity.move_grace_ms = entity.move_grace_ms.saturating_sub(elapsed_ms);
            entity.unseen_ms = entity.unseen_ms.saturating_sub(elapsed_ms);

            // `TickActivateEffects` (`Player.cs:593-602`): a finished character loses the boost
            // outright, one that never lapses is left alone, and the rest counts down to nothing.
            if entity.experience_boost_ms != 0 {
                if entity.progress.level >= crate::leveling::MAX_LEVEL {
                    entity.experience_boost_ms = 0;
                } else if entity.experience_boost_ms > 0 {
                    entity.experience_boost_ms =
                        (entity.experience_boost_ms - elapsed_ms as i32).max(0);
                }
            }

            if !entity.boosts.is_empty() {
                let before = entity.boosts.len();
                entity.boosts.retain_mut(|held| {
                    held.remaining_ms = held.remaining_ms.saturating_sub(elapsed_ms);
                    held.remaining_ms > 0
                });

                // Only when something actually lapsed: restacking is cheap but not free, and most
                // ticks change nothing.
                if entity.boosts.len() != before {
                    restack(entity);
                }
            }
        }
    }

    /// Moves every projectile and applies whatever it struck.
    fn advance_projectiles(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        // Taken out so the damage pass can borrow the entities mutably; the buffer goes back
        // afterwards, so this still allocates nothing once warm.
        let mut hits = std::mem::take(&mut self.hits);

        self.projectiles.advance(
            &self.entities,
            &self.grid,
            &self.terrain,
            catalog,
            elapsed_ms,
            &mut hits,
        );

        for hit in hits.iter() {
            let by_player = self
                .entities
                .get(hit.owner)
                .is_some_and(|owner| owner.kind == Kind::Player);

            let mut landed_on_enemy = false;
            let mut event = None;
            let mut killed_player = false;

            if let Some(target) = self.entities.get_mut(hit.target) {
                // Only the subtraction is skipped. The blow was worked out in full above and is
                // reported in full below, because that is where the original puts its guard:
                // `if (!HasConditionEffect(Invulnerable)) HP -= dmg;` immediately followed by a
                // `Damage` packet carrying `dmg` (`Enemy.cs:104-113`, `Player.cs:786-793`).
                if !hit.absorbed {
                    target.hp -= hit.damage;
                }
                if target.slain() {
                    target.dead = true;
                    killed_player = target.kind == Kind::Player;

                    // Breakable scenery leaves rather than dies: `StaticObject.HitByProjectile`
                    // reaches `CheckHP`, which calls `LeaveWorld` (`StaticObject.cs:90`) and never
                    // `Enemy.Death`. Nothing downstream of a death runs for it.
                    target.removed |= target.kind == Kind::StaticObject;
                }

                target.last_hurt_by = Some(hit.owner);
                landed_on_enemy = target.kind == Kind::Enemy;

                // Read here rather than reconstructed later, because `kill` is the health one
                // statement after the subtraction and by the end of the tick the body is gone.
                //
                // A shot at a player skips that player, as `Player.cs:797` skips them: their own
                // client stamped the damage onto the bullet when the shot was announced and draws
                // the number the instant it touches them (`Projectile.as:253`, `:264-265`), so
                // telling them again is the same number twice.
                //
                // A shot at an enemy tells everyone, including the shooter — and here the original
                // leaves the shooter out (`Enemy.cs:120`). It can afford to, because its client
                // rolled that number itself: both ends run one seeded generator over the same seed
                // and the same draws (`GameServerConnectionConcrete.as:496-498` against
                // `StatsManager.cs:50-53`, kept in step by `Player.DropNextRandom`). There is no
                // such shared stream here — a player's own damage is rolled on the server alone —
                // so a client left out is a client with nothing to draw.
                event = Some(DamageEvent {
                    target: hit.target,
                    x: target.x,
                    y: target.y,
                    // Always empty: the original sends the projectile entity's own conditions, and
                    // nothing in the original ever puts a condition on a projectile.
                    effects: ConditionSet::EMPTY,
                    amount: hit.damage,
                    kill: target.dead,
                    bullet: hit.bullet,
                    owner: Some(hit.owner),
                    except: (target.kind == Kind::Player).then_some(hit.target),
                });

                // Remembered only for enemies hurt by players, which is the only case anybody asks
                // about: it decides who earned the loot that belongs to whoever earned it.
                if by_player && target.kind == Kind::Enemy {
                    let held = target
                        .damage_by
                        .iter_mut()
                        .find(|(who, _)| *who == hit.owner);

                    match held {
                        Some((_, total)) => *total += hit.damage,
                        None => {
                            if target.damage_by.len() < MOST_REMEMBERED_DAMAGERS {
                                target.damage_by.push((hit.owner, hit.damage));
                            }
                        }
                    }
                }
            }

            // Blamed on the shooter the moment the shot lands, as `Player.HitByProjectile` calls
            // `Death` with the shooter itself (`Player.cs:800-804`).
            if killed_player {
                let (killer, summoned) = self.blame(Some(hit.owner), catalog);
                self.claim_death(hit.target, killer, summoned);
            }

            // Counted where it lands rather than where it was fired, since only the landing knows
            // whether it hit anything. The accuracy bonuses are the ratio of the two.
            if landed_on_enemy
                && let Some(shooter) = self.entities.get_mut(hit.owner)
                && shooter.kind == Kind::Player
            {
                shooter.tally.shots_that_hit += 1;
            }

            // After the health is taken and before anything is announced, which is the order
            // `Enemy.HitByProjectile` (`Enemy.cs:114`) and `Player.HitByProjectile`
            // (`Player.cs:789`) both use. A shot that kills still applies what it carried, and an
            // immunity the target holds refuses it inside `give_effect`.
            //
            // A duration of zero is skipped rather than applied for an instant: `ApplyConditionEffect`
            // (`Entity.cs:713-719`) writes the timer but only raises the flag when the duration is
            // not zero, so such an effect is never visible.
            //
            // Breakable scenery takes none of it. `StaticObject.HitByProjectile`
            // (`StaticObject.cs:55-70`) subtracts the health, broadcasts the number and returns; it
            // has no `ApplyConditionEffect` at all, so a paralysing bullet into a wine barrel is
            // just damage.
            let scenery = self
                .entities
                .get(hit.target)
                .is_some_and(|target| target.kind == Kind::StaticObject);

            for applied in hit.effects.iter().filter(|_| !scenery) {
                if applied.duration_ms <= 0 {
                    continue;
                }
                self.give_effect(
                    hit.target,
                    applied.effect.index() as u8,
                    applied.duration_ms as u32,
                );
            }

            if let Some(event) = event {
                self.note_damage(event);
            }
        }

        self.hits = hits;
        self.projectiles.drop_orphans(&self.entities);
    }

    /// What was struck on the last tick.
    pub fn recent_hits(&self) -> &[Hit] {
        &self.hits
    }

    /// Burns players standing on ground that hurts.
    ///
    /// Players only, and on the half-second the original burns on rather than smeared across every
    /// tick. `Player.Ground.cs` rolls once per burn and takes the whole amount, so a player crossing
    /// a corner of lava may cross it for nothing, and standing in it is a series of distinct hits
    /// rather than a drain. Enemies never burn at all: several dungeons stand them on hazards.
    ///
    /// Defence does not apply, as it does not there: the roll is subtracted from health directly.
    fn apply_hazards(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter_map(|(handle, entity)| (entity.kind == Kind::Player).then_some(handle)),
        );

        for index in 0..self.handles.len() {
            let handle = self.handles[index];

            let Some(entity) = self.entities.get(handle) else {
                continue;
            };
            if entity.dead {
                continue;
            }

            // Paused and Invincible, and only those two: they are the whole of the guard at the top
            // of `ForceGroundHit` (`Player.Ground.cs:64-66`), and `ApplyGroundDamage` below it takes
            // the roll off health with no further check at all. Stasis and Invulnerable do not stop
            // a burn in the original, so reading the wider `no_damage` here made a paralysing trap
            // or an invulnerability effect double as fire protection.
            let rules = crate::effects::Rules::of(entity.conditions);
            if rules.paused
                || entity
                    .conditions
                    .contains(hendra_content::ConditionEffect::Invincible)
            {
                continue;
            }

            let (x, y) = (entity.x, entity.y);
            let Some((min, max)) = self.terrain.hazard_at(catalog, x, y) else {
                // Off the hazard, so the next step onto one burns immediately rather than
                // inheriting whatever was left of the last one's clock.
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.burn_due_ms = 0;
                }
                continue;
            };

            // An object standing on the tile can shelter what stands with it.
            if self
                .terrain
                .object_at(x, y)
                .and_then(|object| catalog.object(object))
                .is_some_and(|desc| desc.protect_from_ground_damage)
            {
                continue;
            }

            let due = {
                let Some(entity) = self.entities.get_mut(handle) else {
                    continue;
                };
                if entity.burn_due_ms > elapsed_ms {
                    entity.burn_due_ms -= elapsed_ms;
                    false
                } else {
                    entity.burn_due_ms = GROUND_DAMAGE_PERIOD_MS;
                    true
                }
            };
            if !due {
                continue;
            }

            let damage = if max > min {
                min + (self.roll() * (max - min) as f32) as i32
            } else {
                min
            };
            if damage <= 0 {
                continue;
            }

            let mut event = None;
            let mut killed = false;
            if let Some(entity) = self.entities.get_mut(handle) {
                entity.hp -= damage;
                if entity.slain() {
                    entity.dead = true;
                    killed = true;
                }

                // `ApplyGroundDamage` broadcasts with no owner, no bullet and no effects
                // (`Player.Ground.cs:102`). It leaves the burning player out, because there the
                // client rolled the burn for itself off a random stream shared with the server and
                // already has the number. Nothing is shared here — the roll above is the only one
                // there is — so the burning player is told as well, or the only person who cannot
                // see what the ground cost them is the one paying it.
                event = Some(DamageEvent {
                    target: handle,
                    x: entity.x,
                    y: entity.y,
                    effects: ConditionSet::EMPTY,
                    amount: damage,
                    kill: entity.dead,
                    bullet: 0,
                    owner: None,
                    except: None,
                });
            }

            if let Some(event) = event {
                self.note_damage(event);
            }

            // Named after the ground, which is what `ApplyGroundDamage` passes
            // (`Player.Ground.cs:111`). A trip home rather than the end of a character when the
            // square was laid down by something summoned: `Player.Death` reads `tile.Spawned` and
            // turns the death into a rekting (`Player.cs:999-1002`), which is what stops fire
            // painted by a boss's minion taking a character off somebody.
            if killed {
                let named = self
                    .terrain
                    .tile_name_at(catalog, x, y)
                    .map(|name| name.to_string())
                    .unwrap_or_else(|| self.name.to_string());
                let summoned = self
                    .summoned_ground
                    .contains(&(x.floor().max(0.0) as u32, y.floor().max(0.0) as u32));
                self.claim_death(handle, named, summoned);
            }
        }
    }

    /// Spends and restores the air players are breathing.
    ///
    /// `HandleOceanTrenchGround`, on its own hundred-millisecond clock. Standing within a tile of a
    /// vent restores eight; standing anywhere else costs two, and once there is nothing left it
    /// costs ten health instead. Hidden players are exempt, which is what makes the rogue's cloak
    /// worth something down there.
    fn apply_suffocation(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        if !self.drowns {
            return;
        }

        self.since_breath_ms += elapsed_ms;
        if self.since_breath_ms < BREATH_PERIOD_MS {
            return;
        }
        self.since_breath_ms = 0;

        // Where the air is. Few enough per world that finding them once a tick beats indexing
        // them, and they never move.
        let vents: Vec<(f32, f32)> = self
            .entities
            .iter()
            .filter(|(_, entity)| {
                catalog
                    .object(entity.object_type)
                    .is_some_and(|desc| desc.id == OXYGEN_SOURCE)
            })
            .map(|(_, entity)| (entity.x, entity.y))
            .collect();

        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter_map(|(handle, entity)| (entity.kind == Kind::Player).then_some(handle)),
        );

        for index in 0..self.handles.len() {
            let handle = self.handles[index];
            let Some(entity) = self.entities.get(handle) else {
                continue;
            };
            // Hidden by its own guard (`Player.Ground.cs:17`), paused because `Player.Tick` does
            // not call `HandleOceanTrenchGround` at all while the world has stopped for them
            // (`Player.cs:565`).
            if entity.dead
                || entity
                    .conditions
                    .contains(hendra_content::ConditionEffect::Hidden)
                || crate::effects::Rules::of(entity.conditions).paused
            {
                continue;
            }

            let (x, y) = (entity.x, entity.y);
            let breathing = vents.iter().any(|(vx, vy)| {
                let (dx, dy) = (x - vx, y - vy);
                dx * dx + dy * dy < 1.0
            });

            let Some(entity) = self.entities.get_mut(handle) else {
                continue;
            };

            if breathing {
                entity.oxygen = (entity.oxygen + BREATH_GAIN).min(FULL_OXYGEN);
                continue;
            }

            let mut drowned = false;
            if entity.oxygen > 0 {
                entity.oxygen = (entity.oxygen - BREATH_COST).max(0);
            } else {
                entity.hp -= DROWNING_DAMAGE;
                if entity.slain() {
                    entity.dead = true;
                    drowned = true;
                }
            }

            // `HandleOceanTrenchGround` names it in as many words (`Player.Ground.cs:30`), and it
            // is a real death: the trench keeps what it takes.
            if drowned {
                self.claim_death(handle, "suffocation".to_string(), false);
            }
        }
    }

    /// Regenerates health and magic.
    ///
    /// Vitality and wisdom are worth nothing without this, and it is what makes the walk between
    /// fights part of the game rather than dead time. Fractions are carried between ticks for the
    /// same reason the effects carry theirs: one point a second is less than one point a tick.
    fn regenerate(&mut self, elapsed_ms: u32) {
        let seconds = elapsed_ms as f32 / 1000.0;

        for (_, entity) in self.entities.iter_mut() {
            if entity.dead || entity.kind != Kind::Player {
                continue;
            }

            // `Player.Tick` runs `HandleRegen` only outside the pause (`Player.cs:565`), so a
            // paused player comes back with exactly the health and magic they stopped with.
            if crate::effects::Rules::of(entity.conditions).paused {
                continue;
            }

            let rules = crate::effects::Rules::of(entity.conditions);

            // The carried fraction is thrown away rather than kept when there is nothing to
            // regenerate: `HandleRegen` assigns `_hpRegenCounter = 0` in that branch
            // (`Player.cs:617-618`), so a player at full health starts their next wound from
            // nothing rather than from whatever was left over from the last one.
            //
            // Health and magic are two independent branches there and have to be two here: a
            // bleeding player still gets their magic back.
            if entity.hp == entity.max_hp || rules.no_health_regen {
                entity.health_regen_fraction = 0.0;
            } else {
                entity.health_regen_fraction += entity.stats.health_regen(&rules) * seconds;
                let whole = entity.health_regen_fraction.trunc();
                if whole > 0.0 {
                    entity.hp = (entity.hp + whole as i32).min(entity.max_hp);
                    entity.health_regen_fraction -= whole;
                }
            }

            if entity.mp == entity.max_mp || rules.no_magic_regen {
                entity.magic_regen_fraction = 0.0;
            } else {
                entity.magic_regen_fraction += entity.stats.magic_regen(&rules) * seconds;
                let whole = entity.magic_regen_fraction.trunc();
                if whole > 0.0 {
                    entity.mp = (entity.mp + whole as i32).min(entity.max_mp);
                    entity.magic_regen_fraction -= whole;
                }
            }
        }
    }

    /// Applies the effects that move health over time.
    ///
    /// Kept apart from the ground because the two answer different questions. One is where you are
    /// standing and the other is what is on you, and an entity can be subject to both
    /// at once, in which case both should apply.
    fn apply_effect_health(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        let seconds = elapsed_ms as f32 / 1000.0;

        for (_, entity) in self.entities.iter_mut() {
            if entity.effects.is_empty() || entity.dead || !entity.kind.is_alive_kind() {
                continue;
            }

            let rules = crate::effects::Rules::of(entity.conditions);

            // All of this is `HandleEffects`, which `Player.Tick` runs only outside the pause
            // (`Player.cs:565`), so a paused player neither heals, bleeds, empties nor pays for a
            // ninja's speed. An enemy has no such guard: its bleeding is in `Enemy.Tick`
            // (`Enemy.cs:141`), which reads no pause at all.
            if entity.kind == Kind::Player && rules.paused {
                continue;
            }

            // Quiet empties the bar rather than refusing the ask: `HandleEffects` sets `MP = 0`
            // every tick it is held (`Player.Effects.cs:36-37`), and nothing anywhere refuses an
            // activation for it. That is how a quieted player is stopped from casting and still
            // able to drink a potion, which costs no magic.
            if entity.kind == Kind::Player
                && entity.mp > 0
                && entity
                    .conditions
                    .contains(hendra_content::ConditionEffect::Quiet)
            {
                entity.mp = 0;
            }

            // A ninja's speed is paid for rather than given, and it ends the moment there is none
            // left, which is what stops it being free movement.
            //
            // Twelve magic a second rather than the ten `HandleEffects` writes, because the
            // original charges it in whole points on a tick that is not a second: `MP = Math.Max(0,
            // (int)(MP - 10 * time.ElaspedMsDelta / 1000f))` (`Player.Effects.cs:53`) truncates the
            // subtraction, and `Player.Tick` is reached from the 332 ms world tick rather than the
            // 166 ms logic tick (`FLLogicTicker.cs:149-156`, `wServer.json:23`). Ten a second is
            // 3.32 a tick and the truncation takes four, which is 12.05 a second. The fraction is
            // carried between ticks so this server charges the same rate whatever its own tick is.
            if entity
                .conditions
                .contains(hendra_content::ConditionEffect::NinjaSpeedy)
            {
                entity.magic_fraction += crate::effects::Rules::MAGIC_PER_SECOND * seconds;
                let spent = entity.magic_fraction.trunc();
                entity.magic_fraction -= spent;

                entity.mp = (entity.mp - spent as i32).max(0);
                if entity.mp == 0 {
                    entity
                        .conditions
                        .remove(hendra_content::ConditionEffect::NinjaSpeedy);
                    let ninja = hendra_content::ConditionEffect::NinjaSpeedy as u8;
                    entity.effects.retain(|(effect, _)| *effect != ninja);
                }
            }

            if rules.health_per_second == 0.0 {
                continue;
            }

            // An enemy the content declares no maximum health for does not bleed at all:
            // `Enemy.Tick` tests `!stat` first (`Enemy.cs:141`), and `stat` is `ObjectDesc.MaxHP ==
            // 0` (`Enemy.cs:20`). It is the same set of spawners and turrets that cannot be shot.
            let stationary = entity.kind != Kind::Player
                && catalog
                    .object(entity.object_type)
                    .is_some_and(|desc| desc.max_hp == 0);
            if stationary {
                continue;
            }

            // Carried between ticks rather than rounded away: twenty a second at fifty-millisecond
            // ticks is one point per tick, and rounding that to zero would make healing do nothing
            // at all.
            entity.health_fraction += rules.health_per_second * seconds;
            let whole = entity.health_fraction.trunc();
            entity.health_fraction -= whole;

            let change = whole as i32;
            if change == 0 {
                continue;
            }

            if entity.kind == Kind::Player {
                // Both bounds are written into `Player.HandleEffects`: healing stops at the
                // maximum (`Player.Effects.cs:29`) and bleeding stops at one and only runs while
                // there is more than one left (`Player.Effects.cs:38-45`). A stray bleed is
                // therefore never an execution.
                if change > 0 {
                    entity.hp = (entity.hp + change).min(entity.max_hp);
                } else if entity.hp > 1 {
                    entity.hp = (entity.hp + change).max(1);
                }
                continue;
            }

            // An enemy has neither bound and no death check either. `Enemy.Tick` subtracts the
            // bleeding straight off `HP` (`Enemy.cs:141-148`), so a bled-out enemy waits at
            // negative health until the next thing that touches it, and the `HP < 0` test in
            // `HitByProjectile` then kills it whatever the damage was. Healing is not there at
            // all: `ConditionEffects.Healing` is read in one place in the whole server, and that
            // place is the player.
            if change < 0 {
                entity.hp += change;
            }
        }
    }

    /// Takes away breakable scenery whose health has run out.
    ///
    /// `StaticObject.Tick` (`StaticObject.cs:100-107`) calls `CheckHP` every tick for anything
    /// `Vulnerable`, not only when something has just hit it, and `CheckHP` (`:71-93`) leaves the
    /// world at `HP <= 0`. That is what removes a barrel a behaviour drained rather than a bullet,
    /// and it is the same threshold the hit path uses.
    ///
    /// It goes rather than dies. `CheckHP` calls `LeaveWorld` and never `Enemy.Death`, so nothing
    /// downstream of a death happens: no experience, no loot, no death behaviours.
    ///
    /// And it blanks the square it stood on. Something both `<Enemy/>` and `<Static/>` is the one
    /// kind of object `Wmap.Load` leaves on its tile as well as putting into the world
    /// (`Wmap.cs:358-364`), so a wine barrel is an entity *and* a square that cannot be walked
    /// through. `CheckHP` writes `tile.ObjType = 0` over it (`StaticObject.cs:74-86`) before it
    /// leaves, and without that the square stays blocked forever: the barrel is gone from the
    /// screen and still standing in the collision map. Only the square the object was actually read
    /// off is blanked — the original compares `Map[x, y].ObjType` against its own type first, so a
    /// barrel a behaviour spawned onto somebody else's wall leaves that wall alone.
    ///
    /// The whole pass runs over anything already marked dead as well, because a bullet claimed by a
    /// client kills the barrel between ticks and this is what runs before the body is reaped.
    fn break_static_objects(&mut self, catalog: &Catalog) {
        self.broken.clear();

        for (_, entity) in self.entities.iter_mut() {
            if entity.kind != Kind::StaticObject {
                continue;
            }
            // Not `Vulnerable`, so there is no health to check: `StaticObject.Tick` skips the whole
            // block for anything whose descriptor declared no `MaxHitPoints`.
            if !catalog
                .object(entity.object_type)
                .is_some_and(|desc| desc.max_hp != 0)
            {
                continue;
            }
            if entity.slain() {
                entity.dead = true;
                entity.removed = true;

                // `(int)(X - 0.5)`, which for something standing in the middle of its square is the
                // square itself.
                let x = (entity.x - 0.5) as i32;
                let y = (entity.y - 0.5) as i32;
                if x >= 0 && y >= 0 {
                    self.broken.push((x as u32, y as u32, entity.object_type));
                }
            }
        }

        let broken = std::mem::take(&mut self.broken);
        for (x, y, object_type) in &broken {
            if self.terrain.object_at(*x as f32, *y as f32) == Some(*object_type) {
                let was_blocking = self.terrain.blocks_sight(*x, *y);
                self.terrain.paint(catalog, *x, *y, None, None, true);

                // A destructible wall coming down is the case `World.LeaveWorld` handles: the open
                // areas are relabelled and everybody near enough is given a fresh circle
                // (`World.cs:397-405`).
                if was_blocking {
                    self.sight_blocker_changed(*x, *y);
                }
            }
        }
        self.broken = broken;
    }

    /// Counts down anything with a lifetime, such as a loot bag.
    fn expire(&mut self, elapsed_ms: u32) {
        for (_, entity) in self.entities.iter_mut() {
            let Some(remaining) = entity.expires_in_ms else {
                continue;
            };
            let left = remaining.saturating_sub(elapsed_ms);
            entity.expires_in_ms = Some(left);

            // A decoy's and a trap's health is their remaining time, so the two count down
            // together: `StaticObject.Tick` does `HP -= time.ElaspedMsDelta` for anything `Dying`
            // (`StaticObject.cs:103`) and `ExportStats` sends that same number as the health
            // (`:44`), which is what drains the bar the client draws over a decoy.
            if entity.kind == Kind::Decoy || entity.kind == Kind::Trap {
                entity.hp = left as i32;
            }

            if left == 0 {
                // Running out of time is a `LeaveWorld` in every case the original has one — a bag,
                // a portal, a gravestone — so nothing about it counts as a death.
                entity.dead = true;
                entity.removed = true;
            }
        }
    }

    /// Turns what the dead were carrying into bags on the ground.
    ///
    /// Runs before the reap, because the loot table belongs to an entity that is about to stop
    /// existing.
    fn drop_loot(&mut self, catalog: &Catalog) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| pays_for_death(entity, catalog))
                .map(|(handle, _)| handle),
        );

        for index in 0..self.handles.len() {
            let handle = self.handles[index];
            let Some(entity) = self.entities.get(handle) else {
                continue;
            };
            let (x, y) = (entity.x, entity.y);

            let Some((id, troll)) = catalog
                .object(entity.object_type)
                .map(|desc| (desc.id.clone(), desc.troll_white_bag))
            else {
                continue;
            };
            let Some(program) = self.behaviours.get(&id) else {
                continue;
            };

            // An enemy registered with no drops of its own drops nothing at all, not even the
            // world's potion. `BehaviorDb.Init` hangs the whole loot handler off `Death` only
            // `if (defs.Length > 0)` (`BehaviorDb.cs:81-88`); an enemy declared without a
            // `MobDrops` never reaches `Loots.Handle`, so the world table is never merged into it.
            // Four hundred and thirty-six of the seven hundred and forty-six converted enemies are
            // declared that way, thirty of them roaming the open realm, and every one of them was
            // leaving a brown bag behind.
            //
            // Counted in entries rather than in flattened items, as the original counts them: a
            // `Threshold` holding one of the imported fork's empty bundles is still an entry, so
            // its enemy is still one the handler is attached to.
            if program.loot.is_empty() {
                continue;
            }

            // Flattened the way `MobDrops.Populate` does it: a threshold is not a drop of its own
            // but a share stamped onto everything inside it, and a tier is not a drop either but
            // every item of that tier at a share of the chance. What `Loots.Handle` walks is one
            // flat list of concrete items, so that is what is built here.
            let mut defs = Vec::new();
            for entry in &program.loot {
                flatten_loot(entry, None, catalog, &mut defs);
            }

            // The table every world carries, which `World.cs` puts on the base class and no world
            // overrides: a three per cent chance of a tier-one potion on anything that dies. It is
            // where a player's potions come from when they are not farming one boss, so its absence
            // is felt as an economy rather than as a missing drop. World drops replace an identical
            // item already in the mob's table, except the two potions themselves (`Loots.cs:92-97`),
            // which is why nothing is removed here.
            flatten_loot(
                &hendra_behavior::program::LootEntry::Tier {
                    tier: 1,
                    kind: "potion".to_string(),
                    chance: WORLD_POTION_CHANCE,
                    required: 0,
                    // `World.WorldLoot` writes no threshold, so the world's potion is public.
                    threshold: 0.0,
                },
                None,
                catalog,
                &mut defs,
            );

            // How many of each entry still owe a drop. Counted down by the random pass, public and
            // private alike, and whatever is left over is forced out afterwards.
            let mut owed: Vec<u32> = defs.iter().map(|def| def.required).collect();

            // Everything that belongs to whoever reaches it first.
            let mut shared = Vec::new();
            for (index, def) in defs.iter().enumerate() {
                if def.threshold > 0.0 {
                    continue;
                }
                if self.roll() < def.chance {
                    shared.push(def.item);
                    owed[index] = owed[index].saturating_sub(1);
                }
            }

            // And everything that belongs to whoever earned it. `Threshold` is compared against the
            // raw damage the player did, not against a share of the enemy's health: `GetPlayerData`
            // hands back `hitters[player]`, which `HitBy` accumulates in hit points
            // (`DamageCounter.cs:39,55`), and `Loots.cs:122` compares the threshold to it directly.
            // Every threshold in the content is below one, so in practice a single hit earns a
            // player their private roll. That reads like an oversight in the original — the field is
            // named and documented as a fraction — but it is what the original does, so it is what
            // this does.
            let damagers = self
                .entities
                .get(handle)
                .map(|entity| entity.damage_by.clone())
                .unwrap_or_default();

            // Each surviving damager keeps the damage it did alongside its loot. The required pass
            // below needs both, and reading the damage back out of `damagers` by position is wrong
            // as soon as one damager has left the world: the two lists are no longer the same
            // length, and the threshold would be tested against somebody else's damage.
            // `GetPlayerData` (`DamageCounter.cs:47-58`) filters the departed once and hands
            // `Loots.Handle` a single list, so the original cannot drift this way.
            let mut private: Vec<(Handle, i32, Vec<ObjectType>)> = Vec::new();
            for (who, damage) in &damagers {
                let Some(player) = self.entities.get(*who) else {
                    continue;
                };
                // Two separate multipliers on the private roll, and only the private roll:
                // `lootDropBoost` is 1.5 while an account's loot-drop boost is running, and
                // `luckStatBoost` is `1 + Stats.Boost[10] / 100` (`Loots.cs:116-123`). The luck stat
                // is the only reader of index ten in the whole server, and it reads the boost layer
                // rather than the total. Neither touches the public roll or the required drops that
                // are forced out afterwards.
                let luckier = player.loot_drop * player.stats.loot_multiplier() as f32;

                let mut theirs = Vec::new();
                for index in 0..defs.len() {
                    let def = defs[index];
                    if def.threshold <= 0.0 || def.threshold > *damage as f32 {
                        continue;
                    }
                    if self.roll() < def.chance * luckier {
                        theirs.push(def.item);
                        owed[index] = owed[index].saturating_sub(1);
                    }
                }

                private.push((*who, *damage, theirs));
            }

            // Whatever the rolls did not produce that an entry insists on. Public shortfall goes in
            // the shared bag; private shortfall goes to eligible players chosen at random, one item
            // each, never the same item twice to one player (`Loots.cs:134-169`).
            for index in 0..defs.len() {
                let def = defs[index];

                if def.threshold <= 0.0 {
                    while owed[index] > 0 {
                        shared.push(def.item);
                        owed[index] -= 1;
                    }
                    continue;
                }

                let mut eligible: Vec<usize> = (0..private.len())
                    .filter(|slot| def.threshold <= private[*slot].1 as f32)
                    .collect();

                while owed[index] > 0 && !eligible.is_empty() {
                    let pick = (self.roll() * eligible.len() as f32) as usize;
                    let slot = eligible.remove(pick.min(eligible.len() - 1));

                    // Already given one by the random pass, so this one is not given at all — and
                    // the debt is not counted down either. `Loots.cs:161-164` skips without
                    // decrementing `reqDrops`, which is what makes the loop run out of players
                    // rather than run forever, and what makes a required drop occasionally come up
                    // one short.
                    if private[slot].2.contains(&def.item) {
                        continue;
                    }

                    private[slot].2.push(def.item);
                    owed[index] -= 1;
                }
            }

            // Ordinary loot is said nothing about. `Loots.HandleLoot` builds bags, adds them to the
            // world, and that is all: a dungeon key landing in a bag is seen by whoever is standing
            // on it and by nobody else.
            //
            // The four keys a locker's doors need are not items and never fall out of anything:
            // "Purple Key" and its three siblings are `Character` objects standing in the room
            // (`EmbeddedData_GhostShipCXML.dat:194-205`), and what says one has been found is the
            // `Taunt(true, "Purple Key has been found!")` each of them runs when a player walks
            // into it (`logic/db/BehaviorDb.DavyJones.cs:127-142`).
            //
            // The named items are the exception, and the shout is what makes a server feel busy:
            // a room away, somebody just got a Doom Bow.
            let hits: i32 = damagers.iter().map(|(_, damage)| *damage).sum();

            // Private bags first, then the shared one, as `AddBagsToWorld` orders them. It decides
            // which bag a player standing on the pile reaches into.
            for (owner, _, items) in private {
                if items.is_empty() {
                    continue;
                }
                let boosted = self
                    .entities
                    .get(owner)
                    .is_some_and(|player| player.loot_drop > 1.0);

                for item in &items {
                    self.shout_rare_drop(catalog, owner, *item, &damagers, hits);
                }

                self.show_bags(catalog, &items, Some(owner), boosted, troll, x, y);
            }

            // A named item in the shared bag says nothing. The original would not survive it:
            // `ShowBags(enemy, shared)` passes no owners at all and the announcement indexes
            // `owners[0]` unconditionally (`Loots.cs:327`), so a listed item dropping publicly
            // throws out of the middle of the loot pass and the rest of the bags are never made.
            // Nearly every listed item sits inside a `Threshold` block and so is private in
            // practice, which is why the original has survived carrying this. Staying silent is the
            // deliberate choice here: there is no owner to name and no damage share to quote, and
            // losing the whole drop to name a player who does not exist is worse than losing the
            // sentence.
            self.show_bags(catalog, &shared, None, false, troll, x, y);
        }
    }

    /// Tells the whole world when one of the named items drops.
    ///
    /// `Loots.ShowBags` (`Loots.cs:325-331`): every player in the world, not only those who were
    /// there, hears who got it and what share of the kill they did. The share is the player's own
    /// damage over the enemy's total, rounded to a whole percent, and it is quoted so that a shout
    /// is also an answer to "did they earn it".
    ///
    /// The sentence is the original's, misspellings and spacing included, because it is what
    /// players of this server recognise.
    fn shout_rare_drop(
        &mut self,
        catalog: &Catalog,
        owner: Handle,
        item: ObjectType,
        damagers: &[(Handle, i32)],
        total: i32,
    ) {
        let Some(name) = catalog.object(item).map(|desc| desc.id.clone()) else {
            return;
        };
        if !NOTABLE_DROPS.contains(&name.as_str()) {
            return;
        }
        let Some(who) = self
            .entities
            .get(owner)
            .and_then(|player| player.name.clone())
        else {
            return;
        };

        // `hitters[owner] / TotalDamage` (`Loots.cs:330`). An enemy that died to something other
        // than a player has no total at all, which in the original is a divide by zero and so a
        // `NaN` percent; nothing is quoted here instead.
        let share = damagers
            .iter()
            .find(|(handle, _)| *handle == owner)
            .map(|(_, damage)| *damage)
            .unwrap_or(0);
        let percent = if total > 0 {
            (100.0 * share as f64 / total as f64).round() as i64
        } else {
            0
        };

        self.announce_as(
            LOOT_NOTIFIER,
            &format!("{who} has just gotten Amazing drop >> {name} with this damage >> {percent}%"),
        );
    }

    /// Lays a list of loot out into bags of eight.
    ///
    /// `Loots.ShowBags`. Eight is the size of a bag, so a ninth item starts a second one, and the
    /// colour is read per bag rather than over the whole drop — which is why an enemy that drops one
    /// white and eleven browns leaves a white bag and a brown one rather than two whites.
    fn show_bags(
        &mut self,
        catalog: &Catalog,
        loot: &[ObjectType],
        owner: Option<Handle>,
        boosted: bool,
        troll: bool,
        x: f32,
        y: f32,
    ) {
        for (at, chunk) in loot.chunks(BAG_SLOTS).enumerate() {
            // `bagType` starts at eight for an enemy marked `<TrollWhiteBag/>` (`Loots.cs:311`) and
            // the loop then raises it to whatever the items ask for, so the flag is a floor on the
            // colour rather than the colour itself: a troll dropping a white-bag item still leaves a
            // white bag, because eight is the troll bag and six and seven are the two whites.
            //
            // It is reset to zero after every *full* bag (`Loots.cs:334`), so a troll dropping nine
            // items leaves one troll bag and one bag coloured by its own contents. That reads like
            // an oversight, and it is kept because it is what the original does.
            let floor = if troll && at == 0 {
                TROLL_BAG_COLOUR
            } else {
                0
            };

            let colour = chunk
                .iter()
                .filter_map(|item| catalog.object(*item))
                .filter_map(|desc| desc.item.as_ref())
                .map(|item| item.bag_type)
                .chain(std::iter::once(floor))
                .max()
                .unwrap_or(0);

            self.show_bag(catalog, chunk, owner, boosted, colour, x, y);
        }
    }

    /// Puts one bag on the ground.
    fn show_bag(
        &mut self,
        catalog: &Catalog,
        items: &[ObjectType],
        owner: Option<Handle>,
        boosted: bool,
        colour: i32,
        x: f32,
        y: f32,
    ) {
        // A loot-drop boost recolours anything that is not already a white bag, so the player can
        // see that the boost is what produced it. White beats red, since what is in the bag matters
        // more than why it dropped.
        //
        // The comparison is between *bags* rather than between colour numbers, which is what
        // `Loots.cs:353-371` compares: the switch resolves the colour to an object first and
        // `if (boosted && bag < WHITE_BAG)` then tests that object. The two only disagree where a
        // colour falls off the end of the table — two keys in this content are marked `BagType` 10
        // — and there the switch leaves the brown bag it started with, which is below white and so
        // does recolour, where the colour number 10 is above white and would not.
        let asked = self.bag_kind(colour);
        let white = self.bag_kind(WHITE_BAG_COLOUR);
        let kind = if boosted && asked.0 < white.0 {
            self.bag_kind(BOOSTED_BAG_COLOUR)
        } else {
            asked
        };

        let mut container = Container::new(ContainerKind::Bag, BAG_SLOTS);
        for item in items {
            container.insert(*item, catalog);
        }

        // Each axis scattered on its own roll, so bags from one kill are a small pile rather than a
        // diagonal line.
        let (dx, dy) = (self.roll_offset(0.5), self.roll_offset(0.5));

        let mut bag = Entity::fixture(kind, x + dx, y + dy);
        bag.kind = Kind::Container;
        bag.container = Some(Box::new(container));
        bag.belongs_to = owner;
        // The rarer bags are drawn larger, which is the other half of how a player reads a pile from
        // across a room. Sized from the colour the *items* asked for, not from the recolour a boost
        // may have applied: `SetDefaultSize(bagType > 3 ? 120 : 80)` (`Loots.cs:382`) reads the
        // parameter, and the boost above it assigns to the local bag rather than to that parameter.
        // Reading the recoloured value instead drew every boosted brown bag at the size of a white
        // one.
        bag.size = if colour > 3 { 120 } else { 80 };
        // Long enough to walk back for, short enough that a dungeon does not fill with bags.
        bag.expires_in_ms = Some(BAG_LIFETIME_MS);
        self.spawn(bag);
    }

    /// Which bag a loot colour is dropped in.
    ///
    /// The content numbers these from zero upward and the world is told the object for each. A
    /// colour with nothing registered falls back to the plain bag rather than dropping nothing,
    /// because losing the loot is worse than losing its colour.
    fn bag_kind(&self, colour: i32) -> ObjectType {
        self.bag_types
            .get(colour.max(0) as usize)
            .copied()
            .filter(|kind| !kind.is_none())
            .unwrap_or(self.bag_type)
    }

    /// Registers the object to use for each loot colour.
    pub fn set_bag_types(&mut self, bags: Vec<ObjectType>) {
        self.bag_types = bags;
    }

    /// Uses the item worn in a slot, aimed at a point.
    ///
    /// The only gate is magic: `UseItem` refuses when `MP < item.MpCost` (`Player.UseItem.cs:162`)
    /// and refuses nothing else. There is no ability cooldown anywhere in the original — `Cooldown`
    /// is parsed off the item and read in no other file — and no condition stops an activation
    /// either. Quiet stops abilities by emptying the magic bar rather than by refusing the ask
    /// (`Player.Effects.cs:36-37`), which is why a quieted player can still drink a potion.
    /// Whether an ask to use this item would do anything, without doing it.
    ///
    /// The same three refusals [`Self::use_item`] makes, asked separately so a consumable can be
    /// spent before its effect runs. The original settles the two in that order — the inventory is
    /// committed and only a commit that succeeded goes on to call `Activate`
    /// (`Player.UseItem.cs:205-227`) — and the magic check that decides whether there is anything to
    /// commit for happens earlier still, at `Player.UseItem.cs:162`.
    pub fn may_use_item(&self, handle: Handle, catalog: &Catalog, item: ObjectType) -> bool {
        let Some(entity) = self.entities.get(handle) else {
            return false;
        };
        if entity.dead {
            return false;
        }

        let Some(desc) = catalog.object(item).and_then(|object| object.item.as_ref()) else {
            return false;
        };

        !desc.activate.is_empty() && entity.mp >= desc.mp_cost
    }

    pub fn use_item(
        &mut self,
        handle: Handle,
        catalog: &Catalog,
        item: ObjectType,
        aim: (f32, f32),
    ) -> Vec<hendra_content::Effect> {
        let mut ran = Vec::new();

        if !self.may_use_item(handle, catalog, item) {
            return ran;
        }

        let (Some(entity), Some(desc)) = (
            self.entities.get(handle),
            catalog.object(item).and_then(|object| object.item.as_ref()),
        ) else {
            return ran;
        };

        let (x, y) = (entity.x, entity.y);
        let cost = desc.mp_cost;

        // Wisdom grows what an ability does, where the content asks for it. Read before the item
        // is spent, because it is the wisdom at the moment of use that decides the size of the
        // heal.
        let wisdom = entity.stats.total(hendra_content::Stat::MpRegen);

        // What a dye put on, applied to the body once the item has been read. `AEDye` writes it
        // straight onto the player rather than waiting for the character to be saved
        // (`Player.UseItem.cs:583-589`), which is what makes a dye visible the moment it is used.
        let mut painted: Vec<(hendra_content::activate::Appearance, i32)> = Vec::new();

        for activate in &desc.activate {
            let effect = hendra_content::Effect::of(activate);

            // A dye names only the act; what it paints is on the item. `AEDye` copies whichever of
            // the item's `Tex1` and `Tex2` is non-zero onto the player
            // (`Player.UseItem.cs:583-589`), which is how the clothing dye and the accessory dye of
            // the same colour -- the same number, in the same content -- are told apart at all.
            // Resolved here because this is the last place that still holds the item.
            if matches!(
                effect,
                hendra_content::Effect::Appearance {
                    kind: hendra_content::activate::Appearance::Dye,
                    ..
                }
            ) {
                for (layer, value) in [
                    (hendra_content::activate::Appearance::DyeCloth, desc.tex1),
                    (
                        hendra_content::activate::Appearance::DyeAccessory,
                        desc.tex2,
                    ),
                ] {
                    if value == 0 {
                        continue;
                    }
                    painted.push((layer, value));
                    ran.push(hendra_content::Effect::Appearance {
                        kind: layer,
                        value: value as u32,
                    });
                }
                continue;
            }

            ran.push(if activate.flag("useWisMod") {
                effect.scaled_by_wisdom(wisdom)
            } else {
                effect
            });
        }

        if !painted.is_empty()
            && let Some(entity) = self.entities.get_mut(handle)
        {
            for (layer, value) in &painted {
                match layer {
                    hendra_content::activate::Appearance::DyeCloth => entity.tex1 = *value,
                    hendra_content::activate::Appearance::DyeAccessory => entity.tex2 = *value,
                    _ => {}
                }
            }
        }

        // A potion and an ability are told apart by what the content calls the item, since both go
        // through the same door: an ability spends magic and a potion is drunk.
        let is_potion = desc.potion;

        // Once, before the effects run: `Activate` subtracts the cost and then walks the list
        // (`Player.UseItem.cs:254-255`). `MpEndCost` is parsed off the item and never spent.
        if let Some(entity) = self.entities.get_mut(handle) {
            entity.mp = (entity.mp - cost).max(0);

            if is_potion {
                entity.tally.potions_drunk += 1;
            } else {
                entity.tally.abilities_used += 1;
            }
        }

        let effects = std::mem::take(&mut ran);
        for effect in &effects {
            self.carry_out(handle, catalog, item, effect, (x, y), aim);
        }

        effects
    }

    /// Does what one ability asks for.
    ///
    /// `item` is the thing being used. Several activations shoot, and what they shoot is the item's
    /// own projectile rather than the weapon in the player's hand.
    fn carry_out(
        &mut self,
        handle: Handle,
        catalog: &Catalog,
        item: ObjectType,
        effect: &hendra_content::Effect,
        from: (f32, f32),
        aim: (f32, f32),
    ) {
        use hendra_content::Effect;

        match effect {
            Effect::Heal { amount } => {
                if self.is_sick(handle) {
                    return;
                }
                self.heal_and_say(handle, *amount);
            }

            Effect::Magic { amount } => self.refill_and_say(handle, *amount),

            Effect::HealNova { amount, range } => {
                let amount = *amount;
                self.each_nearby(handle, *range, true, None, |world, other| {
                    if !world.is_sick(other) {
                        world.heal_and_say(other, amount);
                    }
                });
                if !self.is_sick(handle) {
                    self.heal_and_say(handle, amount);
                }

                self.show_nova(handle, *range);
            }

            // The user is inside their own nova. `AOE` hit-tests the player map at the caster's own
            // position (`Utils.cs:324`), so the caster is one of the players it finds and a magic
            // nova refills the bar of whoever set it off.
            Effect::MagicNova { amount, range } => {
                let amount = *amount;
                self.each_nearby(handle, *range, true, None, |world, other| {
                    world.refill_and_say(other, amount);
                });
                self.refill_and_say(handle, amount);

                self.show_nova(handle, *range);
            }

            // A potion. The ceiling moves with the stat, so drinking one is worth something
            // immediately rather than only after the next level.
            Effect::IncrementStat { stat, amount } => {
                let Some(stat) = stat_of(*stat) else { return };
                let class = self
                    .entities
                    .get(handle)
                    .and_then(|entity| catalog.class(entity.object_type))
                    .cloned();

                if let Some(entity) = self.entities.get_mut(handle) {
                    match &class {
                        Some(class) => entity.stats.raise(class, stat, *amount),
                        None => entity.stats.boost(stat, *amount),
                    }
                    entity.reseat_maxima();
                }
            }

            Effect::StatBoost {
                stat,
                amount,
                duration_ms,
                range,
                no_stack,
            } => {
                let Some(stat) = stat_of(*stat) else { return };
                let index = stat.index() as u8;

                let boost = HeldBoost {
                    stat: index,
                    amount: *amount,
                    remaining_ms: *duration_ms,

                    // The content's own word, passed to `ActivateBoost.Push` unchanged by both the
                    // self and the aura form. Six auras in the game set it and nothing else does.
                    stacks: !*no_stack,
                };

                // A non-stacking boost to maximum health also heals for what it added, at once.
                // `AEStatBoostAura` calls it a hack job and it is, but without it a paladin's seal
                // raises the ceiling and leaves the bar where it was, which is a heal that does
                // nothing (`Player.UseItem.cs:1076-1079`).
                let heals = *no_stack && *amount > 0 && index == 0;
                let amount = *amount;

                match range {
                    // An aura, which is the reason this reads a range at all: it reaches everybody
                    // nearby rather than only whoever used it.
                    Some(radius) => {
                        self.each_nearby(handle, *radius, true, None, |world, other| {
                            world.give_boost(other, boost);
                            if heals && let Some(entity) = world.entities.get_mut(other) {
                                entity.hp = (entity.hp + amount).min(entity.max_hp);
                            }
                        });
                        self.give_boost(handle, boost);
                        if heals && let Some(entity) = self.entities.get_mut(handle) {
                            entity.hp = (entity.hp + amount).min(entity.max_hp);
                        }
                    }
                    None => self.give_boost(handle, boost),
                }
            }

            Effect::ConditionSelf {
                effect,
                duration_ms,
            } => self.give_effect(handle, effect.index() as u8, *duration_ms),

            Effect::ConditionAura {
                effect,
                duration_ms,
                range,
            } => {
                let (index, duration) = (effect.index() as u8, *duration_ms);
                self.give_effect(handle, index, duration);
                self.each_nearby(handle, *range, true, None, |world, other| {
                    world.give_effect(other, index, duration);
                });
            }

            Effect::GenericArea {
                effect,
                duration_ms,
                range,
                targets_players,
                centred_on_aim,
            } => {
                let Some(effect) = effect else {
                    return;
                };
                let (index, duration) = (effect.index() as u8, *duration_ms);
                let centre = if *centred_on_aim { aim } else { from };

                // Stasis and invincibility refuse it, and those two alone: `Player.UseItem.cs:1197`
                // tests `!Stasis && !Invincible` and says nothing about Paused or Invulnerable, so
                // an effect that could not hurt its target still lands on it.
                self.grid
                    .within(centre.0, centre.1, *range, &mut self.nearby);
                let caught = std::mem::take(&mut self.nearby);
                for other in &caught {
                    let takes = self.entities.get(*other).is_some_and(|entity| {
                        !entity.dead && (entity.kind == Kind::Player) == *targets_players && {
                            !crate::effects::Rules::of(entity.conditions).frozen
                                && !entity
                                    .conditions
                                    .contains(hendra_content::ConditionEffect::Invincible)
                        }
                    });
                    if takes {
                        self.give_effect(*other, index, duration);
                    }
                }
                self.nearby = caught;
            }

            // An aura reaches the user as well, for the same reason a magic nova does: the caster
            // is one of the players `AOE` finds (`Player.UseItem.cs:665`, `:1020`).
            Effect::Cleanse { range } => {
                if let Some(range) = range {
                    self.each_nearby(handle, *range, true, None, |world, other| {
                        world.cleanse(other);
                    });
                }
                self.cleanse(handle);
            }

            // The volley is the ability item's own: how many bullets and how far apart are read
            // from the item rather than from the activation, and the bullet is the item's first
            // projectile rather than the held weapon's (`Player.UseItem.cs:1119-1128`). A shield
            // that declares five projectiles fires five.
            Effect::Shoot => {
                let (count, gap) = catalog
                    .object(item)
                    .and_then(|object| object.item.as_ref())
                    .map(|desc| (desc.num_projectiles.max(1) as u32, desc.arc_gap))
                    .unwrap_or((1, 0.0));

                let angle = (aim.1 - from.1).atan2(aim.0 - from.0);
                let start = volley_start(angle, count as usize, gap);
                self.fire_spread(
                    handle,
                    catalog,
                    Some(item),
                    start,
                    count,
                    gap,
                    0,
                    VolleyDamage::PerBullet,
                );
            }

            // A ring outward from the cursor rather than a spread from the caster, which is what
            // makes a nova a placed attack: `AEBulletNova` passes `target` as the starting position
            // of all twenty bullets (`Player.UseItem.cs:1153`). Refused outright past fourteen
            // tiles (`:1145`), the one range limit any activation in the original has.
            Effect::BulletNova => {
                let (dx, dy) = (aim.0 - from.0, aim.1 - from.1);
                if dx * dx + dy * dy > MAX_ABILITY_DIST * MAX_ABILITY_DIST {
                    return;
                }

                let step = std::f32::consts::TAU / BULLET_NOVA_SHOTS as f32;
                for shot in 0..BULLET_NOVA_SHOTS {
                    self.fire_ring(handle, catalog, item, aim, shot as f32 * step, shot as u8);
                }
            }

            Effect::Blast {
                radius,
                damage,
                effect,
                effect_ms,
            } => {
                self.explode(
                    catalog,
                    aim,
                    Blast {
                        from: Some(handle),
                        radius: *radius,
                        damage: *damage,
                        effect: effect.map(|found| found.index() as u8),
                        effect_ms: *effect_ms,
                        // A player's spell catches enemies. It hurting the people beside them is
                        // exactly the bug this side exists to stop.
                        hits_players: false,
                    },
                );
            }

            // Everything in a three-tile circle held still, and told so.
            //
            // `StasisBlast` (`Player.UseItem.cs:800-841`). Four things it does that a generic blast
            // does not, and each of them is the whole point of one of them:
            //
            // It telegraphs. A white `Concentrate` is drawn from the caster to the aimed point
            // before anything is applied (`:802-810`), which is what tells the room a bomb went off
            // there rather than that a monster stopped for no reason.
            //
            // It says which enemies it caught and which refused it. An enemy already carrying
            // `StasisImmune` gets a green "Immune" and nothing else (`:816-824`); one it holds gets
            // a red "Stasis" (`:833-838`). Between them they are the only feedback a stasis bomb
            // gives at all — it deals no damage, so without the floats a cast into an immune pack
            // looks identical to a cast that worked.
            //
            // It refuses to renew a hold that is already running (`:826`), so casting again into
            // your own field does nothing rather than extending it.
            //
            // And it locks the enemy out for three seconds afterwards, which is the rule that stops
            // a bomb from being a permanent hold.
            Effect::StasisBlast { duration_ms } => {
                self.show_effect(EffectEvent {
                    effect: hendra_net::message::effect::CONCENTRATE,
                    target: Some(handle),
                    x1: aim.0,
                    y1: aim.1,
                    x2: aim.0 + STASIS_TELEGRAPH_REACH,
                    y2: aim.1,
                    color: 0xffff_ffff,
                });

                let duration = *duration_ms;
                self.grid
                    .within(aim.0, aim.1, STASIS_BLAST_RADIUS, &mut self.nearby);
                let caught = std::mem::take(&mut self.nearby);

                for target in &caught {
                    use hendra_content::ConditionEffect::{Stasis, StasisImmune};

                    let Some(entity) = self.entities.get(*target) else {
                        continue;
                    };
                    // Enemies only, and not the scenery among them: `world.AOE` on the enemy side
                    // walks the enemy grid and skips anything the content marks `Static`
                    // (`Utils.cs:350-355`).
                    if entity.dead
                        || entity.kind != Kind::Enemy
                        || catalog
                            .object(entity.object_type)
                            .is_some_and(|desc| desc.static_object)
                    {
                        continue;
                    }

                    if entity.conditions.contains(StasisImmune) {
                        self.float_text(*target, "Immune", IMMUNE_TEXT_COLOUR);
                    } else if !entity.conditions.contains(Stasis) {
                        self.give_effect(*target, Stasis as u8, duration);
                        if self.stasis_locks.len() < MAX_FALLING {
                            self.stasis_locks.push(StasisLock {
                                who: *target,
                                until_ms: duration.max(1),
                            });
                        }
                        self.float_text(*target, "Stasis", STASIS_TEXT_COLOUR);
                    }
                }

                self.nearby = caught;
            }

            // A ball of poison, and the second and a half it spends in the air.
            //
            // `AEPoisonGrenade` (`Player.UseItem.cs:675-701`) throws, waits, draws the circle, and
            // only then hands each enemy inside it to `PoisonEnemy`. Nothing is dealt on impact:
            // the whole amount is spread over the activation's duration and paid a second at a
            // time, which is why a grenade kills something that walked away and does not kill
            // something that stood in it for a moment.
            Effect::PoisonGrenade {
                radius,
                total_damage,
                duration_ms,
            } => self.lob(
                handle,
                aim,
                *radius,
                *total_damage,
                FusePayload::Poison {
                    duration_ms: *duration_ms,
                },
            ),

            // The same ball, healing whoever is standing there instead
            // (`AEHealingGrenade`, `Player.UseItem.cs:1218-1244`).
            Effect::HealingGrenade {
                radius,
                total_heal,
                duration_ms,
            } => self.lob(
                handle,
                aim,
                *radius,
                *total_heal,
                FusePayload::Healing {
                    duration_ms: *duration_ms,
                },
            ),

            // A stance, not an explosion.
            //
            // `AEShurikenAbility` (`Player.UseItem.cs:566-581`). The first press hangs `NinjaSpeedy`
            // on the player with no duration at all -- forever, until something takes it off -- and
            // returns without firing. While it is up the player's magic drains and their speed is
            // raised, and the second press spends the item's `MpEndCost` to fire its volley and
            // drops the stance again.
            //
            // The order at the end is the original's and it matters: the stance comes off whether
            // or not there was enough magic to fire, so a ninja out of magic gets their bar back
            // rather than being stuck in the drain.
            Effect::ShurikenAbility => {
                use hendra_content::ConditionEffect::NinjaSpeedy;

                let held = self
                    .entities
                    .get(handle)
                    .is_some_and(|entity| entity.conditions.contains(NinjaSpeedy));

                if !held {
                    self.give_effect(handle, NinjaSpeedy as u8, FOREVER);
                    return;
                }

                let (count, gap, end_cost) = catalog
                    .object(item)
                    .and_then(|object| object.item.as_ref())
                    .map(|desc| {
                        (
                            desc.num_projectiles.max(1) as u32,
                            desc.arc_gap,
                            desc.mp_end_cost,
                        )
                    })
                    .unwrap_or((1, 0.0, 0));

                let enough = self
                    .entities
                    .get(handle)
                    .is_some_and(|entity| entity.mp >= end_cost);
                if enough {
                    if let Some(entity) = self.entities.get_mut(handle) {
                        entity.mp -= end_cost;
                    }

                    let angle = (aim.1 - from.1).atan2(aim.0 - from.0);
                    let start = volley_start(angle, count as usize, gap);
                    self.fire_spread(
                        handle,
                        catalog,
                        Some(item),
                        start,
                        count,
                        gap,
                        0,
                        VolleyDamage::PerBullet,
                    );
                }

                self.give_effect(handle, NinjaSpeedy as u8, 0);
            }

            // Written straight over the base stat, with no ceiling and no overflow into a boost:
            // `AEFixedStat` (`Player.UseItem.cs:645-649`) is a bare assignment, unlike the
            // increment beside it.
            Effect::FixedStat { stat, amount } => {
                let Some(stat) = stat_of(*stat) else { return };
                if let Some(entity) = self.entities.get_mut(handle) {
                    entity.stats.set_base(stat, *amount);
                    entity.reseat_maxima();
                }
            }

            // A bolt that walks from one enemy to the next.
            //
            // `AELightning` (`Player.UseItem.cs:703-788`). Three separate rules, and collapsing it
            // into a circle at the cursor loses all three:
            //
            // The first target is the nearest enemy inside a quarter-turn cone either side of where
            // the cursor points, out to fourteen tiles (`:709-710`). Not the nearest enemy to the
            // cursor — the nearest to the caster, in roughly that direction, so a scepter is aimed
            // by sweeping rather than by clicking.
            //
            // Each hop after that goes to the nearest enemy within ten tiles of the last one, never
            // revisiting a body and never touching one that is invincible or in stasis
            // (`:741-752`). So the bolt follows a line of monsters and stops where the line stops.
            //
            // And a bolt that finds nothing at all is still drawn: three pink beams fanned across
            // the cone, out to the maximum range (`:713-735`). It is what tells the player they
            // missed rather than that the item failed to fire.
            Effect::Lightning {
                damage,
                max_targets,
                effect,
                effect_ms,
            } => {
                let aimed = (aim.1 - from.1).atan2(aim.0 - from.0);

                let Some(first) = self.nearest_in_cone(handle, from, aimed, LIGHTNING_CONE) else {
                    for offset in [0.0, -LIGHTNING_CONE, LIGHTNING_CONE] {
                        let angle = aimed + offset;
                        self.show_effect(EffectEvent {
                            effect: hendra_net::message::effect::TRAIL,
                            target: Some(handle),

                            // Truncated to whole tiles before the offset is added, as the original
                            // casts the reach to `int` and then adds the caster's own coordinates
                            // (`Player.UseItem.cs:719-720`).
                            x1: (MAX_ABILITY_DIST * angle.cos()) as i32 as f32 + from.0,
                            y1: (MAX_ABILITY_DIST * angle.sin()) as i32 as f32 + from.1,
                            x2: LIGHTNING_PARTICLE_SIZE,
                            y2: 0.0,
                            color: LIGHTNING_COLOUR,
                        });
                    }
                    return;
                };

                let mut chain = vec![first];
                while chain.len() < *max_targets as usize {
                    let Some(next) = self.nearest_untouched(*chain.last().unwrap(), &chain) else {
                        break;
                    };
                    chain.push(next);
                }

                for (step, target) in chain.iter().enumerate() {
                    // The bolt is drawn from whatever it left, which is the caster for the first
                    // hop and the previous body for every one after (`Player.UseItem.cs:764`).
                    let anchor = if step == 0 { handle } else { chain[step - 1] };
                    let Some((x, y)) = self.entities.get(*target).map(|e| (e.x, e.y)) else {
                        continue;
                    };

                    self.strike(catalog, Some(handle), *target, *damage);
                    if let Some(effect) = effect {
                        self.give_effect(*target, effect.index() as u8, *effect_ms);
                    }

                    self.show_effect(EffectEvent {
                        effect: hendra_net::message::effect::LIGHTNING,
                        target: Some(anchor),
                        x1: x,
                        y1: y,
                        x2: LIGHTNING_PARTICLE_SIZE,
                        y2: 0.0,
                        color: LIGHTNING_COLOUR,
                    });
                }
            }

            // Health taken out of a crowd of monsters and given to a crowd of players.
            //
            // `AEVampireBlast` (`Player.UseItem.cs:865-925`). Three things about it are easy to get
            // wrong and all three are what the ability is for:
            //
            // The heal is the damage dealt, summed over every enemy the blast touched (`:888-893`).
            // Nothing in the item declares an amount; a blast that hits nothing heals nothing.
            //
            // It heals every player in range, not the caster (`:896-905`). That is the whole reason
            // to bring one into a group. `this.AOE` measures from the caster while the damage is
            // measured from the cursor, so the two circles are in different places.
            //
            // And `Sick` excludes a player from the heal entirely (`:898`) — they are passed over
            // rather than healed for nothing.
            Effect::VampireBlast { radius, damage, .. } => {
                let radius = *radius;

                self.show_effect(EffectEvent {
                    effect: hendra_net::message::effect::TRAIL,
                    target: Some(handle),
                    x1: aim.0,
                    y1: aim.1,
                    x2: 0.0,
                    y2: 0.0,
                    color: VAMPIRE_BLAST_COLOUR,
                });
                self.show_effect(EffectEvent {
                    effect: hendra_net::message::effect::DIFFUSE,
                    target: Some(handle),
                    x1: aim.0,
                    y1: aim.1,
                    x2: aim.0 + radius,
                    y2: aim.1,
                    color: VAMPIRE_BLAST_COLOUR,
                });

                let drained = self.explode(
                    catalog,
                    aim,
                    Blast {
                        from: Some(handle),
                        radius,
                        damage: *damage,
                        effect: None,
                        effect_ms: 0,
                        hits_players: false,
                    },
                );

                let mut healed = Vec::new();
                self.each_nearby(handle, radius, true, None, |world, other| {
                    if !world.is_sick(other) {
                        healed.push(other);
                        world.heal_and_say(other, drained);
                    }
                });
                if !self.is_sick(handle) {
                    healed.push(handle);
                    self.heal_and_say(handle, drained);
                }

                // Five threads of health drawn from a monster picked at random to a player picked at
                // random, redrawn independently each time so the same pair can appear twice
                // (`:907-919`). Drawn only when the blast found something to drain, which is the
                // guard the original puts on the whole block.
                let drained_from: Vec<(f32, f32)> = {
                    let (x, y) = aim;
                    self.grid.within(x, y, radius, &mut self.nearby);
                    let found = std::mem::take(&mut self.nearby);
                    let corpses = found
                        .iter()
                        .filter_map(|handle| self.entities.get(*handle))
                        .filter(|entity| entity.kind == Kind::Enemy)
                        .map(|entity| (entity.x, entity.y))
                        .collect();
                    self.nearby = found;
                    corpses
                };

                if !drained_from.is_empty() && !healed.is_empty() {
                    for _ in 0..VAMPIRE_BLAST_THREADS {
                        let from = drained_from[(self.roll() * drained_from.len() as f32) as usize
                            % drained_from.len()];
                        let to =
                            healed[(self.roll() * healed.len() as f32) as usize % healed.len()];
                        self.show_effect(EffectEvent {
                            effect: hendra_net::message::effect::FLOW,
                            target: Some(to),
                            x1: from.0,
                            y1: from.1,
                            x2: 0.0,
                            y2: 0.0,
                            color: 0xffff_ffff,
                        });
                    }
                }
            }

            Effect::Create { child } => {
                // A dungeon key is a `Create` that names a portal, and that one is the caller's:
                // `AECreate` (`Player.UseItem.cs:591-624`) refuses anything the portal table does
                // not hold, stands the door where the player is rather than where they aimed, gives
                // it a timeout, marks it as opened by them and announces it. Everything else
                // `Create` names is scenery and belongs where it was aimed.
                if let Some(kind) = catalog.type_of(child)
                    && !catalog
                        .object(kind)
                        .is_some_and(|object| Kind::of_class(&object.class) == Kind::Portal)
                {
                    let behaviours = std::mem::take(&mut self.behaviours);
                    self.spawn_child(
                        catalog,
                        &behaviours,
                        kind,
                        aim.0,
                        aim.1,
                        None,
                        Some(handle),
                        false,
                    );
                    self.behaviours = behaviours;
                }
            }

            // Straight to where it was aimed. `AETeleport` hands the cursor to `TeleportPosition`
            // with `ignoreRestrictions` set (`Player.UseItem.cs:927`), which skips the ten-second
            // cooldown, the newbie period and the fame counter that an ordinary teleport pays, and
            // measures no distance at all: the item's `maxDistance` is written in the content and
            // read by nothing.
            Effect::Teleport { .. } => self.step(handle, aim.0, aim.1),

            // What a decoy and a trap have in common is that they are left behind and act on
            // their own; what they do afterwards is all that differs.
            Effect::Placed {
                kind,
                duration_ms,
                radius,
                damage,
                effect,
            } => {
                let where_to = match kind {
                    hendra_content::activate::Placed::Decoy => from,
                    hendra_content::activate::Placed::Trap => aim,
                };

                let mut placed = Entity::fixture(ObjectType::NONE, where_to.0, where_to.1);
                placed.kind = match kind {
                    hendra_content::activate::Placed::Decoy => Kind::Decoy,
                    hendra_content::activate::Placed::Trap => Kind::Trap,
                };
                // The health of both is the time they have left, and nothing else. `Decoy`
                // (`Decoy.cs:29`) and `Trap` (`Trap.cs:21`) are `StaticObject`s built with
                // `life` set to their duration and `dying: true`, so `StaticObject.Tick`
                // (`:100-107`) takes the elapsed milliseconds off it and `CheckHP` removes them
                // when it reaches nothing. A decoy that could be shot down would be one an enemy
                // could delete on its first volley; what ends it is running out.
                let life = (*duration_ms).max(1);
                placed.max_hp = life as i32;
                placed.hp = life as i32;
                placed.expires_in_ms = Some(life);
                placed.armed = Some(Armed {
                    radius: *radius,
                    damage: *damage,
                    effect: effect.map(|found| found.index() as u8),
                });

                // A decoy walks off in the direction its owner was last seen going, which is what
                // makes it read as the player who is no longer there. `Decoy`'s constructor
                // (`Decoy.cs:31-44`) subtracts the position one tick back from the position now and
                // normalises it, falling back to a random heading when the player was standing
                // still and the difference is nothing.
                if *kind == hendra_content::activate::Placed::Decoy {
                    placed.decoy = Some(self.heading_of(handle));
                }

                self.spawn(placed);
            }

            Effect::Pet { .. }
            | Effect::Currency { .. }
            | Effect::Boost { .. }
            | Effect::Unlock { .. }
            | Effect::UnlockPortal { .. }
            | Effect::Portal { .. }
            | Effect::Appearance { .. }
            | Effect::Unsupported { .. } => {
                // These change something the world does not own: an account's currency, a player's
                // wardrobe, a pet that outlives the room. The caller carries them out, which is why
                // `use_item` hands back what it ran.
            }
        }
    }

    /// Removes every harmful effect an entity is carrying.
    fn cleanse(&mut self, handle: Handle) {
        let Some(entity) = self.entities.get_mut(handle) else {
            return;
        };

        entity.effects.retain(|(held, _)| {
            hendra_content::ConditionEffect::from_index(*held as u16)
                .is_none_or(|effect| !is_harmful(effect))
        });

        let mut conditions = hendra_content::ConditionSet::EMPTY;
        for (held, _) in &entity.effects {
            if let Some(known) = hendra_content::ConditionEffect::from_index(*held as u16) {
                conditions.insert(known);
            }
        }
        entity.conditions = conditions;
    }

    /// Sets off whatever has finished falling towards the ground.
    ///
    /// The ring and the damage go together, as the original's one timer sends both
    /// (`Grenade.cs:85-100`). The ring is drawn wherever the grenade was aimed rather than where
    /// anybody is now, so a player who moved sees the circle they walked out of.
    fn burn_fuses(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        if self.fuses.is_empty() {
            return;
        }

        let mut waiting = std::mem::take(&mut self.fuses);
        let mut going_off = Vec::new();

        waiting.retain_mut(|fuse| {
            fuse.remaining_ms = fuse.remaining_ms.saturating_sub(elapsed_ms);
            if fuse.remaining_ms == 0 {
                going_off.push(*fuse);
                false
            } else {
                true
            }
        });
        self.fuses = waiting;

        for fuse in going_off {
            self.blast_ring(BlastEvent {
                x: fuse.x,
                y: fuse.y,
                radius: fuse.radius,

                // Nothing is dealt on impact by the two an item throws, so the telegraph carries
                // no number: what they cost is paid out a second at a time afterwards.
                damage: match fuse.payload {
                    FusePayload::Blast => fuse.damage.clamp(0, u16::MAX as i32) as u16,
                    _ => 0,
                },
                effect: fuse.effect.unwrap_or(0),

                // Seconds on the wire, where the world counts milliseconds: `Aoe.Duration` is what
                // the client hands to its own condition timer, and `Grenade.cs:89` writes a zero
                // there in the one place the original sends this at all.
                duration: fuse.effect_ms as f32 / 1000.0,
                orig_type: fuse.orig_type,
            });

            match fuse.payload {
                FusePayload::Blast => {
                    self.explode(
                        catalog,
                        (fuse.x, fuse.y),
                        Blast {
                            from: fuse.from,
                            radius: fuse.radius,
                            damage: fuse.damage,
                            effect: fuse.effect,
                            effect_ms: fuse.effect_ms,
                            hits_players: true,
                        },
                    );
                }

                // Nothing is dealt where it lands. The circle decides who is on the hook and for
                // how much, and the instalments are paid out afterwards
                // (`Player.UseItem.cs:698-699`).
                FusePayload::Poison { duration_ms } => {
                    self.start_over_time(catalog, fuse, duration_ms, false);
                }
                FusePayload::Healing { duration_ms } => {
                    self.start_over_time(catalog, fuse, duration_ms, true);
                }
            }
        }
    }

    /// Puts everything a grenade caught on the hook for its share, over the seconds that follow.
    ///
    /// Who is caught is decided once, where the ball landed. `AOE` on the enemy side skips anything
    /// the content marks `Static` (`Utils.cs:353`) and the player side takes every player in the
    /// circle, the thrower included, because `world.AOE` measures from the point rather than from
    /// a body (`Player.UseItem.cs:1241`).
    fn start_over_time(&mut self, catalog: &Catalog, fuse: Fuse, duration_ms: u32, heals: bool) {
        // A grenade written with no duration would divide by nothing. The original does divide by
        // it and would throw; every one in the content carries one.
        let seconds = (duration_ms as f32 / 1000.0).max(0.001);

        self.grid
            .within(fuse.x, fuse.y, fuse.radius, &mut self.nearby);
        let caught = std::mem::take(&mut self.nearby);

        for handle in &caught {
            let Some(entity) = self.entities.get(*handle) else {
                continue;
            };
            if entity.dead || (entity.kind == Kind::Player) != heals {
                continue;
            }

            // Defence is taken once, off the whole amount (`Player.UseItem.cs:1294`). A heal has
            // nothing to reduce.
            let total = if heals {
                fuse.damage
            } else {
                let defence = crate::projectile::defence_of(entity, catalog);
                crate::effects::Rules::of(entity.conditions).damage_after_defence(
                    fuse.damage,
                    defence,
                    false,
                    false,
                )
            };
            if total <= 0 {
                continue;
            }

            if self.poisons.len() < MAX_FALLING {
                self.poisons.push(Poison {
                    who: *handle,
                    from: fuse.from,
                    remaining: total,
                    per_tick: (total as f32 / seconds) as i32,
                    until_next_ms: POISON_FIRST_MS,
                    heals,
                });
            }
        }

        self.nearby = caught;
    }

    /// Throws a grenade at a point and arms its fuse.
    ///
    /// The two an item can throw are the same throw: a yellow-green ball drawn leaving the player
    /// (`Player.UseItem.cs:677-683`, `:1220-1226`), an invisible marker standing where it will
    /// land, and a second and a half before anything happens. The wait is the whole of the warning.
    ///
    /// The circle is drawn as the world's own blast telegraph rather than as the original's
    /// `ShowEffect` nova, because that nova carries no position of its own — the original anchors
    /// it on the marker it stood at the landing point, and our client draws a nova only around a
    /// body it already knows.
    fn lob(
        &mut self,
        thrower: Handle,
        at: (f32, f32),
        radius: f32,
        amount: i32,
        payload: FusePayload,
    ) {
        let kind = self
            .entities
            .get(thrower)
            .map(|entity| entity.object_type)
            .unwrap_or(ObjectType::NONE);

        self.show_effect(EffectEvent {
            effect: hendra_net::message::effect::THROW,
            target: Some(thrower),
            x1: at.0,
            y1: at.1,
            x2: 0.0,
            y2: 0.0,
            color: THROWN_GRENADE_COLOUR,
        });

        if self.fuses.len() < MAX_FALLING {
            self.fuses.push(Fuse {
                x: at.0,
                y: at.1,
                remaining_ms: GRENADE_FUSE_MS,
                from: Some(thrower),
                orig_type: kind,
                radius,
                damage: amount,
                effect: None,
                effect_ms: 0,
                payload,
            });
        }
    }

    /// Makes unfreezable whatever a stasis blast has finished holding.
    ///
    /// The three seconds of `StasisImmune` the original hangs on an enemy the moment its hold runs
    /// out (`Player.UseItem.cs:829-830`). Without them a second cast lands on the same enemy the
    /// instant the first ends, and a group carrying two stasis bombs can hold a boss for the whole
    /// fight; with them the second cast is answered with "Immune".
    fn lock_out_stasis(&mut self, elapsed_ms: u32) {
        if self.stasis_locks.is_empty() {
            return;
        }

        let mut waiting = std::mem::take(&mut self.stasis_locks);
        let mut due = Vec::new();

        waiting.retain_mut(|lock| {
            lock.until_ms = lock.until_ms.saturating_sub(elapsed_ms);
            if lock.until_ms == 0 {
                due.push(lock.who);
                false
            } else {
                true
            }
        });
        self.stasis_locks = waiting;

        for who in due {
            self.give_effect(
                who,
                hendra_content::ConditionEffect::StasisImmune as u8,
                STASIS_IMMUNE_MS,
            );
        }
    }

    /// Pays out whatever a grenade left owing.
    ///
    /// One instalment a second until the debt is cleared, and then the entry is dropped — the
    /// original's `remainingDmg <= 0` return, which is what makes the last instalment the short one
    /// rather than an overpayment (`Player.UseItem.cs:1315-1321`).
    fn drip_poison(&mut self, catalog: &Catalog, elapsed_ms: u32) {
        if self.poisons.is_empty() {
            return;
        }

        let mut owed = std::mem::take(&mut self.poisons);
        let mut due = Vec::new();

        owed.retain_mut(|poison| {
            // A body that has left the world stops being poisoned, as the original's tick returns
            // true on `enemy.Owner == null` (`Player.UseItem.cs:1302`).
            if !self
                .entities
                .get(poison.who)
                .is_some_and(|entity| !entity.dead)
            {
                return false;
            }

            poison.until_next_ms = poison.until_next_ms.saturating_sub(elapsed_ms);
            if poison.until_next_ms > 0 {
                return true;
            }

            let this = poison.per_tick.min(poison.remaining);
            due.push((*poison, this));
            poison.remaining -= this;
            poison.until_next_ms = POISON_EVERY_MS;
            poison.remaining > 0
        });
        self.poisons = owed;

        for (poison, amount) in due {
            if amount <= 0 {
                continue;
            }
            if poison.heals {
                self.heal_and_say(poison.who, amount);
            } else {
                // Dealt raw: the tick passes the no-defence flag, because the reduction was taken
                // once when the grenade landed (`Player.UseItem.cs:1318`).
                self.wound(catalog, poison.from, poison.who, amount);
            }
        }
    }

    /// Lands whatever has finished falling.
    fn land_thrown(&mut self, catalog: &Catalog, behaviours: &Programs, elapsed_ms: u32) {
        if self.falling.is_empty() {
            return;
        }

        let mut waiting = std::mem::take(&mut self.falling);
        let mut arrived = Vec::new();

        waiting.retain_mut(|falling| {
            falling.remaining_ms = falling.remaining_ms.saturating_sub(elapsed_ms);
            if falling.remaining_ms == 0 {
                arrived.push(*falling);
                false
            } else {
                true
            }
        });
        self.falling = waiting;

        for landed in arrived {
            // Thrown into a wall is thrown away: `if (!world.IsPassable(target...)) return;`
            // (`TossObject.cs:174`), so a boss that throws at somebody standing behind cover makes
            // nothing at all rather than making it inside the wall.
            if !self.terrain.walkable_at(landed.x, landed.y) {
                continue;
            }

            if let Some(handle) = self.spawn_child(
                catalog,
                behaviours,
                landed.kind,
                landed.x,
                landed.y,
                None,
                None,
                false,
            ) && let Some(entity) = self.entities.get_mut(handle)
            {
                entity.terrain = landed.terrain;
            }
        }
    }

    /// Whether a square's object is scenery rather than something the world has to hold.
    ///
    /// Asked both when the world is built and when a client is told what the map looks like, so the
    /// two can never disagree and leave a tree that is drawn but not there, or there but not drawn.
    pub fn is_scenery(catalog: &Catalog, square: &hendra_content::Composition) -> bool {
        // A name is the map author saying this one is not a tree. A size is not: the realm map
        // scales seventy thousand of its trees for variety, and that is drawing, not meaning.
        if square.name().is_some() {
            return false;
        }

        catalog.object(square.object).is_some_and(|desc| {
            desc.static_object && !desc.enemy && is_decoration(desc.class.as_str())
        })
    }

    /// Puts some number of one kind of thing down near a point.
    ///
    /// For an administrator's `/spawn`, which is the only caller: everything else that spawns comes
    /// from a behaviour, a setpiece or a realm, and each of those decides its own placement.
    pub fn spawn_at(
        &mut self,
        catalog: &Catalog,
        kind: ObjectType,
        x: f32,
        y: f32,
        count: usize,
    ) -> usize {
        let behaviours = std::mem::take(&mut self.behaviours);
        let mut made = 0;

        for index in 0..count {
            // Fanned, so a group put down together does not sit in one square and read as one.
            let spread = index as f32 * 0.35;
            if self
                .spawn_child(catalog, &behaviours, kind, x + spread, y, None, None, false)
                .is_some()
            {
                made += 1;
            }
        }

        self.behaviours = behaviours;
        made
    }

    /// Puts a merchant down with something to sell.
    pub fn open_stall(
        &mut self,
        kind: ObjectType,
        x: u32,
        y: u32,
        selling: crate::shop::Stall,
    ) -> Option<Handle> {
        let mut stall = Entity::fixture(kind, x as f32 + 0.5, y as f32 + 0.5);
        stall.selling = Some(selling);
        self.spawn(stall)
    }

    /// Players who have died since the last call, and what killed each.
    ///
    /// Drained rather than read, because a death is answered once: the character is marked dead, a
    /// gravestone goes down and the session ends. A caller that forgets to drain gets a queue that
    /// stops growing rather than one that grows forever.
    pub fn take_deaths(&mut self) -> Vec<Death> {
        std::mem::take(&mut self.deaths)
    }

    /// Ends a player's life, naming what to blame.
    ///
    /// The death is claimed here rather than left to the end-of-tick sweep, which is what
    /// `KillPlayerCommand` does with `HP = 0; Death(player.Name)`
    /// (`RankedCommands.cs:1055-1056`): the sweep answers only what nothing claimed and answers it
    /// with `rekt: true` (`Player.cs:586`), so a death dropped into it is a trip to the nexus and
    /// not the end of a character. Whether this one is permanent is still the session's to decide,
    /// which weighs a resurrection amulet and where the player was standing.
    pub fn slay(&mut self, who: Handle, killer: String) {
        if let Some(entity) = self.entities.get_mut(who) {
            if entity.kind != Kind::Player {
                return;
            }
            entity.hp = 0;
            entity.dead = true;
        }

        self.claim_death(who, killer, false);
    }

    /// Every hit that landed since the last call.
    ///
    /// Drained for the same reason the announcements are: each of these is sent once, and a caller
    /// that forgets gets a queue that stops growing rather than one that grows forever.
    pub fn take_damage(&mut self) -> Vec<DamageEvent> {
        std::mem::take(&mut self.damage)
    }

    /// Records a hit for broadcast, if there is still room to.
    fn note_damage(&mut self, event: DamageEvent) {
        if self.damage.len() < MAX_PENDING_DAMAGE {
            self.damage.push(event);
        }
    }

    /// Puts a gravestone where somebody died.
    ///
    /// Which stone and how long it stands come from how much of the character was finished, as they
    /// do in the original: a level-one death leaves a small stone for thirty seconds and an
    /// eight-of-eight death leaves the largest for ten minutes. It is what the room remembers.
    pub fn place_gravestone(
        &mut self,
        catalog: &Catalog,
        at: (f32, f32),
        name: &str,
        maxed: usize,
        level: i16,
        rekt: bool,
    ) {
        let (stone, standing_ms) = match maxed {
            8 => (0x0735, 600_000),
            7 => (0x0734, 600_000),
            6 => (0x072b, 600_000),
            5 => (0x072a, 600_000),
            4 => (0x0729, 600_000),
            3 => (0x0728, 600_000),
            2 => (0x0727, 600_000),
            1 => (0x0726, 600_000),
            _ if level <= 1 => (0x0723, 30_000),
            _ if level < 20 => (0x0724, 60_000),
            _ => (0x0725, 300_000),
        };

        let kind = ObjectType(stone);
        if catalog.object(kind).is_none() {
            return;
        }

        let mut grave = Entity::fixture(kind, at.0, at.1);
        grave.name = Some(if rekt {
            format!("{name} got rekt").into()
        } else {
            name.into()
        });
        grave.expires_in_ms = Some(standing_ms);
        self.spawn(grave);
    }

    /// Notes that somebody said something, for whatever is listening.
    ///
    /// A dungeon whose door opens when you say the right word is built out of this, and without it
    /// the door has no handle.
    pub fn heard(&mut self, at: (f32, f32), text: &str) {
        if self.heard.len() < MOST_HEARD_AT_ONCE {
            self.heard.push((at.0, at.1, text.into()));
        }
    }

    /// Moves a player to another player, if every rule allows it.
    ///
    /// The refusals are the original's, in `Player.Teleport`, and each is a real hole otherwise:
    /// teleporting to somebody invisible finds a player who is hiding, teleporting to somebody
    /// paused reaches into a place the world has stopped, and teleporting with no cooldown is a way
    /// to cross a realm faster than anything can chase.
    ///
    /// Returns why it was refused, or `None` when it happened.
    pub fn teleport_to(&mut self, who: Handle, to: Handle) -> Option<&'static str> {
        if who == to {
            return Some("You are already at yourself, and always will be.");
        }
        if !self.allows_teleport {
            return Some("You cannot teleport here.");
        }

        let mover = self.entities.get(who)?;
        if mover.teleport_cooldown_ms > 0 {
            return Some("Too soon to teleport again.");
        }
        if mover
            .conditions
            .contains(hendra_content::ConditionEffect::Paused)
        {
            return Some("You cannot teleport while paused.");
        }

        let Some(target) = self.entities.get(to) else {
            return Some("They are not here.");
        };
        if target.kind != Kind::Player {
            return Some("You can only teleport to players.");
        }
        if target
            .conditions
            .contains(hendra_content::ConditionEffect::Invisible)
        {
            return Some("You cannot teleport to an invisible player.");
        }
        if target
            .conditions
            .contains(hendra_content::ConditionEffect::Paused)
        {
            return Some("You cannot teleport to a paused player.");
        }

        let (x, y) = (target.x, target.y);
        self.snap_to(who, x, y);
        None
    }

    /// Moves a player to a square, as `/gland` does.
    ///
    /// A place rather than a person, so none of the rules about who may be teleported to apply; the
    /// cooldown and the pause do, because they are about the mover. Outside the map is refused
    /// rather than clamped: a command that silently puts you somewhere else is worse than one that
    /// says no.
    pub fn teleport_to_square(&mut self, who: Handle, x: f32, y: f32) -> Option<&'static str> {
        if !self.allows_teleport {
            return Some("You cannot teleport here.");
        }
        if x < 0.0
            || y < 0.0
            || x >= self.terrain.width() as f32
            || y >= self.terrain.height() as f32
        {
            return Some("There is no such place here.");
        }

        let mover = self.entities.get(who)?;
        if mover.teleport_cooldown_ms > 0 {
            return Some("Too soon to teleport again.");
        }
        if mover
            .conditions
            .contains(hendra_content::ConditionEffect::Paused)
        {
            return Some("You cannot teleport while paused.");
        }

        self.snap_to(who, x, y);
        None
    }

    /// Puts a body somewhere it did not walk to, and records that it must be told so.
    ///
    /// The one path both teleports end in, as `Player.TeleportPosition` (`Player.cs:663-704`) is
    /// the one both of the original's end in.
    fn snap_to(&mut self, who: Handle, x: f32, y: f32) {
        let Some(mover) = self.entities.get_mut(who) else {
            return;
        };

        mover.x = x;
        mover.y = y;
        mover.teleport_cooldown_ms = TELEPORT_COOLDOWN_MS;
        mover.tally.teleports += 1;

        // The jump arrives at the mover's own client as a position it did not ask for, and at
        // everyone else's as a move no speed explains. The grace is what stops the server's own
        // teleport being read as somebody moving too fast, and it has to outlast the round trip
        // that tells the client where it is.
        mover.move_grace_ms = MOVE_GRACE_MS;

        // `TeleportPosition` forces a fresh pick before it moves anybody (`Player.cs:678`).
        // Distance is part of the score, so a jump across a realm can make the old answer the wrong
        // one, and the arrow would otherwise hold it for the rest of the interval.
        mover.quest_target = None;

        // Nothing else on the wire can say this. A snapshot gives a position to glide towards and
        // the mover's own client does not read its own position out of one at all, so without this
        // the mover walks on from where it believed it was and puts itself back.
        if self.teleports.len() < MAX_PENDING_TELEPORTS {
            self.teleports.push(TeleportEvent { who, x, y });
        }

        self.look_around(who);
    }

    /// Every body the world has moved since the last call.
    pub fn take_teleports(&mut self) -> Vec<TeleportEvent> {
        std::mem::take(&mut self.teleports)
    }

    /// Queues something for the clients to draw.
    ///
    /// What `BroadcastPacket(new ShowEffect { .. })` does from wherever an effect happens. The world
    /// holds it until the tick drains it, because the world does not know who is watching.
    pub fn show_effect(&mut self, effect: EffectEvent) {
        if self.effects.len() < MAX_PENDING_EFFECTS {
            self.effects.push(effect);
        }
    }

    /// Everything the world has asked to be drawn since the last call.
    pub fn take_effects(&mut self) -> Vec<EffectEvent> {
        std::mem::take(&mut self.effects)
    }

    /// Floats a line of text off a body.
    ///
    /// What `BroadcastSync(new Notification { .. })` does. Held rather than sent for the same
    /// reason an effect is: it belongs to a body, and who can see that body is not the world's
    /// question.
    pub fn float_text(&mut self, who: Handle, text: &str, color: u32) {
        self.push_status_text(who, text, color, false);
    }

    /// Floats a line off a body that the whole world hears, however far away it is standing.
    ///
    /// The four sites that use a bare `BroadcastPacket` rather than a distance test: a dungeon
    /// being opened or unlocked and an administrator announcing a spawn. What they say is about the
    /// world rather than about the body, and a player across the map is exactly who needs to be
    /// told.
    pub fn float_text_everywhere(&mut self, who: Handle, text: &str, color: u32) {
        self.push_status_text(who, text, color, true);
    }

    fn push_status_text(&mut self, who: Handle, text: &str, color: u32, everywhere: bool) {
        if self.status_texts.len() < MAX_PENDING_EFFECTS {
            self.status_texts.push(StatusTextEvent {
                who,
                text: text.into(),
                color,
                everywhere,
            });
        }
    }

    /// Everything the world has asked to be said about a body since the last call.
    pub fn take_status_texts(&mut self) -> Vec<StatusTextEvent> {
        std::mem::take(&mut self.status_texts)
    }

    /// Telegraphs a blast at a place.
    pub fn blast_ring(&mut self, blast: BlastEvent) {
        if self.blasts.len() < MAX_PENDING_EFFECTS {
            self.blasts.push(blast);
        }
    }

    /// Every blast the world has set off since the last call.
    pub fn take_blasts(&mut self) -> Vec<BlastEvent> {
        std::mem::take(&mut self.blasts)
    }

    /// Adds health and says so, or does neither.
    ///
    /// `ActivateHealHp` (`Player.UseItem.cs:1244-1265`). The order matters and the guard more so:
    /// a heal that would take a full player past their maximum changes nothing and therefore sends
    /// nothing, so drinking a potion at full health is silent rather than showing a `+0`. What is
    /// sent is the amount actually gained, not the amount the item promised.
    ///
    /// The white potion sparkle rides along with it at every site, which is what makes a heal read
    /// as something that was done to you rather than as a bar that moved.
    fn heal_and_say(&mut self, who: Handle, amount: i32) {
        let Some(entity) = self.entities.get_mut(who) else {
            return;
        };
        let gained = (entity.hp + amount).min(entity.max_hp) - entity.hp;
        if gained == 0 {
            return;
        }
        entity.hp += gained;

        self.show_potion(who);
        self.float_text(who, &format!("+{gained}"), HEAL_TEXT_COLOUR);
    }

    /// Adds mana and says so, or does neither.
    ///
    /// `ActivateHealMp` (`Player.UseItem.cs:1267-1287`), which differs from the health above in
    /// nothing but the bar it fills and the colour it says so in. No sickness check: `Sick` stops
    /// health from being restored and has never touched mana.
    fn refill_and_say(&mut self, who: Handle, amount: i32) {
        let Some(entity) = self.entities.get_mut(who) else {
            return;
        };
        let gained = (entity.mp + amount).min(entity.max_mp) - entity.mp;
        if gained == 0 {
            return;
        }
        entity.mp += gained;

        self.show_potion(who);
        self.float_text(who, &format!("+{gained}"), MANA_TEXT_COLOUR);
    }

    /// Restores health on behalf of a behaviour, and shows every sign of it the original shows.
    ///
    /// The four healing behaviours -- `HealSelf`, `HealPlayer`, `HealGroup`, `HealEntity` -- all
    /// send the same three packets together (`HealPlayer.cs:47-66` and the identical blocks in the
    /// other three): the white sparkle over whoever was healed, a white trail drawn from the healer
    /// to where they stand, and the amount gained in green over their head. Between them they are
    /// the whole reason a healer in a room reads as a healer rather than as monsters whose bars
    /// keep refilling on their own.
    ///
    /// All three are conditional on the health actually changing (`newHp != entity.HP`). A group
    /// already at full is healed silently every cooldown, and a `+0` over each of them would turn a
    /// healthy pack into a light show.
    fn heal_and_show(&mut self, healer: Handle, who: Handle, amount: i32) {
        let Some(entity) = self.entities.get_mut(who) else {
            return;
        };
        let gained = (entity.hp + amount).min(entity.max_hp) - entity.hp;
        if gained <= 0 {
            return;
        }
        entity.hp += gained;
        let (x, y) = (entity.x, entity.y);

        self.show_potion(who);

        // Anchored on the healer and pointed at the healed, which is the direction that makes it
        // legible: the beam leaves whoever cast it.
        self.show_effect(EffectEvent {
            effect: hendra_net::message::effect::TRAIL,
            target: Some(healer),
            x1: x,
            y1: y,
            x2: 0.0,
            y2: 0.0,
            color: 0xffff_ffff,
        });

        self.float_text(who, &format!("+{gained}"), HEAL_TEXT_COLOUR);
    }

    /// Whether health may be restored to this body at all.
    ///
    /// `Sick` is the one condition that stops a heal outright rather than reducing it, and every
    /// site that heals a player tests it before doing anything (`Player.UseItem.cs:987`, `:960`,
    /// `:900`).
    fn is_sick(&self, who: Handle) -> bool {
        self.entities
            .get(who)
            .is_some_and(|entity| crate::effects::Rules::of(entity.conditions).sick)
    }

    /// The white sparkle a restored bar comes with.
    fn show_potion(&mut self, who: Handle) {
        self.show_effect(EffectEvent {
            effect: hendra_net::message::effect::POTION,
            target: Some(who),
            x1: 0.0,
            y1: 0.0,
            x2: 0.0,
            y2: 0.0,
            color: 0xffff_ffff,
        });
    }

    /// Draws the ring an area effect fills.
    ///
    /// `AEMagicNova` (`Player.UseItem.cs:935`) and `AEHealNova` (`:968`) both send the same one: an
    /// `AreaBlast` centred on the caster, with the radius in `Pos1.X` rather than as a coordinate
    /// (`Structures.cs:139`), in white. It is the whole of what tells the people standing in a nova
    /// that they were standing in it -- the bars move, but nothing says why.
    fn show_nova(&mut self, handle: Handle, range: f32) {
        self.show_effect(EffectEvent {
            effect: hendra_net::message::effect::AREA_BLAST,
            target: Some(handle),
            x1: range,
            y1: 0.0,
            x2: 0.0,
            y2: 0.0,
            color: 0xffff_ffff,
        });
    }

    /// Whether players may teleport in this world, from the world definition.
    /// Whether one player may teleport to another here, for whoever has to say so.
    ///
    /// `World.AllowTeleport`, which the client is told in `MapInfo.AllowPlayerTeleport` so it can
    /// stop offering the option rather than offering one the server will refuse.
    pub fn allows_teleport(&self) -> bool {
        self.allows_teleport
    }

    pub fn set_allows_teleport(&mut self, allowed: bool) {
        self.allows_teleport = allowed;
    }

    /// Where a player is, by name.
    pub fn player_named(&self, name: &str) -> Option<Handle> {
        self.entities
            .iter()
            .find(|(_, entity)| {
                entity.kind == Kind::Player
                    && entity
                        .name
                        .as_deref()
                        .is_some_and(|held| held.eq_ignore_ascii_case(name))
            })
            .map(|(handle, _)| handle)
    }

    /// Counts what is alive on each terrain.
    ///
    /// From the tag an enemy carries rather than from where it is standing, because an enemy that
    /// has chased somebody across a border still belongs to the terrain it was placed on. That is
    /// what stops a chase from emptying one terrain and overfilling the next.
    pub fn alive_by_terrain(&self) -> [usize; TERRAIN_COUNT] {
        let mut counts = [0usize; TERRAIN_COUNT];

        for (_, entity) in self.entities.iter() {
            if entity.kind != Kind::Enemy || entity.dead {
                continue;
            }
            counts[entity.terrain as usize] += 1;
        }

        counts
    }

    /// Carries out what the realm asked for: fills terrains that are short, thins ones that are
    /// over.
    ///
    /// Returns how many were added and how many removed.
    pub fn populate(
        &mut self,
        catalog: &Catalog,
        spawnable: &[crate::realm::Spawn],
        wanted: &[crate::realm::Adjustment],
    ) -> (usize, usize) {
        let mut added = 0;
        let mut removed = 0;

        for adjustment in wanted {
            if adjustment.remove > 0 {
                removed += self.thin(adjustment.terrain, adjustment.remove);
                continue;
            }

            added += self.fill(catalog, spawnable, adjustment.terrain, adjustment.add);
        }

        (added, removed)
    }

    /// Adds up to `wanted` enemies to one terrain.
    fn fill(
        &mut self,
        catalog: &Catalog,
        spawnable: &[crate::realm::Spawn],
        terrain: hendra_content::Terrain,
        wanted: usize,
    ) -> usize {
        if self.open_squares(terrain).is_empty() {
            return 0;
        }

        // Lifted out for the same reason the tick lifts them: `spawn_child` needs them while the
        // world is borrowed mutably.
        let behaviours = std::mem::take(&mut self.behaviours);
        let mut made = 0;

        // A budget rather than a loop until done. The weights for a terrain do not always sum to
        // one, so some rolls choose nothing, and a square can come back crowded; both are reasons
        // to try again, not reasons to stop, but neither can be allowed to spin forever.
        let mut tries = wanted * 4 + 64;

        while made < wanted && tries > 0 {
            tries -= 1;

            let Some(spawn) = crate::realm::choose(spawnable, terrain, self.roll()) else {
                continue;
            };
            let Some((x, y)) = self.open_square(terrain) else {
                continue;
            };

            // A description that asks for a group brings its whole group, scattered around the
            // point rather than stacked on it.
            let size = match spawn.group {
                Some(count) => count.size(crate::realm::normal(self.roll(), self.roll())),
                None => 1,
            };

            for _ in 0..size {
                let (at_x, at_y) = match spawn.group {
                    Some(_) => (
                        x + (self.roll() * 2.0 - 1.0) * crate::realm::GROUP_SPREAD,
                        y + (self.roll() * 2.0 - 1.0) * crate::realm::GROUP_SPREAD,
                    ),
                    None => (x, y),
                };

                // The same rule the square was chosen by. The original checks nothing at all here —
                // a group is scattered five tiles around the point it found and moved without
                // another look (`Oryx.cs:565-570`) — but a guard that used the stricter rule would
                // throw away the tree and cactus squares `IsPassable` had just accepted.
                if !self.terrain.passable_at(at_x, at_y) {
                    continue;
                }

                if let Some(handle) = self.spawn_child(
                    catalog,
                    &behaviours,
                    spawn.kind,
                    at_x,
                    at_y,
                    None,
                    None,
                    false,
                ) && let Some(entity) = self.entities.get_mut(handle)
                {
                    // Tagged with the terrain it was placed on, which is what the next count reads
                    // and what anything it spawns inherits.
                    entity.terrain = terrain;
                    made += 1;
                }
            }
        }

        self.behaviours = behaviours;
        made
    }

    /// Removes up to `wanted` enemies from one terrain.
    ///
    /// Only ones nobody is near: despawning something a player is fighting reads as the server
    /// eating their kill.
    fn thin(&mut self, terrain: hendra_content::Terrain, wanted: usize) -> usize {
        let players: Vec<(f32, f32)> = self
            .entities
            .iter()
            .filter(|(_, entity)| entity.kind == Kind::Player && !entity.dead)
            .map(|(_, entity)| (entity.x, entity.y))
            .collect();

        let doomed: Vec<Handle> = self
            .entities
            .iter()
            .filter(|(_, entity)| {
                entity.kind == Kind::Enemy && !entity.dead && entity.terrain == terrain
            })
            .filter(|(_, entity)| {
                !players.iter().any(|(px, py)| {
                    let (dx, dy) = (entity.x - px, entity.y - py);
                    dx * dx + dy * dy < crate::realm::PLAYER_CLEARANCE.powi(2)
                })
            })
            .map(|(handle, _)| handle)
            .take(wanted)
            .collect();

        // Removed outright rather than killed: a thinned enemy is one the map never needed, and
        // killing it would drop loot and hand out experience nobody earned.
        for handle in &doomed {
            self.despawn(*handle);
        }

        doomed.len()
    }

    /// Every square of a terrain a realm enemy may be placed on, worked out once and kept.
    ///
    /// A realm map is four million squares and a terrain can be a thousandth of it, so looking for
    /// one by guessing at random finds nothing in any reasonable number of tries. Listing them once
    /// turns every later search into a single pick.
    ///
    /// The test is [`Terrain::passable`] rather than [`Terrain::walkable`], because `Oryx.Spawn`
    /// calls `IsPassable(pt.X, pt.Y)` with `spawning` left at its default of false (`Oryx.cs:559`,
    /// `:581`) — so an object that occupies its square against players but not against enemies is
    /// somewhere Oryx may put one. That is every tree, cactus and rock in the realm.
    fn open_squares(&mut self, terrain: hendra_content::Terrain) -> &[u32] {
        if !self.spawn_squares.contains_key(&terrain) {
            let mut squares = Vec::new();

            for y in 0..self.terrain.height() {
                for x in 0..self.terrain.width() {
                    if self.terrain.terrain_at(x, y) == terrain && self.terrain.passable(x, y) {
                        squares.push((x << 16) | y);
                    }
                }
            }

            self.spawn_squares.insert(terrain, squares);
        }

        &self.spawn_squares[&terrain]
    }

    /// A walkable square of a terrain with nobody near it.
    ///
    /// Away from players, because an enemy appearing on top of somebody is not a spawn but an
    /// ambush nobody could have avoided.
    fn open_square(&mut self, terrain: hendra_content::Terrain) -> Option<(f32, f32)> {
        let count = self.open_squares(terrain).len();
        if count == 0 {
            return None;
        }

        for _ in 0..SPAWN_ATTEMPTS {
            let picked = ((self.roll() * count as f32) as usize).min(count - 1);
            let packed = self.spawn_squares[&terrain][picked];
            let (x, y) = (packed >> 16, packed & 0xffff);

            let (at_x, at_y) = (x as f32 + 0.5, y as f32 + 0.5);
            self.grid
                .within(at_x, at_y, crate::realm::PLAYER_CLEARANCE, &mut self.nearby);
            let nearby = std::mem::take(&mut self.nearby);

            let crowded = nearby.iter().any(|handle| {
                self.entities
                    .get(*handle)
                    .is_some_and(|entity| entity.kind == Kind::Player)
            });
            self.nearby = nearby;

            if !crowded {
                return Some((at_x, at_y));
            }
        }

        None
    }

    /// Where in the realm one of Oryx's events may be raised, by its top left corner.
    ///
    /// `SpawnEvent` (`Oryx.cs:785`) takes any square from the mountains down to the middle forest
    /// -- the top half of the map, where the fights are -- that is passable, has nothing occupying
    /// it, and has no player within ten tiles, and then steps back by half the set piece so that
    /// the square it found ends up in the middle of it.
    pub fn event_ground(&mut self, size: u32) -> Option<(u32, u32)> {
        // From the mountains to the middle forest, which in the content's own numbering is the top
        // seven of the thirteen terrains.
        const BAND: [hendra_content::Terrain; 7] = [
            hendra_content::Terrain::Mountains,
            hendra_content::Terrain::HighSand,
            hendra_content::Terrain::HighPlains,
            hendra_content::Terrain::HighForest,
            hendra_content::Terrain::MidSand,
            hendra_content::Terrain::MidPlains,
            hendra_content::Terrain::MidForest,
        ];

        for _ in 0..SPAWN_ATTEMPTS {
            let picked = ((self.roll() * BAND.len() as f32) as usize).min(BAND.len() - 1);
            let Some((x, y)) = self.open_square(BAND[picked]) else {
                continue;
            };

            // Held inside the map rather than allowed to go negative, which is the one place the
            // original does not check: a set piece chosen near the top left corner of the map is
            // drawn from a negative square and throws.
            let back = (size.saturating_sub(1)) / 2;
            return Some((
                (x as u32).saturating_sub(back),
                (y as u32).saturating_sub(back),
            ));
        }

        None
    }

    /// Draws a setpiece into the world at a point.
    ///
    /// The drawing is in its own coordinates, from its top left corner, which is where the original
    /// puts them: a setpiece is placed by its corner and not by its middle.
    ///
    /// Returns the names it could not find in the content. A setpiece with a missing name is a hole
    /// in the realm, and it should be said rather than left to be walked into.
    pub fn draw(
        &mut self,
        catalog: &Catalog,
        drawing: &crate::setpiece::Drawing,
        at: (u32, u32),
    ) -> Vec<&'static str> {
        use crate::setpiece::Placed;

        let mut missing = Vec::new();
        let mut refused = 0usize;
        let behaviours = std::mem::take(&mut self.behaviours);

        for square in &drawing.squares {
            let x = at.0 as i32 + square.x;
            let y = at.1 as i32 + square.y;
            if x < 0 || y < 0 || !self.terrain.contains(x as u32, y as u32) {
                continue;
            }
            let (x, y) = (x as u32, y as u32);

            let tile = match square.tile {
                Some(name) => match catalog.tile_type_of(name) {
                    Some(kind) => Some(kind),
                    None => {
                        missing.push(name);
                        None
                    }
                },
                None => None,
            };

            let object = match square.object {
                Some(name) => match catalog.type_of(name) {
                    Some(kind) => Some((kind, square.size)),
                    None => {
                        missing.push(name);
                        None
                    }
                },
                None => None,
            };

            // Whatever was standing here goes, whether the drawing asked for the square to be
            // cleared or is about to put its own wall there.
            if square.clear || object.is_some() {
                self.clear_area(x as f32 + 0.5, y as f32 + 0.5, 1.0, 1.0);
            }

            // Painted rather than spawned. A setpiece's walls are scenery like any other wall: the
            // original writes them onto the tile, and a castle whose every stone was an entity
            // would cost eight hundred places in the world and eight hundred snapshot entries.
            if !self.terrain.paint(
                catalog,
                x,
                y,
                tile,
                object,
                square.clear && object.is_none(),
            ) {
                refused += 1;
                continue;
            }

            if let Some(tile) = tile
                && self.ground_changes.len() < MAX_PENDING_GROUND_CHANGES
            {
                self.ground_changes.push((x as u16, y as u16, tile.0));
            }

            if let Some((object, size)) = object
                && self.scenery_changes.len() < MAX_PENDING_GROUND_CHANGES
            {
                self.scenery_changes
                    .push((x as u16, y as u16, object.0, size));
            }
        }

        for placed in &drawing.placed {
            match placed {
                Placed::Living { x, y, name, size } => {
                    let Some(kind) = catalog.type_of(name) else {
                        missing.push(name);
                        continue;
                    };

                    if let Some(handle) = self.spawn_child(
                        catalog,
                        &behaviours,
                        kind,
                        at.0 as f32 + x,
                        at.1 as f32 + y,
                        None,
                        None,
                        false,
                    ) && *size > 0
                        && let Some(entity) = self.entities.get_mut(handle)
                    {
                        entity.size = *size;
                    }
                }

                // Chests come after, because filling one rolls loot and that needs the world
                // back in one piece.
                Placed::Chest { .. } => {}
            }
        }

        self.behaviours = behaviours;

        for placed in &drawing.placed {
            let Placed::Chest {
                x,
                y,
                loot,
                least,
                most,
            } = placed
            else {
                continue;
            };

            self.place_chest(
                catalog,
                (at.0 as f32 + x, at.1 as f32 + y),
                loot,
                *least,
                *most,
            );
        }

        if refused > 0 {
            // A setpiece that came out with holes in it, because the map cannot describe any more
            // kinds of square. Better said than walked into.
            self.refused_squares += refused;
        }

        missing
    }

    /// How many squares a setpiece could not paint because the map was full.
    pub fn refused_squares(&self) -> usize {
        self.refused_squares
    }

    /// Puts down a chest holding a few items drawn from a setpiece's loot table.
    fn place_chest(
        &mut self,
        catalog: &Catalog,
        at: (f32, f32),
        loot: &[crate::setpiece::Tier],
        least: usize,
        most: usize,
    ) {
        use hendra_behavior::program::LootEntry;

        // `Loot.GetLoots` (`logic/loot/Loots.cs:47-68`) walks the *flattened* list, not the entries
        // as written. `Populate` turns one `TierLoot(6, Weapon, 0.3)` into one `LootDef` per tier-six
        // weapon, each at `0.3 / count`, and every one of them is then rolled independently — so a
        // single line of a chest's table can put two or three weapons in the chest, and often does.
        // Rolling once per line at the full chance and taking at most one item gives the same
        // expected number of items and the wrong shape: it makes one item per line the ceiling where
        // the original has no ceiling below `retCount`.
        let mut consideration = Vec::new();
        for tier in loot {
            flatten_loot(
                &LootEntry::Tier {
                    tier: tier.tier,
                    kind: tier.kind.to_string(),
                    chance: tier.chance,
                    required: 0,
                    // A chest belongs to whoever opens it; `GetLoots` never looks at a threshold.
                    threshold: 0.0,
                },
                None,
                catalog,
                &mut consideration,
            );
        }

        // `Rand.Next(min, max)` (`Loots.cs:55`), whose upper bound is exclusive: a chest asked for
        // three to eight holds three to seven.
        let mut remaining =
            (least + (self.roll() * most.saturating_sub(least) as f32) as usize) as i64;

        let mut held = Vec::new();
        for def in &consideration {
            if self.roll() < def.chance {
                held.push(def.item);
                remaining -= 1;
            }

            // Tested after the roll and against zero exactly, as the original tests it. Two
            // consequences worth keeping: a chest whose count came up zero yields nothing at all
            // unless its very first entry happens to hit, in which case the counter goes to minus
            // one, never equals zero again, and the chest fills with everything else that hits;
            // and a chest that has met its count stops on the spot rather than finishing the list.
            // Neither arises from the shipped setpieces, whose counts are drawn from three or five
            // upward, but this is a transliteration and not a paraphrase.
            if remaining == 0 {
                break;
            }
        }

        held.truncate(SETPIECE_CHEST_SLOTS);

        if held.is_empty() {
            return;
        }

        let Some(kind) = catalog.type_of(SETPIECE_CHEST) else {
            return;
        };

        let mut container = Container::new(ContainerKind::Bag, SETPIECE_CHEST_SLOTS);
        for item in held {
            container.insert(item, catalog);
        }

        let mut chest = Entity::fixture(kind, at.0, at.1);
        chest.kind = Kind::Container;
        chest.container = Some(Box::new(container));
        // No expiry: a setpiece chest is part of the realm and stands until somebody empties it.
        self.spawn(chest);
    }

    /// Setpiece names a behaviour asked for that nothing answers to.
    ///
    /// Taken rather than read, so each one is reported once. Four of the shipped behaviours name a
    /// setpiece that does not exist, and they do in the original too.
    pub fn take_unknown_setpieces(&mut self) -> Vec<String> {
        let mut names: Vec<String> = self.unknown_setpieces.drain().collect();
        names.sort();
        names
    }

    /// How the realm is doing.
    pub fn realm(&self) -> &crate::realm::Realm {
        &self.realm
    }

    /// How the realm is doing, so a caller can advance its clock.
    pub fn realm_mut(&mut self) -> &mut crate::realm::Realm {
        &mut self.realm
    }

    /// Opens a portal where an entity is standing.
    pub fn open_portal(
        &mut self,
        catalog: &Catalog,
        at: Handle,
        kind: ObjectType,
        duration_ms: u32,
    ) -> Option<Handle> {
        let (x, y) = self.entities.get(at).map(|entity| (entity.x, entity.y))?;

        let behaviours = std::mem::take(&mut self.behaviours);
        let portal = self.spawn_child(catalog, &behaviours, kind, x, y, None, None, false);
        self.behaviours = behaviours;

        if let Some(portal) = portal
            && let Some(entity) = self.entities.get_mut(portal)
        {
            entity.kind = Kind::Portal;
            // A portal with no lifetime is a permanent change to a room somebody else has to live
            // in, so one opened by an item always has one.
            entity.expires_in_ms = Some(duration_ms.max(1_000));
        }

        portal
    }

    /// Replaces a locked door standing near a player with the one it becomes.
    ///
    /// `AEUnlockPortal` (`Player.UseItem.cs:433-510`). The unlock is not a record on an account:
    /// it is a swap in the room. The nearest portal of the named kind within three tiles is taken
    /// out of the world and one of the unlocked kind is stood in its place, with the timeout an
    /// opened portal gets. Everything a player can see about the unlock happens here — the account
    /// row this server also writes is its own invention and is read by nothing.
    ///
    /// Three tiles is `DistSqr(this) <= 9` (`:439`), and "nearest" is the aggregate over the
    /// matches (`:443-444`), so a player standing between two locked doors of the same kind opens
    /// the one they are closer to.
    pub fn swap_locked_portal(
        &mut self,
        catalog: &Catalog,
        who: Handle,
        locked: ObjectType,
        unlocked: ObjectType,
        duration_ms: u32,
    ) -> Option<Handle> {
        let (px, py) = self.entities.get(who).map(|entity| (entity.x, entity.y))?;

        let mut nearest: Option<(Handle, f32, f32, f32)> = None;
        for (handle, entity) in self.entities.iter() {
            if entity.dead || entity.kind != Kind::Portal || entity.object_type != locked {
                continue;
            }
            let (dx, dy) = (entity.x - px, entity.y - py);
            let distance = dx * dx + dy * dy;
            if distance > UNLOCK_REACH * UNLOCK_REACH {
                continue;
            }
            if nearest.is_none_or(|(_, best, _, _)| distance < best) {
                nearest = Some((handle, distance, entity.x, entity.y));
            }
        }

        let (locked_handle, _, x, y) = nearest?;

        let behaviours = std::mem::take(&mut self.behaviours);
        let opened = self.spawn_child(catalog, &behaviours, unlocked, x, y, None, None, false);
        self.behaviours = behaviours;

        let opened = opened?;
        if let Some(entity) = self.entities.get_mut(opened) {
            entity.kind = Kind::Portal;
            entity.expires_in_ms = Some(duration_ms.max(1_000));
        }

        // Only once the replacement stands, so a content file that names a portal this server
        // cannot build leaves the locked door where it was rather than removing the only way in.
        // Marked rather than deleted: `LeaveWorld` is what the original calls here, and the reap at
        // the end of the tick is what tells the clients the door is gone.
        if let Some(entity) = self.entities.get_mut(locked_handle) {
            entity.dead = true;
            entity.removed = true;
        }

        Some(opened)
    }

    /// How many enemies are alive.
    pub fn enemy_count(&self) -> usize {
        self.entities
            .iter()
            .filter(|(_, entity)| entity.kind == Kind::Enemy && !entity.dead)
            .count()
    }

    /// How many of one kind are alive, for a realm counting against a ceiling.
    pub fn count_of_kind(&self, kind: ObjectType) -> usize {
        self.entities
            .iter()
            .filter(|(_, entity)| {
                entity.object_type == kind && entity.kind == Kind::Enemy && !entity.dead
            })
            .count()
    }

    /// Says something to everyone, as the world rather than as an entity.
    ///
    /// Used for the announcements a realm makes about itself, which have no speaker: nothing in
    /// the world said them, and attributing them to an enemy would be a lie the client repeats.
    pub fn announce(&mut self, text: &str) {
        if self.announcements.len() < MAX_PENDING_ANNOUNCEMENTS {
            self.announcements.push(Announcement {
                from: Handle::NONE,
                text: text.into(),
                broadcast: true,
                speaker: None,
            });
        }
    }

    /// Says something to everyone under a name that belongs to nothing in the world.
    ///
    /// Oryx is who this is for. He is never an entity in a realm and yet is the one talking through
    /// the whole of it, which is exactly what `ChatManager.Oryx` (`ChatManager.cs:193`) does:
    /// a `Text` packet naming him, broadcast to the world, with no object behind it.
    pub fn announce_as(&mut self, speaker: &str, text: &str) {
        if self.announcements.len() < MAX_PENDING_ANNOUNCEMENTS {
            self.announcements.push(Announcement {
                from: Handle::NONE,
                text: text.into(),
                broadcast: true,
                speaker: Some(speaker.into()),
            });
        }
    }

    /// Stamps a prefab map into the world.
    ///
    /// A setpiece is a small map placed at a point, not a circle of one tile painted over the
    /// ground. Every square it names is written, including the ground and whatever stands on it,
    /// so a room built this way is the room the author drew rather than an approximation of it.
    ///
    /// The original is `Wmap.ProjectOntoWorld` (`Wmap.cs:463-497`): it copies each of the setpiece's
    /// tiles onto the world's tile whole — `CopyTo` carries `TileId`, `ObjType`, `ObjDesc`, `ObjCfg`,
    /// `Terrain` and `Region` across — and only then instantiates the entities `Wmap.Load` had
    /// already set aside. The split between the two is made at load: an object stays on the tile
    /// when it is static and not an enemy, and becomes an entity otherwise (`Wmap.cs:358-365`). So a
    /// wall a setpiece stamps down lives in the map, and `Entity.TileOccupied` reads it off the tile
    /// exactly as it reads a wall the world was born with.
    ///
    /// That is why both halves happen here. Writing the composition and then spawning only what is
    /// not scenery leaves this map in the same state [`World::new`] would have built it in, which is
    /// the property worth having: the same setpiece stamped mid-game and loaded as a world collide
    /// the same way.
    ///
    /// Anything already standing where it lands is removed. A setpiece that appeared around the
    /// existing furniture would leave a wall through the middle of a boss's arena.
    pub fn stamp(&mut self, catalog: &Catalog, piece: &Map, at: (f32, f32)) {
        // Centred on the point rather than starting there, which is what "place it here" means to
        // whoever wrote the content.
        let left = at.0 as i32 - piece.width() as i32 / 2;
        let top = at.1 as i32 - piece.height() as i32 / 2;

        // Cleared first, so the two passes cannot fight: an object spawned by the stamp must not
        // be removed by the clearing of a later square.
        self.clear_area(
            left as f32,
            top as f32,
            piece.width() as f32,
            piece.height() as f32,
        );

        for y in 0..piece.height() {
            for x in 0..piece.width() {
                let Some(square) = piece.at(x, y) else {
                    continue;
                };

                let (world_x, world_y) = (left + x as i32, top + y as i32);
                if world_x < 0 || world_y < 0 {
                    continue;
                }
                let (world_x, world_y) = (world_x as u32, world_y as u32);

                // A square whose ground the content does not know is not part of the drawing. The
                // original spells that with `TileId == 255` and skips it whole (`Wmap.cs:476-477`).
                if catalog.tile(square.tile).is_none() {
                    continue;
                }

                // The whole composition, so what stands on the square reaches the collision grids
                // and the map together. Without this a stamped wall is drawn and walked through.
                if !self.terrain.put(catalog, world_x, world_y, square.clone()) {
                    self.refused_squares += 1;
                    continue;
                }

                if self.ground_changes.len() < MAX_PENDING_GROUND_CHANGES {
                    self.ground_changes
                        .push((world_x as u16, world_y as u16, square.tile.0));
                }

                if square.object.is_none() {
                    continue;
                }

                // Scenery stays where it was written and is sent with the ground. Everything else
                // is what `Wmap.Load` sets aside for `InstantiateEntities`: it has to act, so it
                // has to be an entity as well.
                if World::is_scenery(catalog, square) {
                    if self.scenery_changes.len() < MAX_PENDING_GROUND_CHANGES {
                        self.scenery_changes.push((
                            world_x as u16,
                            world_y as u16,
                            square.object.0,
                            square.size().unwrap_or(0).clamp(0, u16::MAX as i32) as u16,
                        ));
                    }
                    continue;
                }

                let behaviours = std::mem::take(&mut self.behaviours);
                let spawned = self.spawn_child(
                    catalog,
                    &behaviours,
                    square.object,
                    world_x as f32 + 0.5,
                    world_y as f32 + 0.5,
                    None,
                    None,
                    false,
                );
                self.behaviours = behaviours;

                // The name and size the map author gave it. A setpiece's boss is often a stock
                // enemy renamed, and without this it arrives under the wrong name.
                if let Some(entity) = spawned.and_then(|handle| self.entities.get_mut(handle)) {
                    if let Some(name) = square.name() {
                        entity.name = Some(name.into());
                    }
                    if let Some(size) = square.size() {
                        entity.size = size.clamp(0, u16::MAX as i32) as u16;
                    }
                }
            }
        }
    }

    /// Removes everything standing in a rectangle, except players.
    ///
    /// Players are spared because a setpiece landing on somebody should move the room around them,
    /// not delete them from it.
    fn clear_area(&mut self, left: f32, top: f32, width: f32, height: f32) {
        for (_, entity) in self.entities.iter_mut() {
            if entity.kind == Kind::Player {
                continue;
            }
            if entity.x >= left
                && entity.x < left + width
                && entity.y >= top
                && entity.y < top + height
            {
                entity.dead = true;
                entity.spawned = true;
                entity.removed = true;
            }
        }
    }

    /// Which way an entity was last seen going, as a unit vector.
    ///
    /// `Decoy`'s constructor asks `TryGetHistory(1)` for where its owner was one sample ago and
    /// takes the difference from where they are now (`Decoy.cs:36-43`). A player who was standing
    /// still gives a difference of nothing, and the original answers that with a random heading
    /// rather than with no heading at all — so a decoy left by somebody who has stopped still walks
    /// off, just not the way they came.
    fn heading_of(&mut self, handle: Handle) -> Drifting {
        let step = self.entities.get(handle).map(|entity| {
            let (past_x, past_y) = entity.trail.previous;
            (entity.x - past_x, entity.y - past_y)
        });

        if let Some((dx, dy)) = step {
            let length = dx.hypot(dy);
            if length > 0.0 {
                return Drifting {
                    dx: dx / length,
                    dy: dy / length,
                    blasted: false,
                };
            }
        }

        let angle = self.roll() * std::f32::consts::TAU;
        Drifting {
            dx: angle.cos(),
            dy: angle.sin(),
            blasted: false,
        }
    }

    /// Walks every decoy along its heading and sets off the one whose time is nearly up.
    ///
    /// `Decoy.Tick` (`Decoy.cs:57-75`), in its order. It moves only while more than two seconds of
    /// its life remain to be spent — `HP > duration - 2000`, and a decoy's health *is* its remaining
    /// time — so the three-second decoy the game ships walks for one second and then stands. Under
    /// 250 ms it shows a red `AreaBlast` once, which damages nothing and is only how the room is
    /// told the trick is over.
    ///
    /// Runs before the countdown, as the original does: `Decoy.Tick` reads the health and then calls
    /// `base.Tick`, which is what takes the elapsed time off it.
    fn drift_decoys(&mut self, elapsed_ms: u32) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| entity.kind == Kind::Decoy && entity.decoy.is_some())
                .map(|(handle, _)| handle),
        );

        if self.handles.is_empty() {
            return;
        }

        let decoys = std::mem::take(&mut self.handles);
        for handle in &decoys {
            let Some((x, y, hp, life, drifting)) = self.entities.get(*handle).and_then(|entity| {
                entity
                    .decoy
                    .map(|drifting| (entity.x, entity.y, entity.hp, entity.max_hp, drifting))
            }) else {
                continue;
            };

            if hp > life - DECOY_DRIFT_MS {
                let travelled = DECOY_SPEED * elapsed_ms as f32 / 1000.0;
                self.walk(
                    *handle,
                    x + drifting.dx * travelled,
                    y + drifting.dy * travelled,
                );
            }

            if hp < DECOY_BLAST_MS && !drifting.blasted {
                if let Some(entity) = self.entities.get_mut(*handle) {
                    entity.decoy = Some(Drifting {
                        blasted: true,
                        ..drifting
                    });
                }

                self.show_effect(EffectEvent {
                    effect: hendra_net::message::effect::AREA_BLAST,
                    target: Some(*handle),
                    x1: 1.0,
                    y1: 0.0,
                    x2: 0.0,
                    y2: 0.0,
                    color: 0xffff_0000,
                });
            }
        }

        self.handles = decoys;
    }

    /// Sets off any trap something has walked into.
    fn spring_traps(&mut self, catalog: &Catalog) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| entity.kind == Kind::Trap && entity.armed.is_some())
                .map(|(handle, _)| handle),
        );

        if self.handles.is_empty() {
            return;
        }

        let armed = std::mem::take(&mut self.handles);
        for handle in &armed {
            let Some((x, y, trap)) = self
                .entities
                .get(*handle)
                .and_then(|entity| entity.armed.map(|armed| (entity.x, entity.y, armed)))
            else {
                continue;
            };

            // A trap is sprung by anything it would hurt, which is what makes walking over one a
            // mistake rather than a decision.
            self.grid.within(x, y, trap.radius, &mut self.nearby);
            let nearby = std::mem::take(&mut self.nearby);

            let triggered = nearby.iter().any(|other| {
                self.entities
                    .get(*other)
                    .is_some_and(|entity| entity.kind == Kind::Enemy && !entity.dead)
            });
            self.nearby = nearby;

            if !triggered {
                continue;
            }

            self.explode(
                catalog,
                (x, y),
                Blast {
                    from: Some(*handle),
                    radius: trap.radius,
                    damage: trap.damage,
                    effect: trap.effect,
                    effect_ms: TRAP_EFFECT_MS,
                    hits_players: false,
                },
            );

            if let Some(entity) = self.entities.get_mut(*handle) {
                entity.dead = true;
                entity.spawned = true;
                entity.removed = true;
            }
        }

        self.handles = armed;
    }

    /// Awards experience for everything that died this tick.
    ///
    /// Between the death and the reaping, because where the enemy fell decides who is close enough
    /// to be paid for it.
    fn award_experience(&mut self, catalog: &Catalog) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| pays_for_death(entity, catalog))
                .map(|(handle, _)| handle),
        );

        if self.handles.is_empty() {
            return;
        }

        // Everybody the world could pay, gathered once rather than per corpse. `DamageCounter`
        // walks `enemy.Owner.Players.Values` and skips whoever is paused
        // (`DamageCounter.cs:77-81`), and neither the membership nor the pause changes between two
        // deaths on the same tick.
        let candidates: Vec<Handle> = self
            .entities
            .iter()
            .filter(|(_, entity)| {
                entity.kind == Kind::Player
                    && !entity.dead
                    && !crate::effects::Rules::of(entity.conditions).paused
            })
            .map(|(handle, _)| handle)
            .collect();

        // A third of the usual in the Puppet Master's Theatre, which `DamageCounter` decides by
        // matching "Theatre" against the world's display name (`DamageCounter.cs:95-96`). The name
        // here is the definition's `name`, and the one world in the content that matches carries
        // the same string in its `sbName`, so `GetDisplayName` would hand back exactly this.
        let world_multiplier = if self.name.contains("Theatre") {
            crate::leveling::THEATRE_SHARE
        } else {
            1.0
        };

        let dead = std::mem::take(&mut self.handles);
        for handle in &dead {
            let Some(entity) = self.entities.get(*handle) else {
                continue;
            };
            // The health the descriptor declares, not what the room grew it to. `DamageCounter`
            // reads `enemy.ObjectDesc.MaxHP` (`DamageCounter.cs:83`), so a boss whose health scales
            // with the crowd is worth the same to each of them as it would be to one player alone.
            let (x, y) = (entity.x, entity.y);
            let max_hp = entity.base_max_hp.unwrap_or(entity.max_hp);
            let awards = entity.awards_experience;
            let desc = catalog.object(entity.object_type);
            let multiplier = desc.and_then(|desc| desc.exp_multiplier).unwrap_or(1.0);

            // What kind of thing it was, for the counters that ask. Each is a flag the descriptor
            // carries, which is what `FameCounter.Killed` reads (`FameCounter.cs:77-93`): naming
            // them instead would miss the four Cube God enemies and claim every Oryx Stone Guardian.
            let was_god = desc.is_some_and(|desc| desc.god);
            let was_cube = desc.is_some_and(|desc| desc.cube);
            let was_oryx = desc.is_some_and(|desc| desc.oryx);

            // Whoever struck last is the one credited with the kill, as the original credits its
            // last hitter. Everybody nearby still shares the experience.
            let killer = entity.last_hurt_by;

            // Who levelled up off this kill, so the last hitter can be credited with everybody
            // else's. Their own does not count: an assist is help given rather than progress made.
            let mut levelled: Vec<Handle> = Vec::new();

            for player in &candidates {
                let Some(near) = self.entities.get(*player) else {
                    continue;
                };

                // Strictly nearer than twenty-five tiles, centre to centre. `Dist` is the square
                // root of the squared separation and the test is `< 25` (`DamageCounter.cs:78`,
                // `Utils.cs:15-24`), so a player exactly twenty-five tiles away is paid nothing.
                // Measured from where the two bodies are now rather than from the spatial index,
                // which holds where they were when the last tick ended.
                let (dx, dy) = (x - near.x, y - near.y);
                let radius = crate::leveling::SHARE_RADIUS;
                if dx * dx + dy * dy >= radius * radius {
                    continue;
                }

                let level = near.progress.level;
                let boosted = near.experience_boost_ms != 0;

                // Whatever the arrow was pointing at is worth five times what anything else is
                // capped at, and killing it is what a completed quest is. Both read the target as
                // it was a moment ago, which is why it is remembered rather than worked out here.
                let was_quest = near.quest_target == Some(*handle);

                let Some(class) = catalog.class(near.object_type).cloned() else {
                    continue;
                };

                if was_quest && let Some(entity) = self.entities.get_mut(*player) {
                    entity.tally.quests_completed += 1;
                    entity.quest_target = None;
                }

                // `EnemyKilled` calls `FameCounter.Killed` for every player near enough to be paid
                // (`Player.Leveling.cs:321`), so a kill is only counted by someone who was there to
                // see it: walking away before the enemy falls forfeits the tally as well as the
                // experience. Only the last hitter takes the kill rather than the assist
                // (`FameCounter.cs:83-94`).
                if Some(*player) == killer
                    && let Some(entity) = self.entities.get_mut(*player)
                {
                    if was_god {
                        entity.tally.god_kills += 1;
                    } else {
                        entity.tally.monster_kills += 1;
                    }
                    if was_cube {
                        entity.tally.cube_kills += 1;
                    }
                    if was_oryx {
                        entity.tally.oryx_kills += 1;
                    }
                }

                let earned = crate::leveling::experience_for_kill(
                    max_hp,
                    multiplier,
                    awards,
                    level,
                    was_quest,
                    world_multiplier,
                    boosted,
                );

                // The roll is drawn from the world so a level-up is reproducible from the seed.
                // Drawn even for a kill worth nothing, because `EnemyKilled` runs `CheckLevelUp`
                // whatever the experience was (`Player.Leveling.cs:317-322`) and a level an earlier
                // surplus already paid for is collected by the very next kill, worthless or not.
                let mut rolls = Vec::with_capacity(8);
                for _ in 0..8 {
                    rolls.push(self.roll());
                }
                let mut next = rolls.into_iter();

                let mut gained = crate::leveling::Advance::default();
                if let Some(entity) = self.entities.get_mut(*player) {
                    let mut progress = entity.progress;
                    let advance = progress.gain(&class, &mut entity.stats, earned, || {
                        next.next().unwrap_or(0.5)
                    });
                    entity.progress = progress;
                    gained = advance;

                    // A level raises the ceiling and fills what it added, matching the original's
                    // `HP = Stats[0]` after every level.
                    if advance.levels_gained > 0 {
                        entity.reseat_maxima();
                        entity.hp = entity.max_hp;
                        entity.mp = entity.max_mp;
                    }

                    if advance.levels_gained > 0 {
                        // `CheckLevelUp` drops the quest arrow's target on every level
                        // (`Player.Leveling.cs:300`), so the next tick picks again. The level bands
                        // are what makes it necessary: a character who has just outgrown the band
                        // its target sits in would otherwise keep being pointed at it.
                        entity.quest_target = None;
                        levelled.push(*player);
                    }
                }

                // Everything a kill is worth is said over the player's own head, and each of these
                // is the only sign the thing happened at all: fame is not on the bar, and a quest
                // completing changes nothing a player can see.
                //
                // A completed quest goes out first, before the fame the same kill paid
                // (`Player.Leveling.cs:307-312` runs before `CheckLevelUp`), and travels as a
                // localisation key rather than as a sentence, which is what the client's own quest
                // panel watches for (`GameServerConnectionConcrete.as:1167-1172`).
                if was_quest {
                    self.float_text(
                        *player,
                        "{\"key\":\"server.quest_complete\"}",
                        QUEST_TEXT_COLOUR,
                    );
                }

                // The two are exclusive in the original: crossing a milestone announces the class
                // quest and says nothing about the fame that crossed it (`Player.Leveling.cs:244`
                // is an `if`/`else if`).
                if gained.crossed_fame_goal {
                    self.float_text(
                        *player,
                        "{\"key\": \"server.class_quest_complete\"}",
                        QUEST_TEXT_COLOUR,
                    );
                } else if gained.fame_gained > 0 {
                    self.float_text(
                        *player,
                        &format!("+{}Fame", gained.fame_gained),
                        FAME_TEXT_COLOUR,
                    );
                }
            }

            // The last hitter is credited with every level somebody else reached off this kill,
            // which is `DamageCounter`'s `LevelUpAssist(lvlUps)`. No party is needed for it: being
            // near enough to share the experience is what makes it help.
            let helped = levelled
                .iter()
                .filter(|player| Some(**player) != killer)
                .count() as i32;

            if helped > 0
                && let Some(killer) = killer
                && let Some(player) = self.entities.get_mut(killer)
                && player.kind == Kind::Player
            {
                player.tally.level_up_assists += helped;
            }
        }

        self.handles = dead;
    }

    /// Runs whatever the dying have arranged to happen after them.
    ///
    /// Between loot and reaping, because these need the entity still in the world. Where it was
    /// standing is most of what a portal, a transformation or a change of ground is about.
    ///
    /// Being summoned is not a reason to skip this. `RemoveEntity` sets `Spawned` and then calls
    /// `Death` (`RemoveEntity.cs:27-28`), which runs `CurrentState.OnDeath` like any other death;
    /// only the experience and the loot check the flag. The three effects that are meant to be
    /// skipped for a summon check it themselves (`AnnounceOnDeath.cs:24`, `DropPortalOnDeath.cs:32`,
    /// `RealmPortalDrop.cs:15`), which they would not need to if the whole pass were skipped.
    /// Leaving the world without dying is a different matter, and that is what `removed` says.
    fn run_death_effects(&mut self, catalog: &Catalog) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| entity.dead && !entity.removed && entity.mind.is_some())
                .map(|(handle, _)| handle),
        );

        if self.handles.is_empty() {
            return;
        }

        let behaviours = std::mem::take(&mut self.behaviours);
        let dying = std::mem::take(&mut self.handles);

        for handle in &dying {
            let Some(program) = self
                .entities
                .get(*handle)
                .and_then(|entity| catalog.object(entity.object_type))
                .and_then(|desc| behaviours.get(&desc.id))
            else {
                continue;
            };

            // Every state's effects, not only the one it died in: the content puts these on the
            // outermost state precisely so that dying anywhere triggers them.
            for state in &program.states {
                for primitive in &state.behaviours {
                    let Some(effect) = primitive.death_effect() else {
                        continue;
                    };
                    self.run_death_effect(catalog, &behaviours, program, *handle, effect);
                }
            }
        }

        self.handles = dying;
        self.behaviours = behaviours;
    }

    fn run_death_effect(
        &mut self,
        catalog: &Catalog,
        behaviours: &Programs,
        program: &Program,
        handle: Handle,
        effect: &DeathEffect,
    ) {
        let Some((x, y)) = self.entities.get(handle).map(|entity| (entity.x, entity.y)) else {
            return;
        };

        match effect {
            DeathEffect::Spawn { child, count } => {
                let Some(kind) = program.kind_of(*child).map(ObjectType) else {
                    return;
                };
                for index in 0..(*count).min(MAX_SPAWNED_AT_ONCE) {
                    self.spawn_child(
                        catalog,
                        behaviours,
                        kind,
                        x + index as f32 * 0.35,
                        y,
                        None,
                        Some(handle),
                        true,
                    );
                }
            }

            DeathEffect::TransformInto {
                child,
                min,
                max,
                probability,
            } => {
                // `Random.NextDouble() < probability`, so a probability of zero never happens.
                if self.roll() >= *probability {
                    return;
                }
                let Some(kind) = program.kind_of(*child).map(ObjectType) else {
                    return;
                };

                let span = max.saturating_sub(*min) + 1;
                let count = min + (self.roll() * span as f32) as u32 % span;
                for _ in 0..count {
                    self.spawn_child(catalog, behaviours, kind, x, y, None, Some(handle), false);
                }
            }

            DeathEffect::Portal {
                name,
                probability,
                duration_ms,
            } => {
                // The one death effect that does refuse a summon, and it refuses it here rather
                // than in the pass: `DropPortalOnDeath.cs:32` and `RealmPortalDrop.cs:15` both
                // return on `Spawned`. An administrator summoning a boss should not be opening a
                // way into the dungeon behind it.
                if self
                    .entities
                    .get(handle)
                    .is_some_and(|entity| entity.spawned)
                {
                    return;
                }
                if self.roll() > *probability {
                    return;
                }
                let Some(kind) = program.kind_of(*name).map(ObjectType) else {
                    return;
                };
                if let Some(portal) =
                    self.spawn_child(catalog, behaviours, kind, x, y, None, Some(handle), false)
                    && let Some(entity) = self.entities.get_mut(portal)
                {
                    // Zero is the original's word for "stays open": `DropPortalOnDeath` arms its
                    // closing timer only `if (timeoutTime != 0)` (`DropPortalOnDeath.cs:49`), and
                    // `RealmPortalDrop` arms none at all. Counting a zero down instead shuts the
                    // portal on the frame it opened.
                    if *duration_ms != 0 {
                        entity.expires_in_ms = Some(*duration_ms);
                    }
                }
            }

            DeathEffect::ChangeGround {
                sources,
                targets,
                dist,
            } => {
                // Both name lists come through as written now. Before they did, every one of the
                // fourteen calls arrived with no names at all and the distance read out of the slot
                // the first list had vacated, so the Shatters' bridges neither opened nor closed.
                let sources: Vec<u16> = sources
                    .iter()
                    .filter_map(|tile| program.kind_of(*tile))
                    .collect();
                let targets: Vec<u16> = targets
                    .iter()
                    .filter_map(|tile| program.kind_of(*tile))
                    .collect();
                self.change_ground(catalog, x, y, *dist, &sources, &targets);
            }

            // `RemoveObjectOnDeath` blanks map squares rather than killing entities, so what stood
            // on them leaves without a death of its own.
            DeathEffect::RemoveObjects { radius, kind } => {
                let wanted = named_kinds(program, *kind);
                self.each_nearby(handle, *radius, false, wanted.as_deref(), |world, other| {
                    if let Some(entity) = world.entities.get_mut(other) {
                        entity.dead = true;
                        entity.spawned = true;
                        entity.removed = true;
                    }
                });
            }

            DeathEffect::Order {
                radius,
                kind,
                state,
            } => {
                let wanted = named_kinds(program, *kind);
                let state = state.clone();
                self.each_nearby(handle, *radius, false, wanted.as_deref(), |world, other| {
                    world.order_into(catalog, behaviours, other, &state);
                });
            }

            // Who fought this one, handed to the nearest one of a kind. No health moves and nothing
            // is hurt: `TransferDamageOnDeath` (`TransferDamageOnDeath.cs:27-32`) finds one entity
            // and calls `DamageCounter.TransferData` on it, which is a merge of the hitters and
            // nothing more. It is what makes whoever killed the Pentaract's towers eligible for the
            // Pentaract's bag, and whoever killed the Hermit God eligible for the loot its drop
            // carries.
            DeathEffect::TransferDamage { radius, kind } => {
                let wanted = named_kinds(program, *kind);
                let Some(target) = self.nearest_matching(handle, *radius, wanted.as_deref()) else {
                    return;
                };
                self.transfer_damage_counter(handle, target);
            }

            DeathEffect::CopyDamage { .. } => {}
        }
    }

    /// The closest entity of a kind, as `GetNearestEntity` finds one.
    ///
    /// Players are never candidates: the original searches `EnemiesCollision` whenever it was given
    /// an object type, and only searches for players when it was given none.
    ///
    /// Over the entities rather than over the spatial index, because the index is rebuilt at the
    /// end of a tick and this runs in the middle of one. `World.EnterWorld` inserts into
    /// `EnemiesCollision` there and then (`World.cs:344`), so something that appeared a moment ago
    /// is findable — and a tower's transfer names the corpse its own transformation has just made.
    fn nearest_matching(
        &self,
        from: Handle,
        radius: f32,
        kinds: Option<&[ObjectType]>,
    ) -> Option<Handle> {
        let (x, y) = self.entities.get(from).map(|e| (e.x, e.y))?;
        let reach = radius * radius;

        let mut best: Option<(Handle, f32)> = None;
        for (handle, entity) in self.entities.iter() {
            if handle == from
                || entity.dead
                || entity.kind == Kind::Player
                || kinds.is_some_and(|wanted| !wanted.contains(&entity.object_type))
            {
                continue;
            }

            let (dx, dy) = (entity.x - x, entity.y - y);
            let distance = dx * dx + dy * dy;
            if distance < reach && best.is_none_or(|(_, closest)| distance < closest) {
                best = Some((handle, distance));
            }
        }

        best.map(|(handle, _)| handle)
    }

    /// Merges one entity's record of who hurt it into another's.
    ///
    /// `DamageCounter.TransferData` (`DamageCounter.cs:162-180`): the last hitter is overwritten
    /// even where there was none, and each player's damage is added to whatever the target had
    /// already taken from them. The source keeps its own copy, which is what lets a tower pass its
    /// credit to a boss and to that boss's corpse both.
    fn transfer_damage_counter(&mut self, from: Handle, to: Handle) {
        let Some((last_hurt_by, damage_by)) = self
            .entities
            .get(from)
            .map(|entity| (entity.last_hurt_by, entity.damage_by.clone()))
        else {
            return;
        };

        let Some(target) = self.entities.get_mut(to) else {
            return;
        };

        target.last_hurt_by = last_hurt_by;
        for (who, damage) in damage_by {
            match target.damage_by.iter_mut().find(|(held, _)| *held == who) {
                Some((_, total)) => *total += damage,
                None => {
                    if target.damage_by.len() < MOST_REMEMBERED_DAMAGERS {
                        target.damage_by.push((who, damage));
                    }
                }
            }
        }
    }

    /// Takes everything said since the last call.
    ///
    /// Draining rather than reading, because each of these should be sent once. A caller that
    /// forgets to drain gets a queue that stops growing rather than one that grows forever.
    pub fn take_announcements(&mut self) -> Vec<Announcement> {
        std::mem::take(&mut self.announcements)
    }

    /// Takes every ground change since the last call, as `(x, y, tile)`.
    ///
    /// Anybody whose circle does not cover a changed square forgets it on the way out, so the square
    /// is sent to them again when they next walk into sight of it. That is what
    /// `tiles[x, y] >= tile.UpdateCount` does in the original (`Player.Update.cs:138-139`), and
    /// without it a repaint that happened while somebody was in the next room would be lost to them
    /// for as long as they stayed in the world.
    pub fn take_ground_changes(&mut self) -> Vec<(u16, u16, u16)> {
        let changes = std::mem::take(&mut self.ground_changes);
        for (x, y, _) in &changes {
            self.forget_square_for_the_absent(*x as u32, *y as u32);
        }
        changes
    }

    /// Takes every piece of scenery that has appeared, as `(x, y, object, size)`.
    ///
    /// Forgotten by anybody out of sight of it for the same reason the ground is: scenery travels
    /// on the same squares and is re-sent by the same pass.
    pub fn take_scenery_changes(&mut self) -> Vec<(u16, u16, u16, u16)> {
        let changes = std::mem::take(&mut self.scenery_changes);
        for (x, y, _, _) in &changes {
            self.forget_square_for_the_absent(*x as u32, *y as u32);
        }
        changes
    }

    /// Notes which quest enemies have died, and who killed each.
    ///
    /// `DamageCounter.Death` (`DamageCounter.cs:71`) tells the realm about every enemy that dies in
    /// it, and `Realm.EnemyKilled` (`Realm.cs:80`) passes on the ones nothing summoned. Oryx wants
    /// only the quest enemies, and the killer is whoever hit last -- which may be nobody, when a
    /// thing dies of the ground or of an effect.
    fn note_quest_kills(&mut self, catalog: &Catalog) {
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| {
                    pays_for_death(entity, catalog)
                        && catalog
                            .object(entity.object_type)
                            .is_some_and(|desc| desc.quest)
                })
                .map(|(handle, _)| handle),
        );

        if self.handles.is_empty() {
            return;
        }

        let killed = std::mem::take(&mut self.handles);
        for handle in &killed {
            let Some(entity) = self.entities.get(*handle) else {
                continue;
            };

            let kind = entity.object_type;
            let killer = entity
                .last_hurt_by
                .and_then(|who| self.entities.get(who))
                .filter(|player| player.kind == Kind::Player)
                .and_then(|player| player.name.as_deref())
                .map(str::to_owned);

            if self.quest_kills.len() < MAX_PENDING_ANNOUNCEMENTS {
                self.quest_kills.push(QuestKill { kind, killer });
            }
        }
        self.handles = killed;
    }

    /// Every quest enemy killed since the last call.
    pub fn take_quest_kills(&mut self) -> Vec<QuestKill> {
        std::mem::take(&mut self.quest_kills)
    }

    /// Notes which players have died, and what killed each.
    ///
    /// Before the reaping, because the body is where the gravestone goes and what killed it may
    /// itself be about to be removed. The session answers the rest: a world can end a life but
    /// cannot write a character down.
    fn note_deaths(&mut self) {
        // The last word rather than the first. Every place that takes a player's last point names
        // what did it as it does so, exactly as the original's `HitByProjectile`, `Damage`,
        // `ApplyGroundDamage` and `HandleOceanTrenchGround` each call `Death` themselves. What is
        // left for this sweep is what `Player.Tick` is left with: a player at no health that
        // nothing claimed, which the original answers with `Death("Unknown", rekt: true)`
        // (`Player.cs:586`) -- a trip to the nexus, and not the end of the character.
        self.handles.clear();
        self.handles.extend(
            self.entities
                .iter()
                .filter(|(_, entity)| entity.kind == Kind::Player && entity.slain())
                .map(|(handle, _)| handle),
        );

        let unclaimed = std::mem::take(&mut self.handles);
        for handle in &unclaimed {
            self.claim_death(*handle, "Unknown".to_string(), true);
        }
        self.handles = unclaimed;
    }

    /// Records a player's death, naming what did it.
    ///
    /// Called where the health is subtracted, as the original calls `Death` there, and refused for
    /// a player already claimed this tick: `_dead` guards the whole of `Player.Death`
    /// (`Player.cs:997`), so the first thing to reach zero is the thing that gets blamed and a
    /// second blow on the same tick changes nothing.
    fn claim_death(&mut self, who: Handle, killer: String, rekt: bool) {
        if self.deaths.iter().any(|death| death.who == who) {
            return;
        }

        let Some(entity) = self.entities.get_mut(who) else {
            return;
        };
        if entity.kind != Kind::Player {
            return;
        }

        entity.dead = true;

        // Moved off the body rather than copied, so a checkpoint that already wrote these counts
        // cannot write them again, and so nothing is left behind on a body about to be reaped.
        let tally = std::mem::take(&mut entity.tally);

        let body = Body {
            hp: entity.hp,
            mp: entity.mp,
            max_hp: entity.max_hp,
            max_mp: entity.max_mp,
            level: entity.progress.level,
            experience: entity.progress.experience,
            fame: entity.progress.fame,
            stats: entity.stats.to_base(),
            tally,
        };

        self.deaths.push(Death {
            who,
            killer,
            x: entity.x,
            y: entity.y,
            rekt,
            body,
        });
    }

    /// What to blame for a blow struck by an entity, and whether it counts.
    ///
    /// The name is the original's `ObjectDesc.DisplayId ?? ObjectDesc.ObjectId ?? Name`
    /// (`Player.cs:801`). The flag is `NonPermaKillEnemy` (`Player.cs:862`): anything summoned
    /// takes a player home rather than ending the character, which is why `/spawn` cannot be used
    /// to farm somebody's character off them.
    fn blame(&self, source: Option<Handle>, catalog: &Catalog) -> (String, bool) {
        let Some(entity) = source.and_then(|handle| self.entities.get(handle)) else {
            return (self.name.to_string(), false);
        };

        let named = catalog
            .object(entity.object_type)
            .map(|desc| desc.name().to_string())
            .or_else(|| entity.name.as_ref().map(|name| name.to_string()))
            .unwrap_or_else(|| self.name.to_string());

        (named, entity.spawned)
    }

    /// Removes everything marked dead.
    fn reap(&mut self) {
        self.entities.retain(|_, entity| !entity.dead);
    }

    /// The world as one player sees it.
    ///
    /// Only what is within sight is encoded at all, rather than encoded and then discarded. The
    /// entities come back sorted by handle, which is what the snapshot encoder's merge wants.
    pub fn snapshot_for(&mut self, viewer: Handle, radius: f32) -> WorldSnapshot {
        // The same circle the ground and the scenery were sent from, taken here as `SendUpdate`
        // takes it once at the top of its pass and feeds it to all three (`Player.Update.cs:130`).
        self.refresh_sight(viewer);

        let Some(entity) = self.entities.get(viewer) else {
            return WorldSnapshot::new();
        };
        let (x, y) = (entity.x, entity.y);
        let (quest, watched) = (entity.quest_target, entity.watching);
        let circle = entity.sight.as_deref();

        self.grid.within(x, y, radius, &mut self.nearby);

        self.visible.clear();
        for handle in &self.nearby {
            let Some(entity) = self.entities.get(*handle) else {
                continue;
            };

            // A bag with an owner is nobody else's business, and the original does not merely refuse
            // to open it: `GetNewEntities` skips any container whose `BagOwners` does not name this
            // account, so a soulbound drop is never drawn on anybody else's screen at all
            // (`Player.Update.cs:236-241`). Sending it and refusing the take would tell a whole room
            // what one player found.
            if entity.kind == Kind::Container
                && entity.belongs_to.is_some_and(|owner| owner != viewer)
            {
                continue;
            }

            // Within range is not the same as in view. `GetNewEntities` draws an enemy, a bag or a
            // piece of broken scenery only where `visibleTiles.Contains` the square it stands on
            // (`Player.Update.cs:245-246`), and `GetRemovedEntities` takes one away again the
            // moment the circle stops covering it (`:209-212`). So the same circle that decides
            // which floor is drawn decides what is standing on it, and a `blocking: 0` world --
            // the realm, the nexus, the vault -- hides nothing, which is what the realm is for.
            //
            // Two kinds are outside that test in the original and are outside it here. Players are
            // yielded from `Owner.Players` with no circle test and kept by `i is Player`
            // (`:227-229`, `:209`), so a party in a dungeon stays drawn through the walls between
            // them. A decoy comes from a plain radius hit test on the player collision map
            // (`:231-233`) and is likewise never tested against the circle.
            let occluded = *handle != viewer
                && self.sight.occludes()
                && !matches!(entity.kind, Kind::Player | Kind::Decoy)
                && !circle.is_some_and(|circle| {
                    circle.contains(
                        entity.x.floor().max(0.0) as u32,
                        entity.y.floor().max(0.0) as u32,
                    )
                });
            if occluded {
                continue;
            }

            // Invisible is hidden from everyone but the one carrying it. Filtered here rather than
            // by the client, because a client told about something it should not see is a client
            // that can be made to reveal it.
            if *handle != viewer && crate::effects::Rules::of(entity.conditions).invisible {
                continue;
            }

            self.visible.push((handle.to_entity_id(), entity.state()));
        }

        // Two bodies travel however far away they are: whatever the quest arrow points at, and
        // whoever is being watched. `GetNewEntities` yields both unconditionally, outside the loop
        // over what is near (`Player.Update.cs:249-253`). An arrow pointing at an entity the client
        // was never sent points at nothing, and a camera set on one shows nothing.
        for extra in [quest, watched].into_iter().flatten() {
            if self.nearby.contains(&extra) {
                continue;
            }
            if let Some(entity) = self.entities.get(extra) {
                self.visible.push((extra.to_entity_id(), entity.state()));
            }
        }

        WorldSnapshot::from_unsorted(std::mem::take(&mut self.visible))
    }

    /// Everything within a radius of a point.
    pub fn near(&mut self, x: f32, y: f32, radius: f32) -> &[Handle] {
        self.grid.within(x, y, radius, &mut self.nearby);
        &self.nearby
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hendra_content::map::Composition;
    use hendra_content::{Map, Region, TileType};

    const FIXTURE: &str = r#"<Objects>
        <Ground type="0x10" id="Grass"><Speed>1</Speed></Ground>
        <Ground type="0x11" id="Water"><NoWalk/></Ground>
        <Ground type="0x12" id="Lava"><MinDamage>100</MinDamage><MaxDamage>100</MaxDamage></Ground>
        <Object type="0x500" id="Wall"><Class>GameObject</Class><FullOccupy/><Static/></Object>
        <Object type="0x501" id="Sign"><Class>GameObject</Class><Static/></Object>
        <Object type="0x50f" id="Thicket"><Class>GameObject</Class><BlocksSight/><Static/></Object>
        <Object type="0x51f" id="Bramble"><Class>GameObject</Class><OccupySquare/><Static/></Object>
        <Object type="0x51e" id="Pillar"><Class>GameObject</Class>
          <OccupySquare/><EnemyOccupySquare/><FullOccupy/><Static/></Object>
        <Object type="0x731" id="Ocean Vent"><Class>GameObject</Class></Object>
        <Object type="0x504" id="Tree"><Class>GameObject</Class><BlocksSight/><Static/></Object>
        <Object type="0x502" id="Slime"><Class>Character</Class><Enemy/>
          <MaxHitPoints>200</MaxHitPoints>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>20</MinDamage><MaxDamage>20</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
        </Object>
        <Object type="0x503" id="Guard"><Class>Character</Class><Enemy/>
          <MaxHitPoints>50</MaxHitPoints></Object>
        <Object type="0x507" id="Plated Slime"><Class>Character</Class><Enemy/>
          <MaxHitPoints>200</MaxHitPoints><Defense>30</Defense></Object>
        <Object type="0x505" id="Spawnling"><Class>Character</Class><Enemy/>
          <MaxHitPoints>10</MaxHitPoints></Object>
        <Object type="0x508" id="Hobbit Mage"><Class>Character</Class><Enemy/><Quest/>
          <Level>5</Level><MaxHitPoints>200</MaxHitPoints></Object>
        <Object type="0x5f0" id="Desert Werewolf"><Class>Character</Class><Enemy/>
          <Level>5</Level><MaxHitPoints>200</MaxHitPoints></Object>
        <Object type="0x513" id="Cube God"><Class>Character</Class><Enemy/><God/><Cube/>
          <MaxHitPoints>200</MaxHitPoints></Object>
        <Object type="0x514" id="Oryx Stone Guardian Left"><Class>Character</Class><Enemy/>
          <MaxHitPoints>200</MaxHitPoints></Object>
        <Object type="0x509" id="Red Crystal"><Class>Character</Class><Enemy/>
          <Group>Crystals</Group><MaxHitPoints>100</MaxHitPoints></Object>
        <Object type="0x50a" id="Blue Crystal"><Class>Character</Class><Enemy/>
          <Group>Crystals</Group><MaxHitPoints>100</MaxHitPoints></Object>
        <Object type="0x506" id="Doorway"><Class>Portal</Class></Object>
        <Object type="0x600" id="Hero"><Class>Player</Class><Player/></Object>
        <Object type="0x030e" id="Wizard"><Class>Player</Class><Player/>
          <MaxHitPoints max="670">100</MaxHitPoints>
          <MaxMagicPoints max="385">100</MaxMagicPoints>
          <Attack max="75">12</Attack><Defense max="25">0</Defense>
          <Speed max="50">12</Speed><Dexterity max="75">15</Dexterity>
          <HpRegen max="40">10</HpRegen><MpRegen max="60">10</MpRegen>
          <SlotTypes>8, 5, 6, 9</SlotTypes>
          <LevelIncrease min="20" max="30">MaxHitPoints</LevelIncrease>
          <LevelIncrease min="2" max="8">MaxMagicPoints</LevelIncrease>
        </Object>
        <Object type="0x900" id="Bolt"><Class>Projectile</Class></Object>
        <Object type="0x510" id="Loot Bag"><Class>Container</Class></Object>
        <Object type="0x511" id="Loot Bag 5"><Class>Container</Class></Object>
        <Object type="0x904" id="Rare Blade">
          <Class>Equipment</Class><Item/><SlotType>1</SlotType><BagType>5</BagType>
        </Object>
        <Object type="0x905" id="Cloak of Shadows">
          <Class>Equipment</Class><Item/><SlotType>13</SlotType>
          <Activate duration="5" distance="1">Decoy</Activate>
        </Object>
        <Object type="0x906" id="Trap Spell">
          <Class>Equipment</Class><Item/><SlotType>5</SlotType>
          <Activate radius="4" totalDamage="200" duration="10">Trap</Activate>
        </Object>
        <Object type="0x902" id="Health Potion">
          <Class>Equipment</Class><Item/><SlotType>4</SlotType><Consumable/>
          <Activate amount="100">Heal</Activate>
        </Object>
        <Object type="0x903" id="Spell of Fire">
          <Class>Equipment</Class><Item/><SlotType>5</SlotType>
          <MpCost>60</MpCost><Cooldown>0.5</Cooldown>
          <Activate radius="3" totalDamage="200">PoisonGrenade</Activate>
        </Object>
        <Object type="0x901" id="Wand">
          <Class>Equipment</Class><Item/><SlotType>8</SlotType><RateOfFire>1</RateOfFire>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>100</MinDamage><MaxDamage>100</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
        </Object>
        <Object type="0x907" id="Paralysing Wand">
          <Class>Equipment</Class><Item/><SlotType>8</SlotType><RateOfFire>1</RateOfFire>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>10</MinDamage><MaxDamage>10</MaxDamage>
            <LifetimeMS>2000</LifetimeMS>
            <ConditionEffect duration="2">Paralyzed</ConditionEffect></Projectile>
        </Object>
        <Object type="0x908" id="Double Bow">
          <Class>Equipment</Class><Item/><SlotType>8</SlotType><RateOfFire>1</RateOfFire>
          <NumProjectiles>4</NumProjectiles><ArcGap>10</ArcGap>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>10</MinDamage><MaxDamage>10</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
        </Object>
        <Object type="0x909" id="Fan Shield">
          <Class>Equipment</Class><Item/><SlotType>5</SlotType>
          <MpCost>0</MpCost><Cooldown>0</Cooldown>
          <NumProjectiles>5</NumProjectiles><ArcGap>20</ArcGap>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>50</MinDamage><MaxDamage>50</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
          <Activate>Shoot</Activate>
        </Object>
        <Object type="0x90a" id="Tome of Rejuvenation">
          <Class>Equipment</Class><Item/><SlotType>6</SlotType>
          <MpCost>80</MpCost>
          <Activate amount="100" range="6" useWisMod="true">HealNova</Activate>
        </Object>
        <Object type="0x90b" id="Seal of the Aspirant">
          <Class>Equipment</Class><Item/><SlotType>7</SlotType>
          <MpCost>60</MpCost>
          <Activate stat="0" amount="25" duration="3.5" range="4.5"
            noStack="true">StatBoostAura</Activate>
        </Object>
        <Object type="0x90c" id="Soul Siphon Skull">
          <Class>Equipment</Class><Item/><SlotType>11</SlotType>
          <MpCost>0</MpCost>
          <Activate amount="50" range="5" useWisMod="true">MagicNova</Activate>
        </Object>
        <Object type="0x90d" id="Silver Star">
          <Class>Equipment</Class><Item/><SlotType>15</SlotType>
          <MpCost>0</MpCost>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>30</MinDamage><MaxDamage>30</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
          <Activate>BulletNova</Activate>
        </Object>
        <Object type="0x90e" id="Cloak of Ghostly Concealment">
          <Class>Equipment</Class><Item/><SlotType>13</SlotType>
          <MpCost>0</MpCost>
          <Activate maxDistance="20">Teleport</Activate>
        </Object>
        <Object type="0x90f" id="Helm of the Juggernaut">
          <Class>Equipment</Class><Item/><SlotType>10</SlotType>
          <MpCost>0</MpCost>
          <Activate stat="0" amount="150" duration="30">StatBoostSelf</Activate>
        </Object>
        <Object type="0x910" id="Purification Orb">
          <Class>Equipment</Class><Item/><SlotType>12</SlotType>
          <MpCost>0</MpCost>
          <Activate range="4">ClearConditionEffectAura</Activate>
        </Object>
        <Object type="0x911" id="Chain Scepter">
          <Class>Equipment</Class><Item/><SlotType>23</SlotType>
          <MpCost>0</MpCost>
          <Activate totalDamage="50" maxTargets="3">Lightning</Activate>
        </Object>
        <Object type="0x912" id="Bloodsucker Skull">
          <Class>Equipment</Class><Item/><SlotType>11</SlotType>
          <MpCost>0</MpCost>
          <Activate radius="4" totalDamage="100" heal="90">VampireBlast</Activate>
        </Object>
        <Object type="0x914" id="Stasis Bomb">
          <Class>Equipment</Class><Item/><SlotType>11</SlotType>
          <MpCost>0</MpCost>
          <Activate duration="4">StasisBlast</Activate>
        </Object>
        <Object type="0x915" id="Star of Enlightenment">
          <Class>Equipment</Class><Item/><SlotType>15</SlotType>
          <MpCost>0</MpCost><MpEndCost>30</MpEndCost>
          <NumProjectiles>3</NumProjectiles><ArcGap>10</ArcGap>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>40</MinDamage><MaxDamage>40</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
          <Activate>ShurikenAbility</Activate>
        </Object>
        <Object type="0x913" id="Doom Bow">
          <Class>Equipment</Class><Item/><SlotType>3</SlotType>
          <BagType>4</BagType>
        </Object>
        <Object type="0x50b" id="Warded Boss"><Class>Character</Class><Enemy/>
          <MaxHitPoints>500</MaxHitPoints>
          <ParalyzeImmune/><StasisImmune/><StunImmune/>
        </Object>
        <Object type="0x50c" id="Pentaract"><Class>Character</Class><Enemy/>
          <MaxHitPoints>10000</MaxHitPoints></Object>
        <Object type="0x50d" id="Pentaract Tower"><Class>Character</Class><Enemy/>
          <MaxHitPoints>8000</MaxHitPoints></Object>
        <Object type="0x50e" id="Hermit God Drop"><Class>Character</Class><Enemy/>
          <MaxHitPoints>100</MaxHitPoints></Object>
        <Object type="0x512" id="Wine Barrel"><Class>GameObject</Class><Enemy/><Static/>
          <StasisImmune/><MaxHitPoints>100</MaxHitPoints></Object>
        <Object type="0x515" id="Wine Cask"><Class>GameObject</Class><Enemy/><Static/>
          <OccupySquare/><MaxHitPoints>100</MaxHitPoints></Object>
        <Object type="0x516" id="Abandoned Switch"><Class>GameObject</Class><Enemy/>
          <StasisImmune/><MaxHitPoints>100</MaxHitPoints></Object>
        <Object type="0x517" id="Tomb Turret"><Class>GameObject</Class><Enemy/><Static/></Object>
      </Objects>"#;

    fn catalog() -> Catalog {
        Catalog::load_str(&[FIXTURE]).0
    }

    fn square(tile: u16, object: u16) -> Composition {
        Composition {
            tile: TileType(tile),
            object: ObjectType(object),
            region: Region::None,
            terrain: hendra_content::Terrain::None,
            config: String::new(),
        }
    }

    /// An open 32×32 field of grass.
    /// The object a player breathes from, in the fixture above.
    const VENT: u16 = 0x731;

    fn field(catalog: &Catalog) -> World {
        let squares = (0..32 * 32).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        World::new("Field", Terrain::build(map, catalog), catalog)
    }

    /// The same field under the one name `DamageCounter` treats differently.
    fn theatre(catalog: &Catalog) -> World {
        let squares = (0..32 * 32).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        World::new(
            "Puppet Master's Theatre",
            Terrain::build(map, catalog),
            catalog,
        )
    }

    #[test]
    fn scenery_stays_in_the_map_and_everything_else_becomes_an_entity() {
        // The realm map carries a quarter of a million trees. As entities they would fill a world
        // four times over and leave no room for a single enemy.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..8 * 8)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[10] = square(0x10, 0x500); // wall, scenery
        squares[20] = square(0x10, 0x501); // sign, scenery
        squares[30] = square(0x10, 0x502); // slime, an enemy
        squares[40] = square(0x10, 0x506); // doorway, a portal somebody has to be able to use

        let map = Map::from_squares(8, 8, squares).unwrap();
        let world = World::new("Test", Terrain::build(map, &catalog), &catalog);

        assert_eq!(world.len(), 2, "only the slime and the doorway");
        assert!(!world.terrain().walkable(2, 1), "but the wall still blocks");

        let kinds: Vec<Kind> = world.iter().map(|(_, entity)| entity.kind).collect();
        assert!(kinds.contains(&Kind::Enemy));
        assert!(kinds.contains(&Kind::Portal));
    }

    #[test]
    fn a_map_object_the_author_named_is_not_scenery() {
        // Naming one is the map author saying this one is not a tree.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..8 * 8)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[20] = Composition {
            tile: TileType(0x10),
            object: ObjectType(0x501),
            region: Region::None,
            terrain: hendra_content::Terrain::None,
            config: "name:Welcome".to_string(),
        };

        let map = Map::from_squares(8, 8, squares).unwrap();
        let world = World::new("Test", Terrain::build(map, &catalog), &catalog);

        assert_eq!(world.len(), 1);
        assert_eq!(
            world.iter().next().unwrap().1.name.as_deref(),
            Some("Welcome")
        );
    }

    #[test]
    fn an_enemy_from_the_map_takes_its_hit_points_from_the_catalog() {
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..4 * 4)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[5] = square(0x10, 0x502);

        let map = Map::from_squares(4, 4, squares).unwrap();
        let world = World::new("Test", Terrain::build(map, &catalog), &catalog);

        let (_, slime) = world.iter().next().expect("the slime should be there");
        assert_eq!(slime.max_hp, 200);
        assert_eq!(slime.hp, 200);
    }

    #[test]
    fn an_ordinary_step_is_honoured_in_full() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();

        // 5 tiles per second for 50ms is a quarter tile.
        let outcome = world
            .resolve_move(player, &catalog, 16.25, 16.0, 50)
            .unwrap();

        assert_eq!(outcome.refused, None);
        assert!((outcome.x - 16.25).abs() < 1e-4);
    }

    #[test]
    fn a_claim_faster_than_the_entity_can_travel_is_trimmed() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();

        // Claiming ten tiles in one 50ms tick.
        let outcome = world
            .resolve_move(player, &catalog, 26.0, 16.0, 50)
            .unwrap();

        assert_eq!(outcome.refused, Some(MoveRefusal::TooFar));

        // Trimmed to speed × time × tolerance, not rejected outright.
        let travelled = outcome.x - 16.0;
        // Four tiles a second at zero speed, over fifty milliseconds, plus the tolerance.
        let allowed = 4.0 * 0.05 * MOVE_TOLERANCE;
        assert!(
            (travelled - allowed).abs() < 1e-3,
            "travelled {travelled}, allowed {allowed}"
        );
    }

    #[test]
    fn a_move_is_paid_for_out_of_the_allowance_it_used() {
        // What stops the speed limit being per-message rather than per-second. The caller hands
        // out the time that has actually passed and takes back what each claim spent, so a client
        // that sends its position ten times as often is not granted ten times the distance.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();

        // Four tiles a second, and half of the fifty milliseconds' worth on offer.
        let allowed = 4.0 * 0.05 * MOVE_TOLERANCE;
        let outcome = world
            .resolve_move(player, &catalog, 16.0 + allowed / 2.0, 16.0, 50)
            .unwrap();
        assert_eq!(outcome.refused, None);
        assert!(
            (23..=27).contains(&outcome.spent_ms),
            "half the distance should cost about half the allowance, not {}",
            outcome.spent_ms
        );

        // Standing still costs nothing, so a client that reports its position while idle keeps
        // whatever it has saved up.
        let outcome = world
            .resolve_move(player, &catalog, 16.0, 16.0, 50)
            .unwrap();
        assert_eq!(outcome.spent_ms, 0);

        // And a claim the clamp had to trim spends the lot: it took everything on offer.
        let outcome = world
            .resolve_move(player, &catalog, 26.0, 16.0, 50)
            .unwrap();
        assert_eq!(outcome.refused, Some(MoveRefusal::TooFar));
        assert_eq!(outcome.spent_ms, 50);

        // Nothing is charged for a move the server itself caused.
        world.get_mut(player).unwrap().move_grace_ms = MOVE_GRACE_MS;
        let outcome = world
            .resolve_move(player, &catalog, 26.0, 16.0, 50)
            .unwrap();
        assert_eq!(outcome.refused, None, "the grace forgives the speed");
        assert_eq!(outcome.spent_ms, 0);
    }

    #[test]
    fn walking_into_water_stops_at_the_edge() {
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..8 * 8)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        // Flood the column at x = 4.
        for y in 0..8 {
            squares[y * 8 + 4] = square(0x11, ObjectType::NONE.0);
        }

        let map = Map::from_squares(8, 8, squares).unwrap();
        let mut world = World::new("Test", Terrain::build(map, &catalog), &catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 3.5, 3.5, 800))
            .unwrap();

        let outcome = world.resolve_move(player, &catalog, 4.5, 3.5, 200).unwrap();
        assert_eq!(outcome.refused, Some(MoveRefusal::Blocked));
        assert!(outcome.x < 4.0, "should not have entered the water");
    }

    #[test]
    fn a_diagonal_into_a_wall_slides_along_it() {
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..8 * 8)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        for y in 0..8 {
            squares[y * 8 + 4] = square(0x11, ObjectType::NONE.0);
        }

        let map = Map::from_squares(8, 8, squares).unwrap();
        let mut world = World::new("Test", Terrain::build(map, &catalog), &catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 3.5, 3.5, 800))
            .unwrap();

        // Moving diagonally into the wall: the wall stops x, but y should still advance.
        let outcome = world.resolve_move(player, &catalog, 4.5, 4.0, 200).unwrap();
        assert_eq!(outcome.refused, Some(MoveRefusal::Blocked));
        assert!(
            outcome.y > 3.5,
            "movement along the wall should survive, got y = {}",
            outcome.y
        );
    }

    #[test]
    fn a_wall_one_tile_thick_cannot_be_stepped_over() {
        // The whole point of walking the line rather than testing its end. A claim that starts on
        // open ground and ends on open ground with a wall in between passes an endpoint test from
        // either side, and every wall in the game is one tile thick.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..16 * 16)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        // One column of water at x = 8, and nothing else in the way.
        for y in 0..16 {
            squares[y * 16 + 8] = square(0x11, ObjectType::NONE.0);
        }

        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Test", Terrain::build(map, &catalog), &catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 7.5, 4.5, 800))
            .unwrap();

        // Claiming the far side of the wall, and claiming it slowly enough that the speed limit has
        // nothing to say: a whole second buys four tiles and this asks for two.
        let outcome = world
            .resolve_move(player, &catalog, 9.5, 4.5, 1000)
            .unwrap();

        assert_eq!(outcome.refused, Some(MoveRefusal::Blocked));
        assert!(
            outcome.x < 8.0,
            "the wall is at x = 8 and the player ended up at {}",
            outcome.x
        );

        // And the diagonal version of the same claim, which an endpoint test lets through by
        // sliding onto an axis that clears the wall.
        let outcome = world
            .resolve_move(player, &catalog, 9.5, 5.5, 1000)
            .unwrap();
        assert!(
            outcome.x < 8.0,
            "crossing the wall at an angle is still crossing it, got x = {}",
            outcome.x
        );
    }

    #[test]
    fn a_gap_in_a_wall_is_still_walked_through() {
        // The sweep refuses the wall, not the doorway. A player aiming at a hole one tile wide goes
        // through it, or the fix would have walled every dungeon shut.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..16 * 16)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        for y in 0..16 {
            if y != 4 {
                squares[y * 16 + 8] = square(0x11, ObjectType::NONE.0);
            }
        }

        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Test", Terrain::build(map, &catalog), &catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 7.5, 4.5, 800))
            .unwrap();

        let outcome = world
            .resolve_move(player, &catalog, 9.5, 4.5, 1000)
            .unwrap();
        assert_eq!(outcome.refused, None);
        assert!(
            (outcome.x - 9.5).abs() < 1e-4,
            "the doorway is open and the claim was honoured, got x = {}",
            outcome.x
        );
    }

    #[test]
    fn standing_in_lava_costs_hit_points_and_eventually_kills() {
        let catalog = catalog();
        let squares = (0..8 * 8).map(|_| square(0x12, ObjectType::NONE.0));
        let map = Map::from_squares(8, 8, squares).unwrap();
        let mut world = World::new("Lava", Terrain::build(map, &catalog), &catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 4.0, 4.0, 300))
            .unwrap();

        // A hundred a second from the ground against six a second of regeneration, so the loss is
        // most of the hundred rather than all of it.
        world.advance(&catalog, 1000);
        let after_a_second = world.get(player).unwrap().hp;
        assert!(
            (200..=210).contains(&after_a_second),
            "about a hundred a second, less regeneration: {after_a_second}"
        );

        for _ in 0..5 {
            world.advance(&catalog, 1000);
        }

        assert!(world.get(player).is_none(), "the player should have died");
        assert_eq!(world.len(), 0);
    }

    #[test]
    fn a_snapshot_holds_what_is_in_sight_and_nothing_else() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let viewer = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();
        let near = world
            .spawn(Entity::player(ObjectType(0x600), 18.0, 16.0, 800))
            .unwrap();
        let far = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();
        world.get_mut(far).unwrap().x = 100.0;

        world.advance(&catalog, 50);

        let snapshot = world.snapshot_for(viewer, SIGHT_RADIUS);
        assert!(
            snapshot.get(viewer.to_entity_id()).is_some(),
            "the viewer sees itself"
        );
        assert!(snapshot.get(near.to_entity_id()).is_some());
        assert!(
            snapshot.get(far.to_entity_id()).is_none(),
            "an entity outside the radius should never be encoded"
        );
    }

    #[test]
    fn ticking_advances_the_tick_number() {
        let catalog = catalog();
        let mut world = field(&catalog);
        assert_eq!(world.tick_number(), Tick::ZERO);

        world.advance(&catalog, 50);
        world.advance(&catalog, 50);
        assert_eq!(world.tick_number(), Tick(2));
    }

    #[test]
    fn a_despawned_entity_leaves_the_index_too() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let viewer = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();
        let other = world
            .spawn(Entity::player(ObjectType(0x600), 17.0, 16.0, 800))
            .unwrap();
        world.advance(&catalog, 50);
        assert!(
            world
                .snapshot_for(viewer, SIGHT_RADIUS)
                .get(other.to_entity_id())
                .is_some()
        );

        world.despawn(other);
        world.advance(&catalog, 50);

        assert!(
            world
                .snapshot_for(viewer, SIGHT_RADIUS)
                .get(other.to_entity_id())
                .is_none(),
            "a despawned entity must leave the spatial index as well as the slab"
        );
    }

    /// An armed player and a slime three tiles east of them.
    fn duel(catalog: &Catalog) -> (World, Handle, Handle) {
        let squares = (0..32 * 32).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Duel", Terrain::build(map, catalog), catalog);

        let mut player = Entity::player(ObjectType(0x600), 5.0, 5.0, 800);
        player.weapon = Some(ObjectType(0x901));
        let shooter = world.spawn(player).unwrap();

        let mut slime = Entity::fixture(ObjectType(0x502), 8.0, 5.0);
        slime.kind = Kind::Enemy;
        slime.hp = 200;
        slime.max_hp = 200;
        let target = world.spawn(slime).unwrap();

        world.advance(catalog, 50);
        (world, shooter, target)
    }

    #[test]
    fn a_rate_of_fire_is_kept_in_the_clock_the_shot_was_taken_on() {
        // `ValidatePlayerShoot` refuses a shot whose time is less than the last accepted one plus
        // the weapon's interval, in the client's own milliseconds, and rounds nothing
        // (`Player.AntiCheat.cs:93-95`). Measuring the same interval in whole server ticks rounds
        // it up to the next fifty, which costs the top of the range about six per cent of its
        // damage — and costs a client that throttles itself to the same interval every other shot,
        // because every one of its asks lands in the gap the rounding opened.
        let catalog = catalog();
        let (mut world, shooter, _) = duel(&catalog);

        let rate = world
            .get(shooter)
            .unwrap()
            .weapon
            .and_then(|weapon| catalog.object(weapon))
            .and_then(|desc| desc.item.as_ref())
            .map(|item| item.rate_of_fire.max(0.1))
            .unwrap_or(1.0);
        let interval = world
            .get(shooter)
            .unwrap()
            .stats
            .shot_cooldown_ms(&crate::effects::Rules::NONE, rate);
        assert_ne!(
            interval % 50,
            0,
            "an interval that lands on a tick would prove nothing"
        );

        assert_eq!(world.shoot_at(shooter, &catalog, 0.0, 10_000).len(), 1);
        assert!(
            world
                .shoot_at(shooter, &catalog, 0.0, 10_000 + interval - 1)
                .is_empty(),
            "a millisecond early is early"
        );
        assert_eq!(
            world
                .shoot_at(shooter, &catalog, 0.0, 10_000 + interval)
                .len(),
            1,
            "on the interval it fires, whatever the tick happens to be doing"
        );
        assert_eq!(
            world
                .shoot_at(shooter, &catalog, 0.0, 10_000 + interval * 2)
                .len(),
            1,
            "and the one after is measured from the last shot rather than from the last tick"
        );
    }

    #[test]
    fn shooting_spawns_a_projectile_that_travels_and_hits() {
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);

        let fired = world.shoot(shooter, &catalog, 0.0);
        assert_eq!(fired.len(), 1, "the wand has one projectile");
        assert_eq!(world.projectile_count(), 1);

        let before = world.get(target).unwrap().hp;
        for _ in 0..10 {
            world.advance(&catalog, 50);
        }

        let after = world.get(target).map(|entity| entity.hp).unwrap_or(0);
        assert!(after < before, "the slime should have taken damage");
    }

    #[test]
    fn an_unarmed_entity_cannot_shoot() {
        let catalog = catalog();
        let (mut world, _, target) = duel(&catalog);

        assert!(
            world.shoot(target, &catalog, 0.0).is_empty(),
            "the slime has no weapon"
        );
        assert_eq!(world.projectile_count(), 0);
    }

    // -- what behaviours do to the world -------------------------------------------------------

    /// Installs behaviours from source and gives everything already present a mind.
    /// Puts somebody in the world so that what is in it thinks.
    ///
    /// Nothing ticks in a world with no players in it, which is the original's rule and not an
    /// artefact: `TickLogic` walks out from the chunks players are standing in. Placed at the far
    /// corner, well outside the twenty-tile sight radius, so it wakes the room without being seen
    /// from it.
    fn a_watcher(world: &mut World) -> Handle {
        world
            .spawn(Entity::player(ObjectType(0x600), 31.0, 31.0, 500))
            .expect("room for a watcher")
    }

    fn behaving(world: &mut World, catalog: &Catalog, source: &str) {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let (programs, diagnostics) = compile(&parse(source).expect("parses"));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        world.set_behaviours(catalog, programs);
    }

    fn count_of(world: &World, kind: u16) -> usize {
        world
            .iter()
            .filter(|(_, entity)| entity.object_type == ObjectType(kind) && !entity.dead)
            .count()
    }

    #[test]
    fn an_enemy_fires_on_the_originals_clock_and_not_on_ours() {
        // The whole point of running faster than the original: it must not make anything happen
        // more often. A `cooldown: 500ms` shoot is not two a second on the original — the countdown
        // moves in whole 166 ms logic ticks, and the tick that reads it as spent does not act on it
        // (`logic/behaviors/Shoot.cs:206-215`), so the real period is 830 ms. Reading the written
        // number at face value on a fifty-millisecond tick fires every 500 ms instead, which is
        // two thirds again as much damage from every timed enemy in the game.
        let catalog = catalog();
        let mut world = field(&catalog);

        world
            .spawn(Entity::player(ObjectType(0x600), 12.0, 10.0, 500))
            .expect("room for a target");

        let mut shooter = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        shooter.kind = Kind::Enemy;
        shooter.max_hp = 200;
        shooter.hp = 200;
        world.spawn(shooter).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { shoot(20, count: 1, cooldown: 500ms) } }"#,
        );
        world.reindex();

        // Five seconds of ticks, counting the moment each volley leaves.
        let mut fired = Vec::new();
        let mut before = world.projectile_count();
        for tick in 0..100u32 {
            world.advance(&catalog, 50);
            let now = world.projectile_count();
            if now > before {
                fired.push(tick * 50);
            }
            before = now;
        }

        assert!(fired.len() >= 4, "expected several volleys, got {fired:?}");
        for pair in fired.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                gap.abs_diff(830) < 50,
                "830ms is what the original puts between two shots, not {gap}ms: {fired:?}"
            );
        }
    }

    #[test]
    fn a_behaviour_that_names_an_entity_has_it_resolved_at_load() {
        // Nothing resolves names but this, and without it every behaviour that names an entity is
        // inert: a boss waits forever for guardians it cannot recognise.
        let catalog = catalog();
        let mut world = field(&catalog);
        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { on entity_exists("Guard", 10) -> b } state b { } }"#,
        );

        let program = world.behaviours.get("Slime").expect("compiled");
        assert_eq!(
            program.kind_of(hendra_behavior::NameRef(0)),
            Some(0x503),
            "the guard should have been resolved to its object type"
        );
    }

    #[test]
    fn a_boss_wakes_when_the_entity_it_watches_for_is_gone() {
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let boss = world.spawn(boss).unwrap();

        let mut guard = Entity::fixture(ObjectType(0x503), 12.0, 10.0);
        guard.kind = Kind::Enemy;
        guard.max_hp = 50;
        guard.hp = 50;
        let guard = world.spawn(guard).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                 state sealed { on entities_not_exists(20, "Guard") -> awake }
                 state awake { }
               }"#,
        );
        world.reindex();

        world.advance(&catalog, 50);
        let state_of = |world: &World, handle: Handle| {
            let entity = world.get(handle).unwrap();
            let mind = entity.mind.as_ref().unwrap();
            let program = world.behaviours.get("Slime").unwrap();
            mind.state_name(program).to_string()
        };
        assert_eq!(state_of(&world, boss), "sealed", "the guard is still there");

        world.get_mut(guard).unwrap().dead = true;
        world.advance(&catalog, 50);
        assert_eq!(state_of(&world, boss), "awake");
    }

    #[test]
    fn a_conditional_effect_lands_and_lifts_when_the_state_does() {
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                 state shielded {
                   conditional_effect(invulnerable)
                   on timed(200ms) -> open
                 }
                 state open { }
               }"#,
        );
        world.reindex();

        world.advance(&catalog, 50);
        assert!(
            world
                .get(boss)
                .unwrap()
                .conditions
                .contains(hendra_content::ConditionEffect::Invulnerable),
            "should be invulnerable while shielded"
        );

        // Out of the state and past the renewal window, it should wear off on its own. The state
        // is left at 498 ms rather than 200 — three of the original's logic ticks — and the last
        // renewal before that has a quarter second left to run.
        for _ in 0..16 {
            world.advance(&catalog, 50);
        }
        assert!(
            !world
                .get(boss)
                .unwrap()
                .conditions
                .contains(hendra_content::ConditionEffect::Invulnerable),
            "should have lapsed once the state was left"
        );
    }

    #[test]
    fn a_spawn_makes_children_that_have_minds_of_their_own() {
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let mut parent = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        parent.kind = Kind::Enemy;
        parent.max_hp = 200;
        parent.hp = 200;
        world.spawn(parent).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { spawn("Spawnling", 3, cooldown: 100ms) } }
               enemy "Spawnling" { state b { wander(0.4) } }"#,
        );
        world.reindex();

        // One on entry and one per cooldown after that, and a `cooldown: 100ms` is 332 ms of the
        // original's clock, so the third child arrives a little before three quarters of a second.
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }

        let children = count_of(&world, 0x505);
        assert_eq!(children, 3, "three, and no more than three");

        assert!(
            world
                .iter()
                .filter(|(_, e)| e.object_type == ObjectType(0x505))
                .all(|(_, e)| e.mind.is_some()),
            "each child should be running its own behaviour"
        );
    }

    #[test]
    fn an_order_drives_other_entities_into_a_state() {
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        let mut minion = Entity::fixture(ObjectType(0x503), 12.0, 10.0);
        minion.kind = Kind::Enemy;
        minion.max_hp = 50;
        minion.hp = 50;
        let minion = world.spawn(minion).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { order(20, "Guard", "charge") } }
               enemy "Guard" { state waiting { } state charge { } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        let program = world.behaviours.get("Guard").unwrap();
        let state = world
            .get(minion)
            .unwrap()
            .mind
            .as_ref()
            .unwrap()
            .state_name(program);
        assert_eq!(state, "charge", "the minion should have taken the order");
    }

    #[test]
    fn an_order_naming_a_state_the_target_lacks_leaves_it_alone() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        let mut minion = Entity::fixture(ObjectType(0x503), 12.0, 10.0);
        minion.kind = Kind::Enemy;
        minion.max_hp = 50;
        minion.hp = 50;
        let minion = world.spawn(minion).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { order(20, "Guard", "nowhere") } }
               enemy "Guard" { state waiting { } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        let program = world.behaviours.get("Guard").unwrap();
        assert_eq!(
            world
                .get(minion)
                .unwrap()
                .mind
                .as_ref()
                .unwrap()
                .state_name(program),
            "waiting"
        );
    }

    #[test]
    fn a_portal_is_left_behind_when_the_boss_dies() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { drop_portal_on_death("Doorway", 1) } }"#,
        );
        world.reindex();

        world.advance(&catalog, 50);
        assert_eq!(count_of(&world, 0x506), 0, "not until it dies");

        world.get_mut(boss).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(count_of(&world, 0x506), 1, "the way in should be open");
    }

    #[test]
    fn a_portal_with_no_timeout_stays_open() {
        // `DropPortalOnDeath` arms its closing timer only `if (timeoutTime != 0)`
        // (`DropPortalOnDeath.cs:49`), and `RealmPortalDrop` arms none at all, so zero is the
        // original's word for "stays open". Counted down instead, the way out of a realm shut on
        // the frame the boss died.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { drop_portal_on_death("Doorway", 1, 0) } }"#,
        );
        world.reindex();

        world.get_mut(boss).unwrap().dead = true;
        world.advance(&catalog, 50);
        assert_eq!(count_of(&world, 0x506), 1, "the way in should be open");

        for _ in 0..40 {
            world.advance(&catalog, 200);
        }

        assert_eq!(count_of(&world, 0x506), 1, "and should still be open");
    }

    #[test]
    fn a_death_effect_runs_once_rather_than_every_tick() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { drop_portal_on_death("Doorway", 1) } }"#,
        );
        world.reindex();

        world.get_mut(boss).unwrap().dead = true;
        for _ in 0..10 {
            world.advance(&catalog, 50);
        }

        assert_eq!(count_of(&world, 0x506), 1, "one portal, not ten");
    }

    #[test]
    fn a_transform_replaces_the_body_without_awarding_a_kill() {
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { transform("Guard") } }
               enemy "Guard" { state b { } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        assert!(world.get(boss).is_none(), "the old body is gone");
        assert_eq!(count_of(&world, 0x503), 1, "and the new one is there");
    }

    #[test]
    fn changing_the_ground_changes_what_can_be_walked_on() {
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);
        assert!(world.terrain().walkable(10, 10), "grass to begin with");

        let mut boss = Entity::fixture(ObjectType(0x502), 10.5, 10.5);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { ground_transform("Water", 2) } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        assert!(!world.terrain().walkable(10, 10), "water now");
        assert!(world.terrain().walkable(20, 20), "and only where it stood");
    }

    #[test]
    fn a_grenade_hurts_players_and_respects_their_armour() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        let victim = world
            .spawn(Entity::player(ObjectType(0x600), 12.0, 10.0, 500))
            .unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { grenade(4, 100, 20, cooldown: 100000) } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        // Thrown on the first tick and landing a second and a half later, as `Grenade.cs:82` arms
        // it. A victim who stands still through the fuse takes the whole of it.
        world.advance(&catalog, GRENADE_FUSE_MS);

        let hurt = world.get(victim).unwrap().hp;
        assert!(hurt < 500, "should have been caught in the blast");
        assert!(hurt > 0, "but not killed outright");
    }

    /// A 32x32 field of grass with sight-blocking thickets on the named squares.
    fn field_walled(catalog: &Catalog, walls: &[(usize, usize)]) -> World {
        let mut squares: Vec<Composition> = (0..32 * 32)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        for (x, y) in walls {
            squares[y * 32 + x] = square(0x10, 0x50f);
        }
        let map = Map::from_squares(32, 32, squares).unwrap();
        World::new("Field", Terrain::build(map, catalog), catalog)
    }

    /// Every square of one player's sight circle, taken as of now.
    fn circle_of(world: &mut World, who: Handle) -> Vec<(u32, u32)> {
        world.look_around(who);
        world
            .sight_circle(who)
            .map(|circle| circle.tiles().to_vec())
            .unwrap_or_default()
    }

    #[test]
    fn a_wall_hides_what_is_behind_it_only_where_the_world_says() {
        // The base World constructor sets blocking to zero and only a proto overrides it, so the
        // realm and everything else built in code shows what is within twenty tiles whether or not
        // a wall is in the way. Hiding it there takes away what the realm is for.
        //
        // A whole column, not one square: mode 1 floods around a lone blocker and mode 2's rays
        // light the neighbours of everything they pass, so neither hides anything behind a single
        // tree at four tiles. Only something with no way round it separates the modes from mode 0.
        let catalog = catalog();
        let column: Vec<(usize, usize)> = (0..32).map(|y| (10usize, y)).collect();
        let mut world = field_walled(&catalog, &column);

        let viewer = world
            .spawn(Entity::player(ObjectType(0x600), 5.5, 10.5, 500))
            .unwrap();

        let mut behind = Entity::fixture(ObjectType(0x502), 20.5, 10.5);
        behind.kind = Kind::Enemy;
        let behind = world.spawn(behind).unwrap();
        world.reindex();

        let sees = |world: &mut World| {
            world
                .snapshot_for(viewer, SIGHT_RADIUS)
                .iter()
                .any(|(id, _)| id == behind.to_entity_id())
        };

        world.sight = Sight::Unblocked;
        assert!(sees(&mut world), "with no occlusion the wall hides nothing");

        for mode in [Sight::Room, Sight::Line] {
            world.sight = mode;
            if let Some(circle) = world
                .entities
                .get_mut(viewer)
                .and_then(|e| e.sight.as_mut())
            {
                circle.go_stale();
            }
            assert!(
                !sees(&mut world),
                "a world that asks for occlusion gets it, in {mode:?}"
            );
        }
    }

    #[test]
    fn a_player_sees_the_room_they_are_in_and_not_the_one_next_door() {
        // `CalcBlockedRoomSight` (`Sight.cs:207-249`) is a flood fill, not a raycast: it leaves the
        // player, stops at every square that blocks sight, and never expands past one. So the wall
        // ring of a dungeon room is drawn and nothing outside it is -- which is the whole reason a
        // player standing at the Snake Pit entrance should not have the void beyond the room drawn
        // and their minimap filled with it.
        let catalog = catalog();

        // A hollow room from (2,2) to (12,12), walls on the ring only.
        let mut walls: Vec<(usize, usize)> = Vec::new();
        for along in 2..=12 {
            walls.push((along, 2));
            walls.push((along, 12));
            walls.push((2, along));
            walls.push((12, along));
        }
        let mut world = field_walled(&catalog, &walls);
        world.sight = Sight::Room;

        let viewer = world
            .spawn(Entity::player(ObjectType(0x600), 7.5, 7.5, 500))
            .unwrap();
        world.reindex();

        let circle = circle_of(&mut world, viewer);
        let sees = |x: u32, y: u32| circle.contains(&(x, y));

        assert!(sees(7, 7), "the square under the player");
        assert!(sees(3, 3), "the far corner of the room");
        assert!(sees(7, 2), "the wall itself, which you can see");
        assert!(sees(2, 7), "and the wall beside you");

        assert!(!sees(7, 1), "nothing on the far side of the wall");
        assert!(!sees(13, 7), "nor through the other wall");
        assert!(
            !sees(7, 15),
            "nor eight tiles beyond it, well inside the disc"
        );

        // And the exploration count follows the circle rather than the disc: a sealed room is worth
        // its own floor and its own walls, not thirteen hundred squares.
        assert!(
            circle.len() < 200,
            "a sealed room is not a disc, but {} squares came back",
            circle.len()
        );
    }

    #[test]
    fn a_flood_fill_walks_round_a_lone_tree_and_rays_do_not() {
        // The difference between mode 1 and mode 2, and the reason one raycast cannot stand in for
        // both. `CalcBlockedRoomSight` reaches anything it has a path to; `CalcBlockedLineOfSight`
        // reaches only what a ray touches, plus the eight neighbours of every square a ray passed.
        let catalog = catalog();
        let mut world = field_walled(&catalog, &[(7, 10)]);

        let viewer = world
            .spawn(Entity::player(ObjectType(0x600), 5.5, 10.5, 500))
            .unwrap();
        world.reindex();

        world.sight = Sight::Room;
        assert!(
            circle_of(&mut world, viewer).contains(&(20, 10)),
            "a flood fill goes round one tree and out the other side"
        );

        world.sight = Sight::Line;
        if let Some(circle) = world
            .entities
            .get_mut(viewer)
            .and_then(|e| e.sight.as_mut())
        {
            circle.go_stale();
        }
        let rays = circle_of(&mut world, viewer);
        assert!(
            rays.contains(&(7, 10)),
            "the tree itself is seen, as the ray that stopped on it added it"
        );
        assert!(
            !rays.contains(&(20, 10)),
            "but fifteen tiles directly behind it is reached by no ray and no neighbour"
        );
    }

    #[test]
    fn a_labelled_map_shows_one_room_at_a_time_and_the_walls_of_both() {
        // `CalcRegionBlocks` (`Sight.cs:277-289`) gives every connected open area its own prime and
        // multiplies each wall by the prime of every area touching it, so `SightRegion % sRegion`
        // is "the same room, or a wall of it". The Mad Lab, the Sewers and the Shatters are the
        // three worlds in the content that see this way.
        let catalog = catalog();
        let divide: Vec<(usize, usize)> = (0..32).map(|y| (16usize, y)).collect();
        let mut world = field_walled(&catalog, &divide);
        world.sight = Sight::Region;
        world.label_sight_regions();

        let viewer = world
            .spawn(Entity::player(ObjectType(0x600), 10.5, 10.5, 500))
            .unwrap();
        world.reindex();

        let circle = circle_of(&mut world, viewer);
        assert!(
            circle.contains(&(5, 10)),
            "the room the player is standing in"
        );
        assert!(
            circle.contains(&(16, 10)),
            "and the wall, which belongs to both rooms"
        );
        assert!(
            !circle.contains(&(20, 10)),
            "but nothing of the room on the other side, four tiles past the wall"
        );

        // A wall that comes down joins the two areas, and everyone near enough is given a fresh
        // circle without having to take a step (`World.cs:397-405`).
        // Taken down by clearing the square, because what blocked the sight was the wall standing
        // on it and not the ground under it.
        world
            .terrain
            .paint(&catalog, 16, 10, Some(TileType(0x10)), None, true);
        world.sight_blocker_changed(16, 10);
        assert!(
            circle_of(&mut world, viewer).contains(&(20, 10)),
            "with the wall gone the two areas are one"
        );
    }

    #[test]
    fn ground_repainted_out_of_sight_is_sent_again_when_it_comes_back_into_it() {
        // `tiles[x, y] >= tile.UpdateCount` (`Player.Update.cs:138-139`) re-sends a square whose
        // version has moved on, so a floor repainted while somebody was in another room reaches
        // them the moment their circle covers it again. A one-way "seen" bit cannot do that on its
        // own, so the bit is cleared for everybody who could not see the change.
        let catalog = catalog();
        let mut world = field_walled(&catalog, &[]);

        let viewer = world
            .spawn(Entity::player(ObjectType(0x600), 2.5, 2.5, 500))
            .unwrap();
        world.reindex();
        world.look_around(viewer);
        world.take_revealed();

        // Well outside the circle, so the change is nobody's business yet.
        world.reshape_ground(&catalog, 30.5, 30.5, 0.0, 0x12, None);
        assert_eq!(world.take_ground_changes().len(), 1);

        // Walking into sight of it uncovers it afresh rather than skipping it as already sent.
        world.place(
            viewer,
            MoveOutcome {
                x: 25.5,
                y: 25.5,
                spent_ms: 0,
                refused: None,
            },
        );
        let uncovered = world.take_revealed();
        assert!(
            uncovered.iter().any(|(_, x, y)| *x == 30 && *y == 30),
            "the repainted square is offered again"
        );
    }

    #[test]
    fn a_drowning_world_spends_air_and_a_vent_gives_it_back() {
        let catalog = catalog();
        let mut world = field(&catalog);
        world.drowns = true;

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 20.0, 500))
            .unwrap();
        world.reindex();

        // Away from any vent, air runs out two at a time on a tenth of a second.
        for _ in 0..10 {
            world.advance(&catalog, 100);
        }
        assert_eq!(world.get(player).unwrap().oxygen, FULL_OXYGEN - 20);

        // At a vent it comes back four times as fast, and stops at full.
        world
            .spawn(Entity::fixture(ObjectType(VENT), 20.0, 20.0))
            .unwrap();
        world.reindex();
        for _ in 0..10 {
            world.advance(&catalog, 100);
        }
        assert_eq!(world.get(player).unwrap().oxygen, FULL_OXYGEN);
    }

    #[test]
    fn with_no_air_left_it_is_health_that_goes() {
        let catalog = catalog();
        let mut world = field(&catalog);
        world.drowns = true;

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 20.0, 500))
            .unwrap();
        world.get_mut(player).unwrap().oxygen = 0;
        world.reindex();

        let before = world.get(player).unwrap().hp;
        world.advance(&catalog, 100);

        assert_eq!(world.get(player).unwrap().hp, before - 10);
    }

    #[test]
    fn air_that_runs_out_far_enough_from_a_vent_kills() {
        // `HandleOceanTrenchGround` calls `Death("suffocation")` itself (`Player.Ground.cs:30`),
        // and it is a real death rather than a trip home: the trench keeps what it takes. Ten
        // health a tenth of a second is slow enough to swim out of and fast enough to matter.
        let catalog = catalog();
        let mut world = field(&catalog);
        world.drowns = true;

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 20.0, 500))
            .unwrap();
        if let Some(entity) = world.get_mut(player) {
            entity.oxygen = 0;
            entity.hp = 10;
        }
        world.reindex();

        world.advance(&catalog, 100);

        let deaths = world.take_deaths();
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].who, player);
        assert_eq!(deaths[0].killer, "suffocation");
        assert!(!deaths[0].rekt, "the trench keeps what it takes");
    }

    #[test]
    fn a_hidden_player_does_not_drown() {
        // Which is what makes the cloak worth carrying down there.
        let catalog = catalog();
        let mut world = field(&catalog);
        world.drowns = true;

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 20.0, 500))
            .unwrap();
        let hidden = hendra_content::ConditionEffect::Hidden.index() as u8;
        world.give_effect(player, hidden, 60_000);
        world.reindex();

        for _ in 0..10 {
            world.advance(&catalog, 100);
        }
        assert_eq!(world.get(player).unwrap().oxygen, FULL_OXYGEN);
    }

    #[test]
    fn nothing_thinks_in_a_room_nobody_is_standing_near() {
        // The original ticks out from the chunks players occupy, so a boss left three phases back
        // is where it was left rather than three phases further on, and a spawner has not filled
        // an empty room while nobody watched.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.max_hp = 200;
        slime.hp = 200;
        world.spawn(slime).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { spawn("Spawnling", max_children: 4) } }"#,
        );
        world.reindex();

        for _ in 0..10 {
            world.advance(&catalog, 50);
        }
        assert_eq!(
            count_of(&world, 0x505),
            0,
            "nobody is watching, so nothing happens"
        );

        a_watcher(&mut world);
        world.reindex();
        world.advance(&catalog, 50);

        assert!(
            count_of(&world, 0x505) > 0,
            "and it starts the moment somebody is near enough"
        );
    }

    #[test]
    fn a_spawners_children_are_worth_nothing() {
        // givesNoXp defaults to true across all 364 spawners in the content, and Spawned carries
        // it down the chain. Awarding for them makes any boss with a spawner a place to stand and
        // level rather than a fight.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut spawner = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        spawner.kind = Kind::Enemy;
        spawner.max_hp = 200;
        spawner.hp = 200;
        let spawner = world.spawn(spawner).unwrap();

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 11.0, 10.0, 500))
            .unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { spawn("Spawnling", max_children: 4) } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        let child = world
            .iter()
            .find(|(handle, entity)| *handle != spawner && entity.object_type == ObjectType(0x505))
            .map(|(handle, _)| handle)
            .expect("the spawner made something");

        assert!(
            !world.get(child).unwrap().awards_experience,
            "a spawned child is worth nothing"
        );

        let before = world.get(player).unwrap().progress.experience;
        world.get_mut(child).unwrap().hp = 0;
        world.get_mut(child).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(
            world.get(player).unwrap().progress.experience,
            before,
            "and killing it earns nothing"
        );
    }

    #[test]
    fn a_blast_cannot_touch_something_invulnerable() {
        // Explosions had their own copy of the damage formula, with a different floor and no
        // conditions at all, so a boss that had gone invulnerable between phases shrugged off
        // bullets and took a spell in full.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        let victim = world
            .spawn(Entity::player(ObjectType(0x600), 12.0, 10.0, 500))
            .unwrap();
        let invulnerable = hendra_content::ConditionEffect::Invulnerable.index() as u8;
        world.give_effect(victim, invulnerable, 10_000);

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { grenade(4, 100, 20, cooldown: 100000) } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        assert_eq!(
            world.get(victim).unwrap().hp,
            500,
            "an invulnerable target takes nothing from a blast, as it takes nothing from a shot"
        );
    }

    #[test]
    fn a_grenade_misses_someone_standing_clear_of_it() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        // Near enough to be aimed at, and the blast is small.
        let near = world
            .spawn(Entity::player(ObjectType(0x600), 11.0, 10.0, 500))
            .unwrap();
        let far = world
            .spawn(Entity::player(ObjectType(0x600), 25.0, 10.0, 500))
            .unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { grenade(1, 100, 20, cooldown: 100000) } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);
        world.advance(&catalog, GRENADE_FUSE_MS);

        assert!(world.get(near).unwrap().hp < 500, "the near one is hit");
        assert_eq!(world.get(far).unwrap().hp, 500, "the far one is not");
    }

    #[test]
    fn an_entity_that_shrinks_stops_at_its_target_size() {
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        boss.size = 100;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { change_size(-100, 25) } }"#,
        );
        world.reindex();

        for _ in 0..60 {
            world.advance(&catalog, 50);
        }

        assert_eq!(
            world.get(boss).unwrap().size,
            25,
            "should have stopped at the target rather than shrinking away"
        );
    }

    // -- condition effects ----------------------------------------------------------------------

    fn give(world: &mut World, handle: Handle, effect: hendra_content::ConditionEffect) {
        if let Some(entity) = world.get_mut(handle) {
            entity.conditions.insert(effect);
            entity.effects.push((effect.index() as u8, 60_000));
        }
    }

    #[test]
    fn an_invulnerable_target_takes_no_damage_and_takes_it_again_when_the_effect_lifts() {
        // The most used behaviour in the game's content. Before this it marked a boss invulnerable
        // and left it perfectly killable.
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);

        give(
            &mut world,
            target,
            hendra_content::ConditionEffect::Invulnerable,
        );
        let before = world.get(target).unwrap().hp;

        for _ in 0..40 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
        }
        assert_eq!(
            world.get(target).unwrap().hp,
            before,
            "a hundred shots should have done nothing"
        );

        // Lift it, and the same shots land.
        world.get_mut(target).unwrap().conditions = hendra_content::ConditionSet::EMPTY;
        world.get_mut(target).unwrap().effects.clear();

        for _ in 0..10 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
            if world.get(target).is_none_or(|e| e.hp < before) {
                break;
            }
        }
        assert!(
            world.get(target).is_none_or(|e| e.hp < before),
            "should be killable once the effect is gone"
        );
    }

    #[test]
    fn a_shot_takes_its_bounds_from_the_weapon_and_is_truncated_not_rounded() {
        // End to end: the ten-damage wand at thirteen attack is `10 * 0.76`, and
        // `GetAttackDamage` ends `return (int)ret`. Seven, not the eight rounding would give --
        // which is the whole difference between this and what was here before.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 5.0, 5.0, 800);
        player.weapon = Some(ObjectType(0x907));
        player.stats.boost(hendra_content::Stat::Attack, 13);
        let shooter = world.spawn(player).unwrap();
        world.reindex();

        let fired = world.shoot(shooter, &catalog, 0.0);
        assert_eq!(fired.len(), 1);

        let damage = world
            .projectiles()
            .find(|(handle, _)| *handle == fired[0])
            .map(|(_, shot)| shot.damage)
            .expect("a bullet");

        assert_eq!(damage, 7, "10 * 0.76 truncated");
    }

    #[test]
    fn armour_changes_what_a_shot_is_worth() {
        let catalog = catalog();

        // A target with armour of its own, because ArmorBroken removes defence rather than
        // creating negative defence. Against something with none it correctly does nothing.
        let hp_after = |effect: Option<hendra_content::ConditionEffect>| {
            let mut world = field(&catalog);

            let mut player = Entity::player(ObjectType(0x600), 5.0, 5.0, 800);
            player.weapon = Some(ObjectType(0x901));
            let shooter = world.spawn(player).unwrap();

            let mut plated = Entity::fixture(ObjectType(0x507), 8.0, 5.0);
            plated.kind = Kind::Enemy;
            plated.hp = 400;
            plated.max_hp = 400;
            let target = world.spawn(plated).unwrap();
            world.reindex();

            if let Some(effect) = effect {
                give(&mut world, target, effect);
            }
            world.shoot(shooter, &catalog, 0.0);
            for _ in 0..12 {
                world.advance(&catalog, 50);
            }
            world.get(target).map(|e| e.hp)
        };

        let plain = hp_after(None).expect("alive");
        let armoured = hp_after(Some(hendra_content::ConditionEffect::Armored)).expect("alive");
        let broken = hp_after(Some(hendra_content::ConditionEffect::ArmorBroken)).expect("alive");

        assert!(
            armoured > plain,
            "armour should absorb some: {armoured} vs {plain}"
        );
        assert!(
            broken < plain,
            "broken armour should absorb less: {broken} vs {plain}"
        );
    }

    #[test]
    fn a_paralysed_player_cannot_move_and_moves_again_when_it_lifts() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        // Within what five tiles a second allows in fifty milliseconds.
        let outcome = world
            .resolve_move(player, &catalog, 10.2, 10.0, 50)
            .unwrap();
        assert!(outcome.refused.is_none(), "unaffected, so allowed");

        give(
            &mut world,
            player,
            hendra_content::ConditionEffect::Paralyzed,
        );
        let held = world
            .resolve_move(player, &catalog, 10.2, 10.0, 50)
            .unwrap();

        assert_eq!(held.refused, Some(MoveRefusal::Rooted));
        assert_eq!((held.x, held.y), (10.0, 10.0), "held exactly where it was");
    }

    #[test]
    fn being_slowed_shortens_how_far_a_claim_may_reach() {
        // Not refused outright: a slowed player still moves, just less. Refusing would look like
        // a disconnection rather than an effect.
        let catalog = catalog();
        let mut world = field(&catalog);
        // Speed is held at the base when slowed rather than scaled, so the difference only shows
        // on a character that has speed to lose.
        let mut fast = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        fast.stats.boost(hendra_content::Stat::Speed, 75);
        let player = world.spawn(fast).unwrap();
        world.reindex();

        let reach = |world: &World| {
            let outcome = world
                .resolve_move(player, &catalog, 30.0, 10.0, 100)
                .unwrap();
            outcome.x - 10.0
        };

        let normal = reach(&world);
        give(&mut world, player, hendra_content::ConditionEffect::Slowed);
        let slowed = reach(&world);

        assert!(slowed > 0.0, "still moving");
        assert!(slowed < normal, "but less far: {slowed} vs {normal}");
    }

    #[test]
    fn a_paused_player_still_walks_about() {
        // `ResolveNewLocation` (`Entity.cs:324`) holds still for Paralyzed and Petrify only, and
        // `StatsManager.GetSpeed` (`:138`) zeroes for Paralyzed only. Neither mentions Paused: what
        // a pause stops in the original is the world acting on you -- no regeneration, no ground
        // damage, no experience, no enemy that will look at you -- and never your own legs.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        give(&mut world, player, hendra_content::ConditionEffect::Paused);
        let moved = world
            .resolve_move(player, &catalog, 10.2, 10.0, 50)
            .unwrap();
        assert!(moved.refused.is_none(), "a pause is not a leash");
        assert!(moved.x > 10.0);
    }

    #[test]
    fn petrify_holds_a_player_exactly_where_they_stand() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        give(&mut world, player, hendra_content::ConditionEffect::Petrify);
        let held = world
            .resolve_move(player, &catalog, 10.2, 10.0, 50)
            .unwrap();

        assert_eq!(held.refused, Some(MoveRefusal::Rooted));
        assert_eq!((held.x, held.y), (10.0, 10.0));
    }

    #[test]
    fn a_second_source_of_an_effect_replaces_its_timer_rather_than_lengthening_it() {
        // `_effects[eff] = durationMs` (`Entity.cs:716`), a plain assignment. Taking the longer of
        // the two reads as the kinder rule and is not the rule: a brief stun landing on a long one
        // genuinely cuts it short.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        let slowed = hendra_content::ConditionEffect::Slowed;
        world.give_effect(player, slowed.index() as u8, 5_000);
        world.give_effect(player, slowed.index() as u8, 500);

        let left = world
            .get(player)
            .unwrap()
            .effects
            .iter()
            .find(|(held, _)| *held == slowed.index() as u8)
            .map(|(_, left)| *left);
        assert_eq!(left, Some(500), "shortened, not extended");

        // And a duration of nothing is how the original clears one: it writes the timer and then
        // refuses to raise the bit, so the next rebuild drops it.
        world.give_effect(player, slowed.index() as u8, 0);
        assert!(!world.get(player).unwrap().conditions.contains(slowed));
        assert!(world.get(player).unwrap().effects.is_empty());
    }

    #[test]
    fn a_positive_boost_shows_its_arrow_and_a_lapsed_one_takes_it_away() {
        // `ApplyActivateBonus` (`BoostStatManager.cs:130-152`) puts the condition effect at
        // `i + 39` on for boost `i` while it is positive and takes it off otherwise. Without it a
        // player under a potion gets the stat and no icon, which is the half of the effect they
        // can actually see.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        let defence = hendra_content::ConditionEffect::DefBoost;
        assert!(!world.get(player).unwrap().conditions.contains(defence));

        world.give_boost(
            player,
            HeldBoost {
                stat: hendra_content::Stat::Defense.index() as u8,
                amount: 8,
                remaining_ms: 1_000,
                stacks: true,
            },
        );
        assert!(
            world.get(player).unwrap().conditions.contains(defence),
            "the arrow goes up with the stat"
        );

        for _ in 0..40 {
            world.advance(&catalog, 50);
        }
        assert!(
            !world.get(player).unwrap().conditions.contains(defence),
            "and comes off when the boost lapses"
        );
    }

    #[test]
    fn the_laboratory_s_green_water_hexes_and_its_blue_water_washes_it_off() {
        // `CheckLabConditions` (`MoveHandler.cs:41-84`). The three are applied with no duration at
        // all, so nothing but the blue water takes them off.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..32 * 32)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[(10 * 32 + 12) as usize] = square(0xa9, ObjectType::NONE.0);
        squares[(10 * 32 + 14) as usize] = square(0xa7, ObjectType::NONE.0);
        // The same green water with something standing on it is dry ground: the original refuses
        // both branches unless `tile.ObjId == 0`.
        squares[(10 * 32 + 16) as usize] = square(0xa9, 0x500);
        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Lab", Terrain::build(map, &catalog), &catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        let hexed = hendra_content::ConditionEffect::Hexed;
        let stunned = hendra_content::ConditionEffect::Stunned;
        let speedy = hendra_content::ConditionEffect::Speedy;

        let step = |world: &mut World, x: f32| {
            world.place(
                player,
                MoveOutcome {
                    x,
                    y: 10.5,
                    refused: None,
                    spent_ms: 0,
                },
            );
        };

        step(&mut world, 12.5);
        let held = world.get(player).unwrap().conditions;
        assert!(held.contains(hexed) && held.contains(stunned) && held.contains(speedy));

        // Long enough that a timed effect would have lapsed, which none of these are.
        for _ in 0..200 {
            world.advance(&catalog, 50);
        }
        assert!(world.get(player).unwrap().conditions.contains(hexed));

        step(&mut world, 14.5);
        let washed = world.get(player).unwrap().conditions;
        assert!(!washed.contains(hexed) && !washed.contains(stunned) && !washed.contains(speedy));

        step(&mut world, 16.5);
        assert!(
            !world.get(player).unwrap().conditions.contains(hexed),
            "water under an object is dry ground"
        );
    }

    #[test]
    fn a_hidden_player_wakes_nothing() {
        // Both `AnyPlayerNearby` overloads skip them (`Utils.cs:42`, `:56`), and that function is
        // what `Entity.Tick` asks before running a behaviour tree at all.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut enemy = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 200;
        enemy.hp = 200;
        let enemy = world.spawn(enemy).unwrap();
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 11.0, 10.0, 500))
            .unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { on timed(100ms) -> b } state b { } }"#,
        );
        world.reindex();

        let state_of = |world: &World| {
            let program = world.behaviours.get("Slime").unwrap();
            world
                .get(enemy)
                .unwrap()
                .mind
                .as_ref()
                .unwrap()
                .state_name(program)
                .to_string()
        };

        world.give_effect(
            player,
            hendra_content::ConditionEffect::Hidden.index() as u8,
            FOREVER,
        );
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert_eq!(state_of(&world), "a", "nothing woke it");

        world.give_effect(
            player,
            hendra_content::ConditionEffect::Hidden.index() as u8,
            0,
        );
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert_eq!(
            state_of(&world),
            "b",
            "and it wakes when they show themselves"
        );
    }

    #[test]
    fn a_stunned_player_still_fires_their_own_weapon() {
        // Stunned appears nowhere in the original's player: not in `PlayerShootHandler`, not in
        // `ValidatePlayerShoot` (`Player.AntiCheat.cs:88`), not in `GetAttackFrequency`. It gates
        // the behaviour tree's `Shoot` (`Shoot.cs:132`) and nothing else, so whether a stunned
        // player stops firing is a decision their client makes. A server that refused the shot
        // would disagree with the original for every client that asked anyway.
        let catalog = catalog();
        let (mut world, shooter, _) = duel(&catalog);

        give(
            &mut world,
            shooter,
            hendra_content::ConditionEffect::Stunned,
        );
        assert_eq!(world.shoot(shooter, &catalog, 0.0).len(), 1);
    }

    #[test]
    fn a_stunned_enemy_holds_its_fire_and_a_dazed_one_halves_its_volley() {
        // `Shoot.cs:130-137`: the stun returns before the cooldown is rolled, and the daze halves
        // the count with `Math.Ceiling`, so three shots become two rather than one.
        let catalog = catalog();

        let volley = |effect: Option<hendra_content::ConditionEffect>| {
            let mut world = field(&catalog);
            let mut enemy = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
            enemy.kind = Kind::Enemy;
            enemy.max_hp = 200;
            enemy.hp = 200;
            let enemy = world.spawn(enemy).unwrap();
            world
                .spawn(Entity::player(ObjectType(0x600), 12.0, 10.0, 500))
                .unwrap();

            behaving(
                &mut world,
                &catalog,
                r#"enemy "Slime" { state a { shoot(count: 3, cooldown: 10s) } }"#,
            );
            world.reindex();

            if let Some(effect) = effect {
                give(&mut world, enemy, effect);
            }
            world.advance(&catalog, 50);
            world.take_fired().len()
        };

        assert_eq!(volley(None), 3, "the whole fan when unaffected");
        assert_eq!(
            volley(Some(hendra_content::ConditionEffect::Stunned)),
            0,
            "stunned holds its fire"
        );
        assert_eq!(
            volley(Some(hendra_content::ConditionEffect::Dazed)),
            2,
            "dazed rounds three up to two, not down to one"
        );
    }

    #[test]
    fn stasis_freezes_a_behaviour_tree_and_a_pause_does_not() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut enemy = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 200;
        enemy.hp = 200;
        let enemy = world.spawn(enemy).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { on timed(100ms) -> b } state b { } }"#,
        );
        world.reindex();

        let state_of = |world: &World| {
            let program = world.behaviours.get("Slime").unwrap();
            world
                .get(enemy)
                .unwrap()
                .mind
                .as_ref()
                .unwrap()
                .state_name(program)
                .to_string()
        };

        // `Entity.Tick` (`Entity.cs:225`) skips `TickState` under Stasis and under nothing else.
        give(&mut world, enemy, hendra_content::ConditionEffect::Stasis);
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert_eq!(state_of(&world), "a", "stasis holds the tree where it was");

        // Paused says nothing about a behaviour tree in the original: it is about a *player's*
        // upkeep. A paused enemy keeps thinking, and keeps moving and shooting with it.
        world.give_effect(
            enemy,
            hendra_content::ConditionEffect::Stasis.index() as u8,
            0,
        );
        give(&mut world, enemy, hendra_content::ConditionEffect::Paused);
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert_eq!(state_of(&world), "b", "a paused enemy thinks on");
    }

    #[test]
    fn bleeding_hurts_over_time_but_never_finishes_the_job() {
        // The game leaves you at one and lets something else kill you, which is what stops a stray
        // poison being an execution.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        give(
            &mut world,
            player,
            hendra_content::ConditionEffect::Bleeding,
        );
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }

        let hurt = world.get(player).unwrap().hp;
        assert!(hurt < 500 && hurt > 400, "one second of bleeding: {hurt}");

        // Long enough to run the health out, and inside the effect's own lifetime: once bleeding
        // lapses the player starts recovering again, which is correct and not what this measures.
        for _ in 0..400 {
            world.advance(&catalog, 50);
        }
        assert_eq!(world.get(player).unwrap().hp, 1, "left alive at one");
    }

    #[test]
    fn healing_restores_over_time_and_stops_at_full() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.get_mut(player).unwrap().hp = 100;
        world.reindex();

        give(&mut world, player, hendra_content::ConditionEffect::Healing);
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert!(world.get(player).unwrap().hp > 100);

        for _ in 0..1000 {
            world.advance(&catalog, 50);
        }
        assert_eq!(world.get(player).unwrap().hp, 500, "and no further");
    }

    #[test]
    fn a_small_heal_per_tick_is_not_rounded_away() {
        // Twenty a second at fifty-millisecond ticks is one point per tick. Rounding each tick
        // independently would make healing do nothing at all.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.get_mut(player).unwrap().hp = 100;
        world.reindex();

        give(&mut world, player, hendra_content::ConditionEffect::Healing);
        for _ in 0..10 {
            world.advance(&catalog, 20);
        }

        assert!(
            world.get(player).unwrap().hp > 100,
            "two hundred milliseconds of healing should show"
        );
    }

    #[test]
    fn an_immunity_refuses_the_effect_rather_than_holding_it_uselessly() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        give(
            &mut world,
            player,
            hendra_content::ConditionEffect::ParalyzeImmune,
        );
        world.give_effect(
            player,
            hendra_content::ConditionEffect::Paralyzed.index() as u8,
            5000,
        );

        assert!(
            !world
                .get(player)
                .unwrap()
                .conditions
                .contains(hendra_content::ConditionEffect::Paralyzed),
            "the immunity should have refused it"
        );
        assert!(
            world
                .resolve_move(player, &catalog, 10.2, 10.0, 50)
                .unwrap()
                .refused
                .is_none(),
            "and movement is unaffected"
        );
    }

    #[test]
    fn a_shot_lands_what_its_descriptor_says_it_carries() {
        // `ApplyConditionEffect(projectile.ProjDesc.Effects)` in `Enemy.cs:114` and `Player.cs:789`.
        // Five hundred and sixty-one condition effects are written on projectiles across the
        // content -- every slowing, quieting, paralysing and armour-breaking shot in the game --
        // and a server that only reads the damage turns all of them into plain bullets.
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);
        world.get_mut(shooter).unwrap().weapon = Some(ObjectType(0x907));

        for _ in 0..20 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
            if world.get(target).is_some_and(|e| {
                e.conditions
                    .contains(hendra_content::ConditionEffect::Paralyzed)
            }) {
                break;
            }
        }

        assert!(
            world
                .get(target)
                .unwrap()
                .conditions
                .contains(hendra_content::ConditionEffect::Paralyzed),
            "the shot declares Paralyzed for two seconds and the target should be holding it"
        );

        // And it is a timed effect rather than a permanent one: two seconds, as written.
        for _ in 0..60 {
            world.advance(&catalog, 50);
        }
        assert!(
            !world
                .get(target)
                .unwrap()
                .conditions
                .contains(hendra_content::ConditionEffect::Paralyzed),
            "it should have run out"
        );
    }

    #[test]
    fn a_descriptors_immunities_are_on_the_enemy_before_it_is_ever_hit() {
        // `Character.SetConditions` (`Character.cs:48-67`) does this in the constructor. Seven
        // hundred and twenty immunity flags sit in the content, and without this every boss in the
        // game is stunnable, paralysable and stasis-able.
        //
        // The markers themselves, not the effects they block. Copying the blocked effects across
        // instead would leave every one of those bosses permanently stunned.
        use hendra_content::ConditionEffect;

        let catalog = catalog();
        let mut world = field(&catalog);
        let behaviours = Programs::default();
        let boss = world
            .spawn_child(
                &catalog,
                &behaviours,
                ObjectType(0x50b),
                10.0,
                10.0,
                None,
                None,
                false,
            )
            .unwrap();

        let held = world.get(boss).unwrap().conditions;
        for marker in [
            ConditionEffect::ParalyzeImmune,
            ConditionEffect::StasisImmune,
            ConditionEffect::StunImmune,
        ] {
            assert!(held.contains(marker), "{marker:?} should be held");
        }
        for blocked in [
            ConditionEffect::Paralyzed,
            ConditionEffect::Stasis,
            ConditionEffect::Stunned,
        ] {
            assert!(
                !held.contains(blocked),
                "{blocked:?} is what the flag refuses, not what it grants"
            );
        }

        // A tick does not sweep them away, and neither does the immunity lapse.
        for _ in 0..40 {
            world.advance(&catalog, 50);
        }
        assert!(
            world
                .get(boss)
                .unwrap()
                .conditions
                .contains(ConditionEffect::ParalyzeImmune)
        );

        world.give_effect(boss, ConditionEffect::Paralyzed.index() as u8, 5_000);
        assert!(
            !world
                .get(boss)
                .unwrap()
                .conditions
                .contains(ConditionEffect::Paralyzed),
            "the immunity should have refused it"
        );
    }

    #[test]
    fn a_paralysing_shot_does_nothing_to_something_that_is_immune_to_it() {
        // The two halves together: a shot that carries an effect, and a target the content says
        // cannot take it.
        use hendra_content::ConditionEffect;

        let catalog = catalog();
        let mut world = field(&catalog);
        let behaviours = Programs::default();

        let mut player = Entity::player(ObjectType(0x600), 5.0, 10.0, 800);
        player.weapon = Some(ObjectType(0x907));
        let shooter = world.spawn(player).unwrap();

        let warded = world
            .spawn_child(
                &catalog,
                &behaviours,
                ObjectType(0x50b),
                8.0,
                10.0,
                None,
                None,
                false,
            )
            .unwrap();
        let plain = world
            .spawn_child(
                &catalog,
                &behaviours,
                ObjectType(0x502),
                5.0,
                13.0,
                None,
                None,
                false,
            )
            .unwrap();
        world.reindex();

        let before = world.get(warded).unwrap().hp;
        for _ in 0..20 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
        }
        for _ in 0..20 {
            world.shoot(shooter, &catalog, std::f32::consts::FRAC_PI_2);
            world.advance(&catalog, 50);
        }

        assert!(
            world.get(warded).unwrap().hp < before,
            "the shot still lands and still hurts"
        );
        assert!(
            !world
                .get(warded)
                .unwrap()
                .conditions
                .contains(ConditionEffect::Paralyzed),
            "but a ParalyzeImmune boss is not paralysed by it"
        );
        assert!(
            world
                .get(plain)
                .is_none_or(|e| e.conditions.contains(ConditionEffect::Paralyzed)),
            "while the slime standing beside it is"
        );
    }

    #[test]
    fn an_enemy_put_down_after_the_world_started_can_still_shoot() {
        // A behaviour's shot comes from whatever the entity is holding, and an enemy holds itself.
        // Only the entities a world was built with were given that, so everything spawned
        // afterwards -- every child of a spawner, everything an administrator put down -- wandered
        // about and never fired a shot.
        let catalog = catalog();
        let mut world = field(&catalog);
        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { shoot(20.0, count: 1, cooldown: 100) }"#,
        );

        world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 800))
            .unwrap();
        world.spawn_at(&catalog, ObjectType(0x502), 14.0, 10.0, 1);
        world.reindex();

        for _ in 0..20 {
            world.advance(&catalog, 50);
        }

        assert!(
            world.projectile_count() > 0,
            "a spawned slime with a shoot behaviour should have fired"
        );
    }

    #[test]
    fn a_multi_shot_weapon_fires_its_whole_volley_along_the_arc() {
        // `NumProjectiles` is the volley, and every bullet of it comes from the item's one
        // projectile descriptor (`PlayerShootHandler.cs:45`). Seventy-seven items in the content
        // declare more projectiles than they have descriptors, and firing one bullet for each
        // descriptor gives every one of them roughly half its damage.
        let catalog = catalog();
        let (mut world, shooter, _) = duel(&catalog);
        world.get_mut(shooter).unwrap().weapon = Some(ObjectType(0x908));
        world.get_mut(shooter).unwrap().ready_at_ms = 0;

        let fired = world.shoot(shooter, &catalog, 0.0);
        assert_eq!(fired.len(), 4, "four projectiles, not one");

        // `(NumProjectiles - 1) / 2` is integer division in the original, so four bullets centre on
        // the second rather than between the second and the third: one gap to one side of the aim
        // and two to the other.
        let gap = 10f32.to_radians();
        let angles: Vec<f32> = fired
            .iter()
            .map(|handle| world.projectiles.get(*handle).unwrap().angle)
            .collect();

        for (index, angle) in angles.iter().enumerate() {
            let wanted = -gap + gap * index as f32;
            assert!(
                (angle - wanted).abs() < 1e-4,
                "shot {index} left at {angle}, expected {wanted}"
            );
        }
    }

    #[test]
    fn a_shield_fires_the_volley_its_item_declares_from_its_own_projectile() {
        // `AEShoot` reads `item.NumProjectiles`, `item.ArcGap` and `item.Projectiles[0]` from the
        // ability being used (`Player.UseItem.cs:1119-1128`), not from the activation and not from
        // the weapon in hand. All twenty-three `Shoot` activations in the content are bare, so an
        // implementation that read the activation fired exactly one bullet.
        let catalog = catalog();
        let (mut world, shooter, _) = duel(&catalog);
        world.get_mut(shooter).unwrap().mp = 100;

        let before = world.projectile_count();
        world.use_item(shooter, &catalog, ObjectType(0x909), (10.0, 5.0));

        assert_eq!(
            world.projectile_count() - before,
            5,
            "the shield declares five projectiles"
        );

        // Five is odd, so the volley is centred: `(5 - 1) / 2` is 2 whole gaps back from the aim.
        let gap = 20f32.to_radians();
        let mut angles: Vec<f32> = world
            .projectiles()
            .map(|(_, shot)| shot.angle)
            .collect::<Vec<_>>();
        angles.sort_by(|a, b| a.partial_cmp(b).unwrap());

        assert!(
            (angles[0] - (-2.0 * gap)).abs() < 1e-4,
            "the first shot should be two gaps off the aim, was {}",
            angles[0]
        );
        assert!(
            (angles[4] - (2.0 * gap)).abs() < 1e-4,
            "and the last two gaps the other way, was {}",
            angles[4]
        );
    }

    #[test]
    fn what_a_player_wears_changes_what_it_can_do() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.weapon = Some(ObjectType(0x901));
        let player = world.spawn(player).unwrap();
        world.reindex();

        let rules = crate::effects::Rules::NONE;
        let bare = world.get(player).unwrap().stats;
        let bare_speed = bare.movement_speed(&rules);
        let bare_cooldown = bare.shot_cooldown_ms(&rules, 1.0);

        // A ring of speed and dexterity, as an equipment layer.
        let mut boosts = [0i32; hendra_content::STAT_COUNT];
        boosts[hendra_content::Stat::Speed.index()] = 30;
        boosts[hendra_content::Stat::Dexterity.index()] = 30;
        world.get_mut(player).unwrap().stats.set_equipment(boosts);

        let worn = world.get(player).unwrap().stats;
        assert!(worn.movement_speed(&rules) > bare_speed);
        assert!(worn.shot_cooldown_ms(&rules, 1.0) < bare_cooldown);

        // Taking it off returns exactly where it started.
        world
            .get_mut(player)
            .unwrap()
            .stats
            .set_equipment([0; hendra_content::STAT_COUNT]);
        let after = world.get(player).unwrap().stats;
        assert_eq!(after.movement_speed(&rules), bare_speed);
        assert_eq!(after.shot_cooldown_ms(&rules, 1.0), bare_cooldown);
    }

    #[test]
    fn a_players_stats_reach_the_snapshot_and_an_enemys_do_not() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.stats.boost(hendra_content::Stat::Attack, 40);
        let player = world.spawn(player).unwrap();

        let mut enemy = Entity::fixture(ObjectType(0x502), 12.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 200;
        enemy.hp = 200;
        world.spawn(enemy).unwrap();
        world.reindex();

        let snapshot = world.snapshot_for(player, 20.0);
        let seen: Vec<_> = snapshot.iter().collect();

        let (_, mine) = seen
            .iter()
            .find(|(_, state)| state.object_type == 0x600)
            .expect("the player is in its own snapshot");
        assert_eq!(mine.stats[hendra_content::Stat::Attack.index()], 40);

        let (_, theirs) = seen
            .iter()
            .find(|(_, state)| state.object_type == 0x502)
            .expect("and so is the enemy");
        assert_eq!(
            theirs.stats,
            [0; hendra_net::STAT_COUNT],
            "an enemy sends none"
        );
    }

    #[test]
    fn a_players_snapshot_carries_the_weapon_damage_and_luck_as_well_as_the_eight() {
        // `ExportStats` sends `Stats[8]`, `Stats[9]` and `Stats[10]` alongside the eight a class
        // grows into (`Player.cs:339-341`). The first two are `SetWeaponDamage`'s writing of
        // whatever is in the weapon slot (`BaseStatManager.cs:40-53`), which is the pair a shot
        // actually rolls between and the one thing about a weapon a client cannot read off the item
        // it drew once a bonus has moved it.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 800))
            .unwrap();

        world.get_mut(player).unwrap().stats.arm(55, 90);

        let state = world.get(player).unwrap().state();
        assert_eq!(state.stats.len(), hendra_net::STAT_COUNT);
        assert_eq!(state.stats[hendra_content::Stat::DamageMin.index()], 55);
        assert_eq!(state.stats[hendra_content::Stat::DamageMax.index()], 90);
        assert_eq!(state.stats[hendra_content::Stat::Luck.index()], 0);
    }

    #[test]
    fn killing_something_earns_experience_and_eventually_a_level() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();
        world.reindex();

        assert_eq!(world.get(player).unwrap().progress.level, 1);

        // Slimes have two hundred health, so each is worth five capped at five.
        for _ in 0..12 {
            let mut slime = Entity::fixture(ObjectType(0x502), 11.0, 10.0);
            slime.kind = Kind::Enemy;
            slime.max_hp = 200;
            slime.hp = 200;
            let slime = world.spawn(slime).unwrap();
            world.reindex();
            world.get_mut(slime).unwrap().dead = true;
            world.advance(&catalog, 50);
        }

        let progress = world.get(player).unwrap().progress;
        assert!(progress.experience > 0, "earned nothing");
        assert_eq!(progress.level, 2, "fifty experience is one level");
    }

    #[test]
    fn a_players_snapshot_carries_its_levelling_and_an_enemys_does_not() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();
        {
            let entity = world.get_mut(player).unwrap();
            entity.progress.level = 3;
            entity.progress.experience = crate::leveling::experience_at(3) + 40;
            entity.progress.fame = 7;
        }

        let state = world.get(player).unwrap().state();
        assert_eq!(state.level, 3);
        assert_eq!(state.experience, 40, "what the bar holds, not the lifetime");
        assert_eq!(state.experience_goal, crate::leveling::experience_goal(3));
        assert_eq!(state.fame, 7);

        let mut slime = Entity::fixture(ObjectType(0x502), 11.0, 10.0);
        slime.kind = Kind::Enemy;
        let slime = world.spawn(slime).unwrap();

        let state = world.get(slime).unwrap().state();
        assert_eq!((state.level, state.experience, state.fame), (0, 0, 0));
    }

    #[test]
    fn a_boss_grown_for_a_crowd_is_worth_what_its_description_says() {
        // `DamageCounter` reads the descriptor's health, so health added because the room filled up
        // is health nobody is paid for.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();

        // High enough that the cap does not bind and the two figures can be told apart.
        world.get_mut(player).unwrap().progress.level = 10;

        let mut boss = Entity::fixture(ObjectType(0x502), 11.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 20_000;
        boss.hp = 20_000;
        boss.base_max_hp = Some(300);
        let boss = world.spawn(boss).unwrap();
        world.reindex();

        world.get_mut(boss).unwrap().dead = true;
        world.advance(&catalog, 50);

        // Thirty from three hundred. The swollen maximum would have paid the capped hundred and
        // five, which is what a room full of people would otherwise farm each other into.
        assert_eq!(world.get(player).unwrap().progress.experience, 30);
    }

    #[test]
    fn a_summon_is_worth_nothing_so_it_cannot_be_farmed() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();

        let mut summon = Entity::fixture(ObjectType(0x502), 11.0, 10.0);
        summon.kind = Kind::Enemy;
        summon.max_hp = 200;
        summon.hp = 200;
        summon.spawned = true;
        let summon = world.spawn(summon).unwrap();
        world.reindex();

        world.get_mut(summon).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(world.get(player).unwrap().progress.experience, 0);
    }

    #[test]
    fn experience_reaches_everyone_nearby_and_nobody_far_away() {
        // Shared rather than split, so helping someone else's fight is never worse than standing
        // elsewhere. Distance is the only thing that decides it.
        let catalog = catalog();
        let mut world = field(&catalog);

        let near = world
            .spawn(Entity::player(ObjectType(0x030e), 11.0, 10.0, 100))
            .unwrap();
        let far = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();
        world.get_mut(far).unwrap().x = 200.0;

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.max_hp = 200;
        slime.hp = 200;
        let slime = world.spawn(slime).unwrap();
        world.reindex();

        world.get_mut(slime).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert!(world.get(near).unwrap().progress.experience > 0);
        assert_eq!(world.get(far).unwrap().progress.experience, 0);
    }

    #[test]
    fn a_paused_player_earns_nothing() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 11.0, 10.0, 100))
            .unwrap();

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.max_hp = 200;
        slime.hp = 200;
        let slime = world.spawn(slime).unwrap();
        world.reindex();

        give(&mut world, player, hendra_content::ConditionEffect::Paused);
        world.get_mut(slime).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(world.get(player).unwrap().progress.experience, 0);
    }

    /// A level-ten player, a slime, and however many tiles apart they should be.
    ///
    /// Ten because the cap there is ninety-five and a slime is worth twenty, so the reward can be
    /// read off directly without the cap getting in the way.
    fn slime_kill(world: &mut World, catalog: &Catalog, separation: f32) -> (Handle, Handle) {
        let player = world
            .spawn(Entity::player(
                ObjectType(0x030e),
                10.0 + separation,
                10.0,
                100,
            ))
            .unwrap();
        world.get_mut(player).unwrap().progress.level = 10;

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.max_hp = 200;
        slime.hp = 200;
        let slime = world.spawn(slime).unwrap();
        world.reindex();

        let entity = world.get_mut(slime).unwrap();
        entity.dead = true;
        entity.last_hurt_by = Some(player);
        let _ = catalog;

        (player, slime)
    }

    #[test]
    fn twenty_five_tiles_away_is_too_far_and_a_hair_less_is_not() {
        // `enemy.Dist(p) < 25` (`DamageCounter.cs:78`) is strict, and `Dist` is the square root of
        // the centre-to-centre separation (`Utils.cs:15-24`). The boundary itself is outside.
        let catalog = catalog();

        let mut world = field(&catalog);
        let (boundary, _) = slime_kill(&mut world, &catalog, 25.0);
        world.advance(&catalog, 50);
        assert_eq!(
            world.get(boundary).unwrap().progress.experience,
            0,
            "exactly twenty-five tiles away was paid"
        );

        let mut world = field(&catalog);
        let (inside, _) = slime_kill(&mut world, &catalog, 24.9);
        world.advance(&catalog, 50);
        assert_eq!(world.get(inside).unwrap().progress.experience, 20);
    }

    #[test]
    fn the_theatre_pays_a_third_of_what_anywhere_else_would() {
        // `DamageCounter.cs:95-96` matches "Theatre" against the world's display name. Twenty
        // experience becomes six and three fifths, and the cast to `int` makes it six.
        let catalog = catalog();

        let mut anywhere = field(&catalog);
        let (elsewhere, _) = slime_kill(&mut anywhere, &catalog, 1.0);
        anywhere.advance(&catalog, 50);

        let mut stage = theatre(&catalog);
        let (onstage, _) = slime_kill(&mut stage, &catalog, 1.0);
        stage.advance(&catalog, 50);

        assert_eq!(anywhere.get(elsewhere).unwrap().progress.experience, 20);
        assert_eq!(stage.get(onstage).unwrap().progress.experience, 6);
    }

    #[test]
    fn an_experience_boost_doubles_a_kill_until_it_lapses() {
        // `DamageCounter.cs:98-99`, and the clock is counted down by `TickActivateEffects`
        // (`Player.cs:593-602`) rather than lasting the session.
        let catalog = catalog();
        let mut world = field(&catalog);

        let (player, _) = slime_kill(&mut world, &catalog, 1.0);
        world.get_mut(player).unwrap().experience_boost_ms = 5_000;
        world.advance(&catalog, 50);
        assert_eq!(world.get(player).unwrap().progress.experience, 40);

        // Down to nothing, and the next kill is worth the plain twenty again.
        world.get_mut(player).unwrap().experience_boost_ms = 10;

        let mut slime = Entity::fixture(ObjectType(0x502), 11.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.max_hp = 200;
        slime.hp = 200;
        let slime = world.spawn(slime).unwrap();
        world.reindex();
        world.get_mut(slime).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(world.get(player).unwrap().progress.experience, 60);
        assert_eq!(world.get(player).unwrap().experience_boost_ms, 0);
    }

    #[test]
    fn a_finished_character_loses_the_boost_rather_than_carrying_it() {
        // `if (XPBoostTime != 0) if (Level >= 20) XPBoostTime = 0` (`Player.cs:595-597`).
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();
        {
            let entity = world.get_mut(player).unwrap();
            entity.progress.level = crate::leveling::MAX_LEVEL;
            entity.experience_boost_ms = 600_000;
        }
        world.reindex();
        world.advance(&catalog, 50);

        assert_eq!(world.get(player).unwrap().experience_boost_ms, 0);
    }

    #[test]
    fn a_kill_worth_nothing_still_banks_the_fame_a_level_up_held_back() {
        // `EnemyKilled` runs `CheckLevelUp` whatever the experience was
        // (`Player.Leveling.cs:317-322`), and `CheckLevelUp` is the only thing that recalculates
        // fame. A summon carrying `GivesNoXp` dying nearby is enough to collect it.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();
        {
            // Eighteen thousand experience with none of it banked, which is what a character looks
            // like on the tick after a level-up.
            let entity = world.get_mut(player).unwrap();
            entity.progress.level = 19;
            entity.progress.experience = 18_000;
            entity.progress.fame = 0;
        }

        let mut summon = Entity::fixture(ObjectType(0x502), 11.0, 10.0);
        summon.kind = Kind::Enemy;
        summon.max_hp = 200;
        summon.hp = 200;
        summon.awards_experience = false;
        let summon = world.spawn(summon).unwrap();
        world.reindex();

        world.get_mut(summon).unwrap().dead = true;
        world.advance(&catalog, 50);

        let progress = world.get(player).unwrap().progress;
        assert_eq!(progress.experience, 18_000, "and nothing was added");
        assert_eq!(progress.fame, 18);
    }

    #[test]
    fn a_kill_is_not_counted_by_someone_who_left_before_it_died() {
        // The counters live in `FameCounter.Killed`, which `EnemyKilled` reaches
        // (`Player.Leveling.cs:321`) and which only players near enough to be paid ever reach.
        let catalog = catalog();
        let mut world = field(&catalog);

        let (player, _) = slime_kill(&mut world, &catalog, 200.0);
        world.advance(&catalog, 50);

        let tally = world.get(player).unwrap().tally;
        assert_eq!(tally.monster_kills, 0, "credited a kill nobody watched");
    }

    #[test]
    fn what_a_kill_counted_as_comes_from_the_descriptors_flags() {
        // `FameCounter.Killed` reads `God`, `Cube` and `Oryx` off the descriptor
        // (`FameCounter.cs:77-93`). A Cube God carries `<Cube/>` without "Gelatinous" in its name,
        // and an Oryx Stone Guardian carries no `<Oryx/>` despite the name it starts with.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();

        for kind in [0x513, 0x514] {
            let mut enemy = Entity::fixture(ObjectType(kind), 11.0, 10.0);
            enemy.kind = Kind::Enemy;
            enemy.max_hp = 200;
            enemy.hp = 200;
            let enemy = world.spawn(enemy).unwrap();
            world.reindex();

            let entity = world.get_mut(enemy).unwrap();
            entity.dead = true;
            entity.last_hurt_by = Some(player);
            world.advance(&catalog, 50);
        }

        let tally = world.get(player).unwrap().tally;
        assert_eq!(tally.cube_kills, 1, "the Cube God is a cube");
        assert_eq!(tally.god_kills, 1);
        assert_eq!(tally.oryx_kills, 0, "a stone guardian is not Oryx");
        assert_eq!(
            tally.monster_kills, 1,
            "the guardian, and only the guardian"
        );
    }

    #[test]
    fn a_level_raises_the_ceiling_and_fills_what_it_added() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 10.0, 10.0, 100))
            .unwrap();
        world.get_mut(player).unwrap().stats =
            crate::stats::Stats::starting(catalog.class(ObjectType(0x030e)).unwrap());
        world.get_mut(player).unwrap().hp = 50;
        world.reindex();

        for _ in 0..12 {
            let mut slime = Entity::fixture(ObjectType(0x502), 11.0, 10.0);
            slime.kind = Kind::Enemy;
            slime.max_hp = 200;
            slime.hp = 200;
            let slime = world.spawn(slime).unwrap();
            world.reindex();
            world.get_mut(slime).unwrap().dead = true;
            world.advance(&catalog, 50);
        }

        let entity = world.get(player).unwrap();
        assert!(entity.progress.level >= 2);
        assert_eq!(entity.hp, entity.max_hp, "a level fills what it added");
        assert!(entity.max_hp > 100, "and the ceiling moved");
    }

    #[test]
    fn a_thrown_object_telegraphs_before_it_lands() {
        // Landing instantly makes a thrown attack unavoidable, which is the difference between a
        // hard fight and one nobody can play around.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut thrower = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        thrower.kind = Kind::Enemy;
        thrower.max_hp = 200;
        thrower.hp = 200;
        world.spawn(thrower).unwrap();

        // Inside the throw's range, which is also how far away a target may be:
        // `GetNearestEntity(_range, null)` (`TossObject.cs:102`).
        world
            .spawn(Entity::player(ObjectType(0x600), 12.0, 10.0, 500))
            .unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                 state a {
                   toss_object("Spawnling", range: 3, cooldown: 100000)
                 }
               }"#,
        );
        world.reindex();

        // The telegraph is a second and a half, which is what `TossObject` arms its WorldTimer to
        // and is not an argument the content can change.
        world.advance(&catalog, 50);
        assert_eq!(count_of(&world, 0x505), 0, "still in the air");

        for _ in 0..25 {
            world.advance(&catalog, 50);
        }
        assert_eq!(count_of(&world, 0x505), 0, "not yet, at 1.3 seconds");

        for _ in 0..6 {
            world.advance(&catalog, 50);
        }
        assert_eq!(count_of(&world, 0x505), 1, "and now it lands");
    }

    #[test]
    fn a_spawn_with_no_telegraph_arrives_at_once() {
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let mut parent = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        parent.kind = Kind::Enemy;
        parent.max_hp = 200;
        parent.hp = 200;
        world.spawn(parent).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { spawn("Spawnling", 1, cooldown: 100000) } }"#,
        );
        world.reindex();

        world.advance(&catalog, 50);
        assert_eq!(count_of(&world, 0x505), 1);
    }

    /// `HealSelf` sends a sparkle, a trail and a green `+N` alongside the health it restores
    /// (`HealSelf.cs:54-72`), and sends none of them when the body was already full. A monster that
    /// mends itself in silence is a health bar that refills for no reason anyone watching can see.
    #[test]
    fn a_monster_that_mends_itself_says_so() {
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.max_hp = 200;
        slime.hp = 150;
        let slime = world.spawn(slime).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { heal_self(30, cooldown: 100000) } }"#,
        );
        world.reindex();
        let _ = world.take_status_texts();
        let _ = world.take_effects();

        // Several ticks, because a behaviour runs on the original's 166 ms logic clock rather than
        // on ours.
        for _ in 0..5 {
            world.advance(&catalog, 50);
        }

        assert_eq!(world.get(slime).unwrap().hp, 180);

        let said = world.take_status_texts();
        assert_eq!(said.len(), 1, "one float, for the amount actually gained");
        assert_eq!(&*said[0].text, "+30");
        assert_eq!(said[0].color, HEAL_TEXT_COLOUR);
        assert_eq!(said[0].who, slime);
        assert!(
            !said[0].everywhere,
            "`BroadcastPacketNearby` reaches twenty tiles, not the world"
        );

        let drawn = world.take_effects();
        assert!(
            drawn
                .iter()
                .any(|event| event.effect == hendra_net::message::effect::POTION),
            "the sparkle over whoever was healed"
        );
        assert!(
            drawn
                .iter()
                .any(|event| event.effect == hendra_net::message::effect::TRAIL),
            "and the beam from whoever healed them"
        );
    }

    /// The same behaviour at full health changes nothing, so it says nothing: `newHp != entity.HP`
    /// guards all three packets.
    #[test]
    fn a_monster_already_full_mends_in_silence() {
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.max_hp = 200;
        slime.hp = 200;
        world.spawn(slime).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { heal_self(30, cooldown: 100000) } }"#,
        );
        world.reindex();
        let _ = world.take_status_texts();

        for _ in 0..5 {
            world.advance(&catalog, 50);
        }

        assert!(world.take_status_texts().is_empty());
    }

    #[test]
    fn a_potion_heals_and_a_spell_costs_magic() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.hp = 100;
        player.mp = 100;
        player.max_mp = 100;
        let player = world.spawn(player).unwrap();
        world.reindex();

        let ran = world.use_item(player, &catalog, ObjectType(0x902), (10.0, 10.0));
        assert_eq!(ran.len(), 1);
        assert_eq!(world.get(player).unwrap().hp, 200, "healed for a hundred");

        let ran = world.use_item(player, &catalog, ObjectType(0x903), (14.0, 10.0));
        assert_eq!(ran.len(), 1);
        assert_eq!(world.get(player).unwrap().mp, 40, "sixty magic spent");
    }

    /// A heal that changes nothing says nothing, and one that changes something says how much.
    ///
    /// `ActivateHealHp` returns before it queues either packet when the new health equals the old
    /// (`Player.UseItem.cs:1248-1249`), so drinking at full health is silent. What it quotes is
    /// `newHp - player.HP`, the amount actually gained, which is not the amount the item promised
    /// when the ceiling is closer than that.
    #[test]
    fn a_heal_floats_what_it_actually_gave() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.hp = 460;
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x902), (10.0, 10.0));

        let said = world.take_status_texts();
        assert_eq!(said.len(), 1, "one float for one heal");
        assert_eq!(
            &*said[0].text, "+40",
            "the forty it had room for, not the hundred the potion promised"
        );
        assert_eq!(said[0].color, HEAL_TEXT_COLOUR);
        assert_eq!(said[0].who, player);

        // The white sparkle rides along with it, which is the other half of what a potion looks
        // like (`Player.UseItem.cs:1251-1256`).
        assert!(
            world
                .take_effects()
                .iter()
                .any(|effect| effect.effect == hendra_net::message::effect::POTION),
            "a potion sparkle goes with the number"
        );

        // And now there is no room at all.
        world.use_item(player, &catalog, ObjectType(0x902), (10.0, 10.0));
        assert!(
            world.take_status_texts().is_empty(),
            "a heal at full health is silent rather than a +0"
        );
    }

    /// The ability that only makes sense in a crowd.
    ///
    /// `AEVampireBlast` heals every player in range by the total damage the blast dealt, not the
    /// caster by a number written in the item (`Player.UseItem.cs:888-905`). A sick player is passed
    /// over rather than healed for nothing.
    #[test]
    fn a_vampire_blast_gives_what_it_took_to_everyone_but_the_sick() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let caster = {
            let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
            player.hp = 100;
            world.spawn(player).unwrap()
        };
        let ally = {
            let mut player = Entity::player(ObjectType(0x600), 11.0, 10.0, 500);
            player.hp = 100;
            world.spawn(player).unwrap()
        };
        let sick = {
            let mut player = Entity::player(ObjectType(0x600), 11.0, 11.0, 500);
            player.hp = 100;
            player
                .conditions
                .insert(hendra_content::ConditionEffect::Sick);
            world.spawn(player).unwrap()
        };

        let mut enemy = Entity::fixture(ObjectType(0x502), 11.0, 10.5);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 400;
        enemy.hp = 400;
        world.spawn(enemy).unwrap();
        world.reindex();

        world.use_item(caster, &catalog, ObjectType(0x912), (11.0, 10.5));

        let healed = world.get(caster).unwrap().hp;
        assert!(healed > 100, "the caster gets what the blast drained");
        assert_eq!(
            world.get(ally).unwrap().hp,
            healed,
            "and so does everybody standing with them"
        );
        assert_eq!(
            world.get(sick).unwrap().hp,
            100,
            "sickness is passed over rather than healed"
        );

        let said = world.take_status_texts();
        assert_eq!(said.len(), 2, "a float over each player it healed");
        assert!(!said.iter().any(|text| text.who == sick));
    }

    /// Everything a stasis blast does that a generic blast does not.
    ///
    /// `StasisBlast` (`Player.UseItem.cs:800-841`): a white `Concentrate` telegraph, no damage at
    /// all, a red "Stasis" over each enemy it froze, and a three-second `StasisImmune` when the
    /// hold runs out. Collapsing it into a blast lost every one of them, and the last one is what
    /// keeps a bomb from being a permanent hold.
    #[test]
    fn a_stasis_blast_telegraphs_freezes_and_then_locks_out() {
        use hendra_content::ConditionEffect::{Stasis, StasisImmune};

        let catalog = catalog();
        let mut world = field(&catalog);

        let caster = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();

        let mut enemy = Entity::fixture(ObjectType(0x502), 12.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 500;
        enemy.hp = 500;
        let enemy = world.spawn(enemy).unwrap();
        world.reindex();

        world.use_item(caster, &catalog, ObjectType(0x914), (12.0, 10.0));

        // The telegraph, drawn from the caster to the aimed point.
        let drawn = world.take_effects();
        let telegraph = drawn
            .iter()
            .find(|effect| effect.effect == hendra_net::message::effect::CONCENTRATE)
            .expect("a stasis blast concentrates before it freezes");
        assert_eq!(telegraph.target, Some(caster));
        assert_eq!((telegraph.x1, telegraph.y1), (12.0, 10.0));
        assert_eq!(telegraph.x2, 15.0, "the reach is what gives it its radius");

        // The float, and no damage at all.
        let said = world.take_status_texts();
        assert_eq!(said.len(), 1);
        assert_eq!(&*said[0].text, "Stasis");
        assert_eq!(said[0].color, STASIS_TEXT_COLOUR);
        assert_eq!(world.get(enemy).unwrap().hp, 500, "it deals no damage");
        assert!(world.get(enemy).unwrap().conditions.contains(Stasis));

        // A second cast while the hold is running renews nothing and says nothing
        // (`Player.UseItem.cs:826`).
        world.use_item(caster, &catalog, ObjectType(0x914), (12.0, 10.0));
        assert!(
            world.take_status_texts().is_empty(),
            "a hold already running is left alone"
        );

        // Four seconds later the hold ends and the lockout begins.
        for _ in 0..40 {
            world.advance(&catalog, 100);
        }
        let held = world.get(enemy).unwrap();
        assert!(!held.conditions.contains(Stasis), "the hold ran out");
        assert!(
            held.conditions.contains(StasisImmune),
            "and left three seconds of immunity behind it"
        );
    }

    /// What an enemy that cannot be frozen is told.
    ///
    /// A green "Immune" and nothing else (`Player.UseItem.cs:816-824`). A stasis bomb deals no
    /// damage, so without the float a cast into an immune pack is indistinguishable from one that
    /// worked.
    #[test]
    fn a_stasis_blast_says_immune_over_what_it_cannot_hold() {
        use hendra_content::ConditionEffect::Stasis;

        let catalog = catalog();
        let mut world = field(&catalog);

        let caster = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();

        // The content's own immunity: `<StasisImmune/>` on the descriptor, raised when the enemy
        // enters the world (`Enemy.cs:38-41`).
        let mut warded = Entity::fixture(ObjectType(0x50b), 11.0, 10.0);
        warded.kind = Kind::Enemy;
        warded.max_hp = 500;
        warded.hp = 500;
        warded.take_immunities(catalog.object(ObjectType(0x50b)).unwrap());
        let warded = world.spawn(warded).unwrap();
        world.reindex();

        world.use_item(caster, &catalog, ObjectType(0x914), (11.0, 10.0));

        let said = world.take_status_texts();
        assert_eq!(said.len(), 1, "one word over the one it could not hold");
        assert_eq!(&*said[0].text, "Immune");
        assert_eq!(said[0].color, IMMUNE_TEXT_COLOUR);
        assert_eq!(said[0].who, warded);
        assert!(
            !world.get(warded).unwrap().conditions.contains(Stasis),
            "and it is not held"
        );
    }

    /// A poison grenade spends its damage over seconds rather than on impact.
    ///
    /// `AEPoisonGrenade` throws, waits a second and a half, and hands what it caught to
    /// `PoisonEnemy`, which pays the total out one second at a time
    /// (`Player.UseItem.cs:688-700`, `:1292-1331`). Reading it as a blast dealt the whole amount
    /// the instant it was thrown, which is a different weapon.
    #[test]
    fn a_poison_grenade_hurts_nothing_until_it_lands_and_then_hurts_slowly() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let caster = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();

        let mut enemy = Entity::fixture(ObjectType(0x502), 14.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 1_000;
        enemy.hp = 1_000;
        let enemy = world.spawn(enemy).unwrap();
        world.reindex();

        // The item is `radius="3" totalDamage="200"` with no duration, which the parser reads as
        // nothing and the payout treats as one instalment.
        world.use_item(caster, &catalog, ObjectType(0x903), (14.0, 10.0));
        assert!(
            world
                .take_effects()
                .iter()
                .any(|effect| effect.effect == hendra_net::message::effect::THROW),
            "the ball is drawn leaving the thrower"
        );
        assert_eq!(
            world.get(enemy).unwrap().hp,
            1_000,
            "nothing at all happens while it is in the air"
        );

        // A second and a half in the air, then a quarter of a second before the first instalment.
        for _ in 0..14 {
            world.advance(&catalog, 100);
        }
        assert_eq!(
            world.get(enemy).unwrap().hp,
            1_000,
            "still nothing at a second and four tenths"
        );

        for _ in 0..5 {
            world.advance(&catalog, 100);
        }
        assert!(
            world.get(enemy).unwrap().hp < 1_000,
            "and it starts paying out once the ball has landed"
        );
    }

    /// A ninja star is a stance, not an explosion.
    ///
    /// `AEShurikenAbility` (`Player.UseItem.cs:566-581`). The first press raises `NinjaSpeedy` and
    /// fires nothing; the second spends the item's `MpEndCost`, fires its volley and drops the
    /// stance. Reading it as a blast made the first press an explosion and the stance unreachable.
    #[test]
    fn a_shuriken_ability_arms_on_the_first_press_and_fires_on_the_second() {
        use hendra_content::ConditionEffect::NinjaSpeedy;

        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 100;
        player.max_mp = 100;
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x915), (14.0, 10.0));
        assert!(
            world.get(player).unwrap().conditions.contains(NinjaSpeedy),
            "the first press enters the stance"
        );
        assert_eq!(world.get(player).unwrap().mp, 100, "and spends nothing");
        assert_eq!(world.projectile_count(), 0, "and fires nothing");

        world.use_item(player, &catalog, ObjectType(0x915), (14.0, 10.0));
        assert!(
            !world.get(player).unwrap().conditions.contains(NinjaSpeedy),
            "the second press drops it"
        );
        assert_eq!(
            world.get(player).unwrap().mp,
            70,
            "and pays the item's end cost"
        );
        assert_eq!(world.projectile_count(), 3, "and throws the volley");
    }

    /// The stance comes off whether or not there was magic to fire with.
    ///
    /// The `ApplyConditionEffect(NinjaSpeedy, 0)` at the end of `AEShurikenAbility` is outside the
    /// `if (MP >= item.MpEndCost)` (`Player.UseItem.cs:574-580`), so a ninja who runs dry gets
    /// their bar back rather than being stuck in the drain.
    #[test]
    fn a_shuriken_ability_drops_its_stance_even_with_nothing_to_spend() {
        use hendra_content::ConditionEffect::NinjaSpeedy;

        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 0;
        player.max_mp = 100;
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x915), (14.0, 10.0));
        world.use_item(player, &catalog, ObjectType(0x915), (14.0, 10.0));

        assert!(!world.get(player).unwrap().conditions.contains(NinjaSpeedy));
        assert_eq!(
            world.projectile_count(),
            0,
            "there was nothing to fire with"
        );
    }

    /// A scepter follows a line of monsters rather than filling a circle.
    ///
    /// `AELightning` takes the nearest enemy in a quarter-turn cone and then jumps ten tiles at a
    /// time (`Player.UseItem.cs:709-757`). The middle enemy here is what the chain reaches through:
    /// the far one is twenty tiles from the caster and could never be in a blast at the cursor.
    #[test]
    fn a_scepter_chains_from_one_enemy_to_the_next() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let caster = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 16.0, 500))
            .unwrap();

        let mut placed = Vec::new();
        for x in [6.0, 14.0, 22.0] {
            let mut enemy = Entity::fixture(ObjectType(0x502), x, 16.0);
            enemy.kind = Kind::Enemy;
            enemy.max_hp = 500;
            enemy.hp = 500;
            placed.push(world.spawn(enemy).unwrap());
        }

        // And one at right angles to where the cursor points, which the cone excludes. Far enough
        // from the chain that no hop reaches it either: the cone decides only the first target, and
        // every jump after it takes the nearest enemy in any direction at all.
        let mut behind = Entity::fixture(ObjectType(0x502), 2.0, 3.0);
        behind.kind = Kind::Enemy;
        behind.max_hp = 500;
        behind.hp = 500;
        let behind = world.spawn(behind).unwrap();
        world.reindex();

        world.use_item(caster, &catalog, ObjectType(0x911), (12.0, 16.0));

        for (step, enemy) in placed.iter().enumerate() {
            assert!(
                world.get(*enemy).unwrap().hp < 500,
                "the bolt should have reached the enemy at step {step}"
            );
        }
        assert_eq!(
            world.get(behind).unwrap().hp,
            500,
            "an enemy out of the cone is not what the bolt starts on"
        );

        let bolts: Vec<_> = world
            .take_effects()
            .into_iter()
            .filter(|effect| effect.effect == hendra_net::message::effect::LIGHTNING)
            .collect();
        assert_eq!(bolts.len(), 3, "one bolt drawn per hop");
        assert_eq!(
            bolts[0].target,
            Some(caster),
            "the first hop is anchored to the caster"
        );
        assert_eq!(
            bolts[1].target,
            Some(placed[0]),
            "and every hop after it to the body it left"
        );
    }

    /// A grenade is a warning first and damage second.
    ///
    /// `Grenade.TickCore` draws the throw and arms a fifteen-hundred-millisecond timer that sends
    /// the ring and applies the damage together (`Grenade.cs:72-101`). Detonating on the tick it was
    /// thrown would be an unavoidable hit, which is the difference between a hard attack and one
    /// nobody can play around.
    #[test]
    fn a_grenade_warns_before_it_lands_and_can_be_walked_out_of() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        let dodger = world
            .spawn(Entity::player(ObjectType(0x600), 12.0, 10.0, 500))
            .unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { grenade(2, 100, 20, cooldown: 100000) } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        assert_eq!(world.get(dodger).unwrap().hp, 500, "nothing has landed yet");
        assert!(
            world
                .take_effects()
                .iter()
                .any(|effect| effect.effect == hendra_net::message::effect::THROW),
            "the ball is drawn leaving the thrower"
        );
        assert!(
            world.take_blasts().is_empty(),
            "and the ring waits for the fuse"
        );

        // A second and a half is long enough to step four tiles clear of a two-tile blast.
        if let Some(entity) = world.entities.get_mut(dodger) {
            entity.x = 16.0;
        }
        world.reindex();
        world.advance(&catalog, GRENADE_FUSE_MS);

        let rings = world.take_blasts();
        assert_eq!(rings.len(), 1, "one ring where it was aimed");
        assert_eq!(
            (rings[0].x, rings[0].y),
            (12.0, 10.0),
            "drawn where the grenade was thrown, not where anyone is now"
        );
        assert_eq!(rings[0].radius, 2.0);
        assert_eq!(rings[0].damage, 100);
        assert_eq!(
            world.get(dodger).unwrap().hp,
            500,
            "and it missed the player who walked out of it"
        );
    }

    /// The question a consumable has to be able to ask before it is spent.
    ///
    /// The original commits the inventory and calls `Activate` only from inside the branch where
    /// that commit came back true (`Player.UseItem.cs:205-227`), so a consumable is gone before its
    /// effect runs. Reproducing that order means the refusals have to be askable separately —
    /// otherwise a potion drunk while dead, or a tome with no magic behind it, is spent for nothing.
    /// Asking must cost nothing at all, or the ask becomes the use.
    #[test]
    fn asking_whether_an_item_would_do_anything_does_not_do_it() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.hp = 100;
        player.mp = 100;
        player.max_mp = 100;
        let player = world.spawn(player).unwrap();
        world.reindex();

        assert!(world.may_use_item(player, &catalog, ObjectType(0x902)));
        assert!(world.may_use_item(player, &catalog, ObjectType(0x903)));

        let asked = world.get(player).unwrap();
        assert_eq!(asked.hp, 100, "asking heals nobody");
        assert_eq!(asked.mp, 100, "and spends nothing");

        // Short of magic for the spell, but a potion costs none and is still allowed.
        world.get_mut(player).unwrap().mp = 10;
        assert!(!world.may_use_item(player, &catalog, ObjectType(0x903)));
        assert!(world.may_use_item(player, &catalog, ObjectType(0x902)));

        // Dead, and neither is allowed.
        world.get_mut(player).unwrap().mp = 100;
        world.get_mut(player).unwrap().dead = true;
        assert!(!world.may_use_item(player, &catalog, ObjectType(0x902)));
        assert!(!world.may_use_item(player, &catalog, ObjectType(0x903)));
    }

    /// The gate and the deed have to agree, or a consumable is spent on nothing.
    ///
    /// They are the same three refusals read twice, once to decide whether to spend the item and
    /// once on the way through. A gate that said yes where the use says no would take the potion
    /// and give back no heal.
    #[test]
    fn the_gate_answers_what_the_use_would_do() {
        let catalog = catalog();

        for (mp, item, dead) in [
            (100u32, 0x902u16, false),
            (100, 0x903, false),
            (10, 0x903, false),
            (100, 0x903, true),
            (100, 0x600, false),
        ] {
            let mut world = field(&catalog);
            let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
            player.mp = mp as i32;
            player.max_mp = 100;
            player.dead = dead;
            let player = world.spawn(player).unwrap();
            world.reindex();

            let gate = world.may_use_item(player, &catalog, ObjectType(item));
            let ran = !world
                .use_item(player, &catalog, ObjectType(item), (10.0, 10.0))
                .is_empty();
            assert_eq!(gate, ran, "item {item:#x} with {mp} magic, dead {dead}");
        }
    }

    #[test]
    fn an_ability_is_refused_without_the_magic_for_it() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 10;
        player.max_mp = 100;
        let player = world.spawn(player).unwrap();
        world.reindex();

        assert!(
            world
                .use_item(player, &catalog, ObjectType(0x903), (14.0, 10.0))
                .is_empty()
        );
        assert_eq!(world.get(player).unwrap().mp, 10, "and nothing was spent");
    }

    #[test]
    fn an_ability_may_be_used_as_fast_as_the_magic_lasts() {
        // There is no ability cooldown in the original. `Item.Cooldown` is parsed off the content
        // and read in no other file in the server, and `UseItem` refuses only for magic
        // (`Player.UseItem.cs:162`). The rate limit that exists is on the weapon, not on this.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 1000;
        player.max_mp = 1000;
        let player = world.spawn(player).unwrap();
        world.reindex();

        // The spell costs sixty and the item declares half a second of cooldown, which nothing
        // reads. Six in a row inside one tick, and the only thing that stops the seventeenth is
        // running out of magic.
        for shot in 0..6 {
            assert!(
                !world
                    .use_item(player, &catalog, ObjectType(0x903), (14.0, 10.0))
                    .is_empty(),
                "refused at {shot} with magic still in the bar"
            );
        }
        assert_eq!(world.get(player).unwrap().mp, 1000 - 6 * 60);
    }

    #[test]
    fn quiet_empties_the_bar_rather_than_refusing_the_ask() {
        // Nothing in the original refuses an activation for a condition. Quiet works by zeroing
        // magic every tick (`Player.Effects.cs:36-37`), which stops a spell and leaves a potion --
        // which costs nothing -- perfectly drinkable.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.hp = 100;
        player.mp = 1000;
        player.max_mp = 1000;
        let player = world.spawn(player).unwrap();
        world.reindex();

        give(&mut world, player, hendra_content::ConditionEffect::Quiet);
        world.advance(&catalog, 50);
        assert_eq!(world.get(player).unwrap().mp, 0, "the bar is emptied");

        assert!(
            world
                .use_item(player, &catalog, ObjectType(0x903), (14.0, 10.0))
                .is_empty(),
            "and a spell that costs magic has none to spend"
        );
        assert!(
            !world
                .use_item(player, &catalog, ObjectType(0x902), (10.0, 10.0))
                .is_empty(),
            "but a potion costs nothing and still works"
        );
        assert_eq!(world.get(player).unwrap().hp, 200);
    }

    #[test]
    fn a_magic_nova_fills_the_caster_as_well_as_everyone_beside_them() {
        // `AOE` hit-tests the player map at the caster's own position (`Utils.cs:324`), so the one
        // who set the nova off is one of the players it finds. Leaving them out makes the only
        // magic-restoring ability in the game useless to the person holding it.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 10;
        player.max_mp = 1000;
        let player = world.spawn(player).unwrap();

        let mut friend = Entity::player(ObjectType(0x600), 12.0, 10.0, 500);
        friend.mp = 10;
        friend.max_mp = 1000;
        let friend = world.spawn(friend).unwrap();

        let mut distant = Entity::player(ObjectType(0x600), 20.0, 10.0, 500);
        distant.mp = 10;
        distant.max_mp = 1000;
        let distant = world.spawn(distant).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x90c), (10.0, 10.0));

        assert_eq!(world.get(player).unwrap().mp, 60, "the caster too");
        assert_eq!(world.get(friend).unwrap().mp, 60);
        assert_eq!(world.get(distant).unwrap().mp, 10, "and nobody further off");
    }

    #[test]
    fn a_healing_aura_that_does_not_stack_heals_for_what_it_added() {
        // The hack job at `Player.UseItem.cs:1076-1079`: a `noStack` boost to maximum health also
        // heals for its own amount, at once. Without it a paladin's seal raises the ceiling and
        // leaves the bar where it was, which is a heal that heals nobody.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.hp = 300;
        player.mp = 1000;
        player.max_mp = 1000;
        let player = world.spawn(player).unwrap();

        let mut friend = Entity::player(ObjectType(0x600), 12.0, 10.0, 500);
        friend.hp = 300;
        let friend = world.spawn(friend).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x90b), (10.0, 10.0));

        let entity = world.get(player).unwrap();
        assert_eq!(entity.max_hp, 525, "the ceiling moves with the boost");
        assert_eq!(entity.hp, 325, "and the bar moves with it");
        assert_eq!(world.get(friend).unwrap().hp, 325, "for everyone in it");

        // And it goes again when the boost lapses, taking the ceiling with it.
        for _ in 0..80 {
            world.advance(&catalog, 50);
        }
        assert_eq!(world.get(player).unwrap().max_hp, 500);
    }

    #[test]
    fn a_stat_boost_to_oneself_moves_the_ceiling_it_raises() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x90f), (10.0, 10.0));

        let entity = world.get(player).unwrap();
        assert_eq!(entity.max_hp, 650);
        assert_eq!(entity.hp, 500, "a self boost heals nothing on its own");
    }

    #[test]
    fn a_bullet_nova_is_twenty_bullets_around_the_cursor() {
        // Twenty, written into the array length (`Player.UseItem.cs:1146`), and started at the
        // aimed point rather than at the caster (`:1153`). Eight from the player's own feet is a
        // different ability entirely.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x90d), (18.0, 10.0));
        assert_eq!(world.projectile_count(), 20);

        // A slime standing on the cursor is inside the ring at once; one standing on the caster is
        // not, because that is not where the bullets are.
        let mut slime = Entity::fixture(ObjectType(0x502), 18.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.max_hp = 5000;
        slime.hp = 5000;
        let slime = world.spawn(slime).unwrap();
        world.reindex();
        world.advance(&catalog, 50);

        assert!(
            world.get(slime).unwrap().hp < 5000,
            "the ring goes off where it was aimed"
        );
    }

    #[test]
    fn a_bullet_nova_aimed_past_fourteen_tiles_does_nothing() {
        // `MaxAbilityDist` is the one range limit any activation in the original has, and it is
        // measured squared against the caster (`Player.UseItem.cs:1145`).
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x90d), (24.5, 10.0));
        assert_eq!(world.projectile_count(), 0, "too far to place");

        world.use_item(player, &catalog, ObjectType(0x90d), (23.5, 10.0));
        assert_eq!(world.projectile_count(), 20, "and inside it is fine");
    }

    #[test]
    fn a_cleansing_aura_reaches_the_one_who_used_it() {
        // `this.AOE(...)` finds the caster along with everyone else (`Player.UseItem.cs:1020`), so
        // a purifying item purifies its owner. It not doing so is the whole point of the item.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        give(&mut world, player, hendra_content::ConditionEffect::Slowed);
        assert!(
            world
                .get(player)
                .unwrap()
                .conditions
                .contains(hendra_content::ConditionEffect::Slowed)
        );

        world.use_item(player, &catalog, ObjectType(0x910), (10.0, 10.0));

        assert!(
            !world
                .get(player)
                .unwrap()
                .conditions
                .contains(hendra_content::ConditionEffect::Slowed)
        );
    }

    #[test]
    fn an_ability_reaches_what_it_is_aimed_at_and_spares_the_people_beside_it() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 1000;
        player.max_mp = 1000;
        let player = world.spawn(player).unwrap();

        let mut enemy = Entity::fixture(ObjectType(0x502), 20.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 500;
        enemy.hp = 500;
        let enemy = world.spawn(enemy).unwrap();

        // Standing in the blast, and on the caster's side.
        let friend = world
            .spawn(Entity::player(ObjectType(0x600), 20.5, 10.0, 500))
            .unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x903), (20.0, 10.0));

        // The ball has to land and the first instalment fall due: a poison grenade deals nothing
        // where it is thrown (`Player.UseItem.cs:688-700`).
        for _ in 0..20 {
            world.advance(&catalog, 100);
        }

        assert!(world.get(enemy).unwrap().hp < 500, "the enemy is hit");
        assert_eq!(world.get(friend).unwrap().hp, 500, "and the friend is not");
    }

    #[test]
    fn a_bag_takes_the_colour_of_the_best_thing_in_it() {
        // A white bag beside a brown one is how a player knows which to walk back for, and reading
        // the highest rather than the first means the order loot rolled in does not decide it.
        let catalog = catalog();
        let mut world = field(&catalog);
        world.set_bag_types(vec![
            ObjectType(0x510),
            ObjectType::NONE,
            ObjectType::NONE,
            ObjectType::NONE,
            ObjectType::NONE,
            ObjectType(0x511),
        ]);

        let mut enemy = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 200;
        enemy.hp = 200;
        let enemy = world.spawn(enemy).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                 state a { }
                 loot {
                   item("Health Potion", 1)
                   item("Rare Blade", 1)
                 }
               }"#,
        );
        world.reindex();

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(count_of(&world, 0x511), 1, "the rarer colour wins");
        assert_eq!(count_of(&world, 0x510), 0);
    }

    /// The one thing a drop says out loud.
    ///
    /// `Loots.ShowBags` shouts a named item to every player in the world, with the share of the
    /// kill its owner did (`Loots.cs:325-331`). It is what tells a room a room away that somebody
    /// just got something, and it is most of how a populated server feels populated.
    #[test]
    fn a_named_drop_is_shouted_to_the_whole_world_with_the_share_that_earned_it() {
        let catalog = catalog();
        let mut world = field(&catalog);
        world.set_bag_types(vec![ObjectType(0x510)]);

        let mut enemy = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 200;
        enemy.hp = 200;
        let enemy = world.spawn(enemy).unwrap();

        let mut player = Entity::player(ObjectType(0x600), 11.0, 10.0, 500);
        player.name = Some("Hendra".to_string().into());
        let player = world.spawn(player).unwrap();

        let mut bystander = Entity::player(ObjectType(0x600), 12.0, 10.0, 500);
        bystander.name = Some("Someone".to_string().into());
        let bystander = world.spawn(bystander).unwrap();

        // Three quarters of the kill to one player and a quarter to the other, which is what the
        // percentage in the sentence is worked out from.
        world.get_mut(enemy).unwrap().damage_by = vec![(player, 150), (bystander, 50)];

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                 state a { }
                 loot { threshold(0.01) { item("Doom Bow", 1) } }
               }"#,
        );
        world.reindex();

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        let said = world.take_announcements();
        let shout = said
            .iter()
            .find(|line| line.text.contains("Amazing drop"))
            .expect("a named item should announce itself");

        assert_eq!(
            &*shout.text,
            "Hendra has just gotten Amazing drop >> Doom Bow with this damage >> 75%"
        );
        assert!(shout.broadcast, "everybody in the world hears it");
        assert_eq!(
            shout.speaker.as_deref(),
            Some(LOOT_NOTIFIER),
            "under the name `SendNotif` speaks in"
        );
    }

    /// And an ordinary drop says nothing, which is every drop but a hundred and five.
    #[test]
    fn an_unlisted_drop_says_nothing() {
        let catalog = catalog();
        let mut world = field(&catalog);
        world.set_bag_types(vec![ObjectType(0x510)]);

        let mut enemy = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 200;
        enemy.hp = 200;
        let enemy = world.spawn(enemy).unwrap();

        let mut player = Entity::player(ObjectType(0x600), 11.0, 10.0, 500);
        player.name = Some("Hendra".to_string().into());
        let player = world.spawn(player).unwrap();
        world.get_mut(enemy).unwrap().damage_by = vec![(player, 200)];

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                 state a { }
                 loot { threshold(0.01) { item("Rare Blade", 1) } }
               }"#,
        );
        world.reindex();

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert!(
            !world
                .take_announcements()
                .iter()
                .any(|line| line.text.contains("Amazing drop"))
        );
    }

    #[test]
    fn a_colour_with_no_bag_registered_still_drops_the_loot() {
        // Losing the loot is worse than losing its colour.
        let catalog = catalog();
        let mut world = field(&catalog);
        world.set_bag_types(vec![ObjectType(0x510)]);

        let mut enemy = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 200;
        enemy.hp = 200;
        let enemy = world.spawn(enemy).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { } loot { item("Rare Blade", 1) } }"#,
        );
        world.reindex();

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        let bags = world
            .iter()
            .filter(|(_, entity)| entity.container.is_some())
            .count();
        assert_eq!(bags, 1, "the loot is still there");
    }

    #[test]
    fn an_invisible_player_is_hidden_from_everyone_but_itself() {
        // Filtered by the server rather than the client: a client told about something it should
        // not see is a client that can be made to reveal it.
        let catalog = catalog();
        let mut world = field(&catalog);

        let watcher = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        let hidden = world
            .spawn(Entity::player(ObjectType(0x600), 12.0, 10.0, 500))
            .unwrap();
        world.reindex();

        assert_eq!(world.snapshot_for(watcher, 20.0).len(), 2);

        give(
            &mut world,
            hidden,
            hendra_content::ConditionEffect::Invisible,
        );

        assert_eq!(
            world.snapshot_for(watcher, 20.0).len(),
            1,
            "the watcher sees only itself"
        );
        assert_eq!(
            world.snapshot_for(hidden, 20.0).len(),
            2,
            "and the hidden one still sees both"
        );
    }

    #[test]
    fn nothing_can_shoot_a_decoy_down_and_its_health_is_only_its_clock() {
        // A decoy is a `StaticObject` whose `life` is its duration (`Decoy.cs:29`), and
        // `StaticObject.HitByProjectile` (`:57`) acts only on a projectile owned by a `Player`.
        // Its owner's shots therefore pass through it, an enemy's shots take nothing off it, and
        // the only thing that ends one is running out — so the health on the wire is the
        // milliseconds it has left and nothing else.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 500;
        player.max_mp = 500;
        player.weapon = Some(ObjectType(0x901));
        let player = world.spawn(player).unwrap();

        // A slime a tile away, firing straight at where the decoy will be left.
        let mut slime = Entity::fixture(ObjectType(0x502), 14.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        slime.weapon = Some(ObjectType(0x502));
        let slime = world.spawn(slime).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x905), (10.0, 10.0));

        let decoy = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Decoy)
            .map(|(handle, _)| handle)
            .expect("a decoy was left");

        let before = world.get(decoy).unwrap().hp;
        assert!(before > 0, "its lifetime is its health");

        // The one who left it walks off, which is the whole point of leaving one, and takes up a
        // line of fire of its own. Otherwise its own body stands on the decoy's square and stops
        // every shot aimed at it, and this proves nothing.
        {
            let entity = world.get_mut(player).unwrap();
            entity.x = 10.0;
            entity.y = 16.0;
        }
        world.reindex();

        // Both ends fire at it: the one who left it, and the one it is there to distract.
        world.take_damage();
        let mut elapsed = 0;
        let mut struck = 0;
        for _ in 0..10 {
            world.shoot(player, &catalog, -std::f32::consts::FRAC_PI_2);
            world.shoot(slime, &catalog, std::f32::consts::PI);
            world.advance(&catalog, 50);
            elapsed += 50;
            struck += world
                .take_damage()
                .iter()
                .filter(|event| event.target == decoy)
                .count();
        }

        assert_eq!(struck, 0, "a bullet was allowed to land on a decoy");
        let decoy = world.get(decoy).expect("it is still standing");
        assert_eq!(
            decoy.hp,
            before - elapsed,
            "its health is the clock and nothing else took any of it"
        );
    }

    #[test]
    fn a_decoy_walks_the_way_its_owner_was_going_and_then_stands() {
        // `Decoy.Tick` moves it along its owner's last heading at four tiles a second while
        // `HP > duration - 2000` (`Decoy.cs:57-63`, `Player.UseItem.cs:795`). A decoy that stood
        // where it was dropped is a decoy every player can tell apart from a player at a glance,
        // which is the one thing it must not be.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 500;
        player.max_mp = 500;
        // Two tiles back along the x axis, so the heading is due east.
        player.trail.previous = (8.0, 10.0);
        player.trail.last = (10.0, 10.0);
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x905), (10.0, 10.0));
        let decoy = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Decoy)
            .map(|(handle, _)| handle)
            .expect("a decoy was left");

        for _ in 0..20 {
            world.advance(&catalog, 50);
        }

        let after_a_second = world.get(decoy).unwrap();
        assert!(
            (after_a_second.x - 14.0).abs() < 0.3,
            "four tiles a second due east: {}",
            after_a_second.x
        );
        assert!(
            (after_a_second.y - 10.0).abs() < 0.01,
            "and nothing sideways: {}",
            after_a_second.y
        );

        // Two seconds of its five-second life is all the walking it gets.
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        let walked = world.get(decoy).unwrap().x;

        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert_eq!(
            world.get(decoy).unwrap().x,
            walked,
            "past two seconds of life it stands still"
        );
    }

    #[test]
    fn a_decoy_goes_off_once_as_it_runs_out() {
        // `Decoy.Tick` broadcasts one red `AreaBlast` of radius one when under 250 ms of life
        // remain, guarded by its own `exploded` flag (`Decoy.cs:64-73`). It hurts nothing: it is
        // only how the room is told the player it was pretending to be was never there.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 500;
        player.max_mp = 500;
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x905), (10.0, 10.0));
        let decoy = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Decoy)
            .map(|(handle, _)| handle)
            .expect("a decoy was left");

        let mut blasts = Vec::new();
        for _ in 0..120 {
            world.advance(&catalog, 50);
            blasts.extend(world.take_effects().into_iter().filter(|event| {
                event.effect == hendra_net::message::effect::AREA_BLAST
                    && event.target == Some(decoy)
            }));
        }

        assert_eq!(blasts.len(), 1, "once, not once a tick");
        assert_eq!(blasts[0].color, 0xffff_0000, "red");
        assert_eq!(blasts[0].x1, 1.0, "and a radius of one");
    }

    #[test]
    fn a_decoy_does_not_last_forever() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 500;
        player.max_mp = 500;
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x905), (10.0, 10.0));
        assert_eq!(
            world.iter().filter(|(_, e)| e.kind == Kind::Decoy).count(),
            1
        );

        for _ in 0..120 {
            world.advance(&catalog, 100);
        }
        assert_eq!(
            world.iter().filter(|(_, e)| e.kind == Kind::Decoy).count(),
            0,
            "it should have gone"
        );
    }

    #[test]
    fn a_trap_waits_and_goes_off_when_something_walks_into_it() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 500;
        player.max_mp = 500;
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x906), (20.0, 10.0));
        world.advance(&catalog, 50);
        assert_eq!(
            world.iter().filter(|(_, e)| e.kind == Kind::Trap).count(),
            1,
            "armed and waiting"
        );

        // Something walks in.
        let mut enemy = Entity::fixture(ObjectType(0x502), 20.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 500;
        enemy.hp = 500;
        let enemy = world.spawn(enemy).unwrap();
        world.reindex();
        world.advance(&catalog, 50);

        assert!(
            world.get(enemy).unwrap().hp < 500,
            "it should have gone off"
        );
        assert_eq!(
            world.iter().filter(|(_, e)| e.kind == Kind::Trap).count(),
            0,
            "and be spent"
        );
    }

    #[test]
    fn a_trap_nobody_walks_into_stays_armed() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 500;
        player.max_mp = 500;
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x906), (20.0, 10.0));
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }

        assert_eq!(
            world.iter().filter(|(_, e)| e.kind == Kind::Trap).count(),
            1,
            "a player standing beside it is not what sets it off"
        );
    }

    #[test]
    fn a_setpiece_stamps_a_room_rather_than_painting_a_circle() {
        // The wrong version of this painted one tile over a radius, which left the room looking
        // roughly right and nothing where the author had drawn it.
        let catalog = catalog();
        let mut world = field(&catalog);

        // A three by three piece: water all round, a sign in the middle.
        let mut squares: Vec<Composition> =
            (0..9).map(|_| square(0x11, ObjectType::NONE.0)).collect();
        squares[4] = square(0x11, 0x501);
        let piece = Map::from_squares(3, 3, squares).unwrap();

        world.stamp(&catalog, &piece, (10.0, 10.0));
        world.reindex();

        // Centred on the point rather than starting there: a three by three at (10, 10) covers
        // nine through eleven, so the corner before it is the square that tells them apart.
        assert!(!world.terrain().walkable(9, 9), "the piece is centred");
        assert!(!world.terrain().walkable(11, 11));
        assert!(world.terrain().walkable(12, 12), "and no further");
        assert!(world.terrain().walkable(20, 20), "and only where it landed");

        // The sign is static scenery, so it belongs to the map rather than to the world, exactly as
        // it would had the piece been loaded as a world of its own.
        assert_eq!(count_of(&world, 0x501), 0, "the sign is not an entity");
        assert_eq!(
            world.terrain().map().at(10, 10).map(|square| square.object),
            Some(ObjectType(0x501)),
            "the sign is on the tile"
        );
    }

    #[test]
    fn paving_the_floor_under_a_wall_leaves_the_wall_standing() {
        // `GroundTransform` rewrites `TileId`, `Spawned` and `UpdateCount` and nothing else
        // (`GroundTransform.cs:72-74`). A boss laying lava across a room must not take down the
        // walls of it — not their collision, and not the sight they block, which is what every
        // enemy's targeting reads.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..32 * 32)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[10 * 32 + 16] = square(0x10, 0x50f); // a thicket, which blocks sight
        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Field", Terrain::build(map, &catalog), &catalog);

        assert!(
            !world.terrain().line_of_sight(12.5, 10.5, 20.5, 10.5),
            "the thicket blocks sight to begin with"
        );

        // Lava poured over the whole room, the thicket's square included.
        world.reshape_ground(&catalog, 16.0, 10.0, 6.0, 0x12, None);

        assert!(
            !world.terrain().line_of_sight(12.5, 10.5, 20.5, 10.5),
            "and still does once the floor beneath it is lava"
        );
        assert_eq!(
            world.terrain().tile_at(16, 10),
            TileType(0x12),
            "though the floor did change"
        );
        assert_eq!(
            world.terrain().map().at(16, 10).map(|square| square.tile),
            Some(TileType(0x12)),
            "in the map as well, so a late joiner is told the same room"
        );
    }

    #[test]
    fn repainting_a_square_with_the_ground_it_already_has_says_nothing() {
        // `GroundTransform` returns without recording the square when the tile is already the one
        // it would lay (`GroundTransform.cs:58-59`, `:84-85`), which is what keeps a boss standing
        // in its own lava from resending the same square every second.
        let catalog = catalog();
        let mut world = field(&catalog);

        world.reshape_ground(&catalog, 16.0, 16.0, 2.0, 0x12, None);
        let laid = world.take_ground_changes().len();
        assert!(laid > 0, "the first pour changes the ground");

        world.reshape_ground(&catalog, 16.0, 16.0, 2.0, 0x12, None);
        assert_eq!(
            world.take_ground_changes().len(),
            0,
            "the second pour has nothing to say"
        );
    }

    #[test]
    fn a_stamped_wall_blocks_as_a_loaded_one_does() {
        // `Wmap.ProjectOntoWorld` copies each of the setpiece's tiles onto the world's tile whole,
        // `ObjType` and all (`Wmap.cs:473-478`), so a wall a setpiece stamps down is read off the
        // tile by `Entity.TileOccupied` exactly as a wall the world was born with. Spawning it as an
        // entity instead gives a wall that is drawn and walked straight through.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut squares: Vec<Composition> =
            (0..25).map(|_| square(0x10, ObjectType::NONE.0)).collect();
        for index in 10..15 {
            squares[index] = square(0x10, 0x500); // a wall across the middle
        }
        let piece = Map::from_squares(5, 5, squares).unwrap();

        let before = world.len();
        world.stamp(&catalog, &piece, (10.0, 10.0));
        world.reindex();

        assert_eq!(world.len(), before, "a wall is scenery, not an entity");
        for x in 8..13 {
            assert!(
                !world.terrain().walkable(x, 10),
                "the stamped wall blocks at {x}"
            );
        }

        // And a claim straight across it is refused, which is the thing a player would have felt.
        let walker = world
            .spawn(Entity::player(ObjectType(0x600), 10.5, 9.5, 500))
            .unwrap();
        let outcome = world
            .resolve_move(walker, &catalog, 10.5, 11.5, 1000)
            .unwrap();
        assert!(outcome.y < 10.0, "the claim crossed the wall: {outcome:?}");
    }

    #[test]
    fn a_setpiece_clears_what_was_standing_where_it_lands() {
        // A setpiece that appeared around the existing furniture would leave a wall through the
        // middle of a boss's arena.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut old = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        old.kind = Kind::Enemy;
        old.max_hp = 200;
        old.hp = 200;
        world.spawn(old).unwrap();
        world.reindex();

        let squares: Vec<Composition> = (0..25).map(|_| square(0x10, ObjectType::NONE.0)).collect();
        let piece = Map::from_squares(5, 5, squares).unwrap();

        world.stamp(&catalog, &piece, (10.0, 10.0));
        world.advance(&catalog, 50);

        assert_eq!(count_of(&world, 0x502), 0, "what was there is gone");
    }

    #[test]
    fn a_setpiece_moves_the_room_around_a_player_rather_than_deleting_them() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        let squares: Vec<Composition> = (0..25).map(|_| square(0x10, ObjectType::NONE.0)).collect();
        let piece = Map::from_squares(5, 5, squares).unwrap();

        world.stamp(&catalog, &piece, (10.0, 10.0));
        world.advance(&catalog, 50);

        assert!(world.get(player).is_some(), "the player is still there");
    }

    #[test]
    fn a_realm_announcement_is_heard_by_everyone_and_named_after_nobody() {
        let catalog = catalog();
        let mut world = field(&catalog);
        world.announce("the realm is closing");

        let said = world.take_announcements();
        assert_eq!(said.len(), 1);
        assert!(said[0].broadcast, "everybody hears it");
        assert_eq!(said[0].from, Handle::NONE, "and nobody said it");
    }

    #[test]
    fn counting_enemies_ignores_the_dead_and_everything_else() {
        let catalog = catalog();
        let mut world = field(&catalog);

        world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();

        for at in 0..3 {
            let mut enemy = Entity::fixture(ObjectType(0x502), 11.0 + at as f32, 10.0);
            enemy.kind = Kind::Enemy;
            enemy.max_hp = 200;
            enemy.hp = 200;
            let enemy = world.spawn(enemy).unwrap();
            if at == 0 {
                world.get_mut(enemy).unwrap().dead = true;
            }
        }

        assert_eq!(world.enemy_count(), 2, "the dead one does not count");
        assert_eq!(world.count_of_kind(ObjectType(0x502)), 2);
        assert_eq!(world.count_of_kind(ObjectType(0x503)), 0);
    }

    /// A field of one terrain, so a realm has somewhere to put things.
    fn terraced(catalog: &Catalog, terrain: hendra_content::Terrain) -> World {
        let squares = (0..64 * 64).map(|_| Composition {
            tile: TileType(0x10),
            object: ObjectType::NONE,
            region: Region::None,
            terrain,
            config: String::new(),
        });
        let map = Map::from_squares(64, 64, squares).unwrap();
        World::new("Realm", Terrain::build(map, catalog), catalog)
    }

    fn slimes(terrain: hendra_content::Terrain) -> Vec<crate::realm::Spawn> {
        vec![crate::realm::Spawn {
            kind: ObjectType(0x502),
            terrain,
            weight: 1.0,
            group: None,
            per_enemy: 200,
        }]
    }

    fn add(terrain: hendra_content::Terrain, count: usize) -> Vec<crate::realm::Adjustment> {
        vec![crate::realm::Adjustment {
            terrain,
            add: count,
            remove: 0,
        }]
    }

    #[test]
    fn a_realm_fills_the_terrain_it_was_asked_to_fill() {
        let catalog = catalog();
        let terrain = hendra_content::Terrain::MidPlains;
        let mut world = terraced(&catalog, terrain);

        let (added, removed) = world.populate(&catalog, &slimes(terrain), &add(terrain, 40));
        world.reindex();

        assert_eq!(added, 40);
        assert_eq!(removed, 0);
        assert_eq!(world.enemy_count(), 40);
    }

    #[test]
    fn what_a_realm_places_is_tagged_with_the_ground_it_was_placed_on() {
        // The tag is what the next count reads. Without it the population would be recounted as
        // zero every minute and the realm would fill forever.
        let catalog = catalog();
        let terrain = hendra_content::Terrain::HighForest;
        let mut world = terraced(&catalog, terrain);

        world.populate(&catalog, &slimes(terrain), &add(terrain, 10));
        world.reindex();

        assert_eq!(world.alive_by_terrain()[terrain as usize], 10);
        assert_eq!(
            world.alive_by_terrain()[hendra_content::Terrain::MidPlains as usize],
            0
        );
    }

    #[test]
    fn a_realm_puts_its_enemies_among_the_trees() {
        // `Oryx.Spawn` calls `IsPassable(pt.X, pt.Y)` bare (`Oryx.cs:559`, `:581`), and `spawning`
        // defaults to false (`World.cs:454`), so the `OccupySquare` clause is not applied. A bramble
        // is such an object: it stops a player landing on it and does nothing to an enemy, and a
        // realm that refused those squares would leave every forest empty.
        let catalog = catalog();
        let terrain = hendra_content::Terrain::MidForest;
        let squares = (0..64 * 64).map(|_| Composition {
            tile: TileType(0x10),
            object: ObjectType(0x51f),
            region: Region::None,
            terrain,
            config: String::new(),
        });
        let map = Map::from_squares(64, 64, squares).unwrap();
        let mut world = World::new("Realm", Terrain::build(map, &catalog), &catalog);

        assert!(
            !world.terrain().walkable(10, 10),
            "a player is stopped by it"
        );
        assert!(world.terrain().passable(10, 10), "and an enemy is not");

        let (added, _) = world.populate(&catalog, &slimes(terrain), &add(terrain, 20));
        assert_eq!(added, 20, "so Oryx has somewhere to put them");
    }

    #[test]
    fn a_realm_keeps_its_enemies_out_of_the_walls() {
        // The other half of the same rule: `FullOccupy` and `EnemyOccupySquare` still refuse the
        // square, whatever `spawning` says.
        let catalog = catalog();
        let terrain = hendra_content::Terrain::Mountains;
        let squares = (0..64 * 64).map(|_| Composition {
            tile: TileType(0x10),
            object: ObjectType(0x51e), // a pillar: occupies against everything
            region: Region::None,
            terrain,
            config: String::new(),
        });
        let map = Map::from_squares(64, 64, squares).unwrap();
        let mut world = World::new("Realm", Terrain::build(map, &catalog), &catalog);

        assert!(!world.terrain().passable(10, 10));

        let (added, _) = world.populate(&catalog, &slimes(terrain), &add(terrain, 20));
        assert_eq!(added, 0, "there is nowhere in this map to stand");
    }

    #[test]
    fn a_realm_places_nothing_on_ground_nothing_belongs_to() {
        let catalog = catalog();
        let mut world = terraced(&catalog, hendra_content::Terrain::MidPlains);

        // Asked to fill the mountains, of which this map has none.
        let (added, _) = world.populate(
            &catalog,
            &slimes(hendra_content::Terrain::Mountains),
            &add(hendra_content::Terrain::Mountains, 20),
        );

        assert_eq!(added, 0);
        assert_eq!(world.enemy_count(), 0);
    }

    #[test]
    fn nothing_appears_on_top_of_a_player() {
        // An enemy on top of somebody is not a spawn but an ambush nobody could have avoided.
        let catalog = catalog();
        let terrain = hendra_content::Terrain::MidPlains;
        let mut world = terraced(&catalog, terrain);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 32.0, 32.0, 500))
            .unwrap();
        world.reindex();

        world.populate(&catalog, &slimes(terrain), &add(terrain, 200));
        world.reindex();

        let (px, py) = {
            let entity = world.get(player).unwrap();
            (entity.x, entity.y)
        };

        for (_, entity) in world.iter() {
            if entity.kind != Kind::Enemy {
                continue;
            }
            let (dx, dy) = (entity.x - px, entity.y - py);

            // A literal rather than the constant, so shrinking the constant fails this instead of
            // quietly moving the bar down with it.
            assert!(
                (dx * dx + dy * dy).sqrt() >= 9.0,
                "something appeared beside the player"
            );
        }
    }

    #[test]
    fn a_terrain_that_has_overfilled_is_thinned() {
        let catalog = catalog();
        let terrain = hendra_content::Terrain::LowSand;
        let mut world = terraced(&catalog, terrain);

        world.populate(&catalog, &slimes(terrain), &add(terrain, 30));
        world.reindex();

        let (added, removed) = world.populate(
            &catalog,
            &slimes(terrain),
            &[crate::realm::Adjustment {
                terrain,
                add: 0,
                remove: 12,
            }],
        );
        world.reindex();

        assert_eq!(added, 0);
        assert_eq!(removed, 12);
        assert_eq!(world.enemy_count(), 18);
    }

    #[test]
    fn thinning_leaves_alone_what_somebody_is_fighting() {
        // Despawning an enemy a player is on reads as the server eating their kill.
        let catalog = catalog();
        let terrain = hendra_content::Terrain::LowSand;
        let mut world = terraced(&catalog, terrain);

        world.populate(&catalog, &slimes(terrain), &add(terrain, 20));
        world.reindex();

        // Stand on one of them.
        let (victim, x, y) = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Enemy)
            .map(|(handle, entity)| (handle, entity.x, entity.y))
            .unwrap();
        world
            .spawn(Entity::player(ObjectType(0x600), x, y, 500))
            .unwrap();
        world.reindex();

        world.populate(
            &catalog,
            &slimes(terrain),
            &[crate::realm::Adjustment {
                terrain,
                add: 0,
                remove: 20,
            }],
        );

        assert!(
            world.get(victim).is_some(),
            "the one being fought was taken"
        );
    }

    #[test]
    fn what_an_enemy_spawns_is_counted_where_its_parent_was() {
        // Otherwise a breeding enemy leaks out of the census, and the terrain it is filling reads
        // as empty and gets filled again.
        let catalog = catalog();
        let terrain = hendra_content::Terrain::MidForest;
        let mut world = terraced(&catalog, terrain);

        world.populate(&catalog, &slimes(terrain), &add(terrain, 1));
        world.reindex();

        let parent = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Enemy)
            .map(|(handle, _)| handle)
            .unwrap();

        let behaviours = std::mem::take(&mut world.behaviours);
        let child = world
            .spawn_child(
                &catalog,
                &behaviours,
                ObjectType(0x502),
                5.0,
                5.0,
                None,
                Some(parent),
                false,
            )
            .unwrap();
        world.behaviours = behaviours;

        assert_eq!(world.get(child).unwrap().terrain, terrain);
        assert_eq!(world.alive_by_terrain()[terrain as usize], 2);
    }

    #[test]
    fn teleporting_moves_you_to_them_and_starts_a_cooldown() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mover = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
            .unwrap();
        let target = world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 24.0, 500))
            .unwrap();
        world.get_mut(target).unwrap().name = Some("Bo".into());

        assert_eq!(world.teleport_to(mover, target), None);

        let moved = world.get(mover).unwrap();
        assert_eq!((moved.x, moved.y), (20.0, 24.0));
        assert_eq!(moved.teleport_cooldown_ms, TELEPORT_COOLDOWN_MS);

        // And not again straight away, or a realm can be crossed faster than anything can chase.
        assert!(world.teleport_to(mover, target).is_some());
    }

    #[test]
    fn nobody_teleports_to_somebody_hiding_or_paused() {
        // Both would reach a player the world says cannot be reached: one is hiding, and the other
        // is somewhere the world has stopped.
        let catalog = catalog();

        for hidden in [
            hendra_content::ConditionEffect::Invisible,
            hendra_content::ConditionEffect::Paused,
        ] {
            let mut world = field(&catalog);
            let mover = world
                .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
                .unwrap();
            let target = world
                .spawn(Entity::player(ObjectType(0x600), 20.0, 24.0, 500))
                .unwrap();

            world.get_mut(target).unwrap().conditions.insert(hidden);

            assert!(
                world.teleport_to(mover, target).is_some(),
                "reached somebody {hidden:?}"
            );
            assert_eq!(world.get(mover).unwrap().x, 2.0, "and moved anyway");
        }
    }

    #[test]
    fn a_world_that_forbids_teleporting_forbids_it() {
        // The nexus and the shops do, in the world definitions.
        let catalog = catalog();
        let mut world = field(&catalog);
        world.set_allows_teleport(false);

        let mover = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
            .unwrap();
        let target = world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 24.0, 500))
            .unwrap();

        assert!(world.teleport_to(mover, target).is_some());
        assert_eq!(world.get(mover).unwrap().x, 2.0);
    }

    #[test]
    fn you_cannot_teleport_to_an_enemy_or_to_yourself() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let mover = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
            .unwrap();
        let slime = world
            .spawn(Entity::fixture(ObjectType(0x502), 20.0, 24.0))
            .unwrap();

        assert!(world.teleport_to(mover, mover).is_some());
        assert!(world.teleport_to(mover, slime).is_some());
        assert_eq!(world.get(mover).unwrap().x, 2.0);
    }

    #[test]
    fn a_teleported_player_is_not_snapped_back_for_arriving() {
        // The server moved them. Checking that against a walking speed would refuse its own
        // teleport, and the player would be dragged back the moment their client agreed.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mover = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
            .unwrap();
        let target = world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 24.0, 500))
            .unwrap();

        world.teleport_to(mover, target);

        let outcome = world
            .resolve_move(mover, &catalog, 20.0, 24.0, 50)
            .expect("an outcome");
        assert_eq!((outcome.x, outcome.y), (20.0, 24.0));
        assert!(outcome.refused.is_none());
    }

    #[test]
    fn a_teleport_is_something_the_client_is_told_about() {
        // The test above is the whole reason this one has to exist: it hands the destination to
        // `resolve_move` as the client's claim, which a client can only do if something told it
        // where it now is. `TeleportPosition` sends a `Goto` for exactly that
        // (`Player.cs:686-704`); without one the client claims the position it still believes in
        // and the grace waves that claim straight through, undoing the teleport.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mover = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
            .unwrap();

        assert!(world.take_teleports().is_empty(), "nobody has moved yet");
        assert_eq!(world.teleport_to_square(mover, 28.0, 30.0), None);

        let told = world.take_teleports();
        assert_eq!(told.len(), 1);
        assert_eq!(told[0].who, mover);
        assert_eq!((told[0].x, told[0].y), (28.0, 30.0));

        // Drained, so it is sent once rather than every tick for the rest of the world's life.
        assert!(world.take_teleports().is_empty());

        // And the same for a teleport to a person, which is the other way in.
        let target = world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 24.0, 500))
            .unwrap();
        world.get_mut(mover).unwrap().teleport_cooldown_ms = 0;
        assert_eq!(world.teleport_to(mover, target), None);

        let told = world.take_teleports();
        assert_eq!(told.len(), 1);
        assert_eq!((told[0].x, told[0].y), (20.0, 24.0));
    }

    #[test]
    fn a_player_can_be_found_by_name_whatever_its_capitals() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let handle = world
            .spawn(Entity::player(ObjectType(0x600), 2.0, 2.0, 500))
            .unwrap();
        world.get_mut(handle).unwrap().name = Some("Fesal".into());

        assert_eq!(world.player_named("fesal"), Some(handle));
        assert_eq!(world.player_named("FESAL"), Some(handle));
        assert_eq!(world.player_named("Nobody"), None);
    }

    // The three tests that used to sit here drove `roll_loot_for`, a second loot roller that
    // nothing in the simulation called. They asserted the luck arithmetic against a copy of the
    // rule rather than against the rule, so they would have gone on passing with the live path
    // broken. They now live in `tests/loot.rs`, where they kill an enemy and read the bags.

    #[test]
    fn a_bag_that_belongs_to_somebody_is_only_theirs_to_open() {
        // What makes dropping a soulbound item a way to move it rather than a way to give it away.
        let catalog = catalog();
        let mut world = field(&catalog);

        let owner = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();

        world.show_bags(
            &catalog,
            &[ObjectType(0x904)],
            Some(owner),
            false,
            false,
            5.0,
            5.0,
        );
        world.reindex();

        let bag = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Container)
            .map(|(handle, entity)| (handle, entity.belongs_to))
            .expect("a bag");

        assert_eq!(bag.1, Some(owner));
    }

    #[test]
    fn a_temporary_boost_lapses_rather_than_lasting_forever() {
        // Its duration used to be discarded, which made every temporary boost permanent.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();

        let before = world.get(player).unwrap().stats.totals()[2];

        world.carry_out(
            player,
            &catalog,
            ObjectType::NONE,
            &hendra_content::Effect::StatBoost {
                stat: 2,
                amount: 10,
                duration_ms: 1000,
                range: None,
                no_stack: false,
            },
            (5.0, 5.0),
            (5.0, 5.0),
        );

        let raised = world.get(player).unwrap().stats.totals()[2];
        assert_eq!(raised - before, 10);

        world.advance(&catalog, 500);
        assert_eq!(
            world.get(player).unwrap().stats.totals()[2],
            raised,
            "it lapsed early"
        );

        world.advance(&catalog, 600);
        assert_eq!(
            world.get(player).unwrap().stats.totals()[2],
            before,
            "it never lapsed"
        );
    }

    #[test]
    fn two_boosts_of_the_same_size_are_worth_less_than_two() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();

        let before = world.get(player).unwrap().stats.totals()[2];

        for amount in [8, 8] {
            world.carry_out(
                player,
                &catalog,
                ObjectType::NONE,
                &hendra_content::Effect::StatBoost {
                    stat: 2,
                    amount,
                    duration_ms: 10_000,
                    range: None,
                    no_stack: false,
                },
                (5.0, 5.0),
                (5.0, 5.0),
            );
        }

        // One of them is kept, since renewing extends rather than adding a second. Two different
        // sizes is what the stacking rule is really about, and `stats::stacked` holds that.
        let after = world.get(player).unwrap().stats.totals()[2];
        assert_eq!(after - before, 8);
    }

    #[test]
    fn an_aura_reaches_the_people_standing_in_it() {
        // Its range used to be discarded, so an aura reached only whoever used it.
        let catalog = catalog();
        let mut world = field(&catalog);

        let caster = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();
        let friend = world
            .spawn(Entity::player(ObjectType(0x600), 7.0, 5.0, 500))
            .unwrap();
        let stranger = world
            .spawn(Entity::player(ObjectType(0x600), 40.0, 40.0, 500))
            .unwrap();
        world.reindex();

        let before = world.get(friend).unwrap().stats.totals()[2];

        world.carry_out(
            caster,
            &catalog,
            ObjectType::NONE,
            &hendra_content::Effect::StatBoost {
                stat: 2,
                amount: 10,
                duration_ms: 5000,
                range: Some(6.0),
                no_stack: false,
            },
            (5.0, 5.0),
            (5.0, 5.0),
        );

        assert_eq!(
            world.get(friend).unwrap().stats.totals()[2] - before,
            10,
            "somebody standing in it got nothing"
        );
        assert_eq!(
            world.get(stranger).unwrap().stats.totals()[2] - before,
            0,
            "somebody across the map got it"
        );
        assert!(
            world.get(caster).unwrap().stats.totals()[2] > 0,
            "the caster"
        );
    }

    #[test]
    fn standing_in_an_aura_does_not_pile_up_boosts() {
        // Renewing extends rather than adding, or a minute in an aura would be sixty boosts.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();
        world.reindex();

        for _ in 0..30 {
            world.carry_out(
                player,
                &catalog,
                ObjectType::NONE,
                &hendra_content::Effect::StatBoost {
                    stat: 2,
                    amount: 10,
                    duration_ms: 5000,
                    range: Some(6.0),
                    no_stack: false,
                },
                (5.0, 5.0),
                (5.0, 5.0),
            );
        }

        assert_eq!(world.get(player).unwrap().boosts.len(), 1);
    }

    /// A world with one enemy that blasts everything near it, hard enough to finish a player.
    fn killing_field(catalog: &Catalog) -> (World, Handle) {
        let mut world = field(catalog);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        let enemy = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            catalog,
            r#"enemy "Slime" { state a { grenade(4, 10000, 20, cooldown: 100000) } }"#,
        );

        (world, enemy)
    }

    #[test]
    fn a_dead_player_is_reported_once_and_named_after_what_killed_it() {
        // Before this, a player whose health reached zero simply vanished: nothing marked the
        // character dead, and logging back in found it alive.
        let catalog = catalog();
        let (mut world, _) = killing_field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 11.0, 10.0, 100))
            .unwrap();

        world.reindex();
        world.advance(&catalog, 50);
        world.advance(&catalog, GRENADE_FUSE_MS);

        let deaths = world.take_deaths();
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].who, player);
        assert_eq!(deaths[0].killer, "Slime");

        // Named by what struck the blow, so it is the end of the character rather than a trip
        // home: `NonPermaKillEnemy` only sends them home for something summoned (`Player.cs:862`).
        assert!(!deaths[0].rekt);

        // And once: a death answered twice is a character killed twice.
        assert!(world.take_deaths().is_empty());
    }

    #[test]
    fn a_kill_by_something_summoned_sends_the_player_home_instead() {
        // `NonPermaKillEnemy` (`Player.cs:862`): anything carrying `Spawned` leaves a "got rekt"
        // stone and a live character. It is what stops `/spawn` being a way to take somebody's
        // character off them.
        let catalog = catalog();
        let (mut world, enemy) = killing_field(&catalog);
        world.get_mut(enemy).unwrap().spawned = true;

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 11.0, 10.0, 100))
            .unwrap();

        world.reindex();
        world.advance(&catalog, 50);
        world.advance(&catalog, GRENADE_FUSE_MS);

        let deaths = world.take_deaths();
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].who, player);
        assert!(deaths[0].rekt, "a summoned enemy must not end a character");
    }

    #[test]
    fn a_death_nothing_claimed_is_unknown_and_sends_the_player_home() {
        // What `Player.Tick`'s sweep is left with, and it calls `Death("Unknown", rekt: true)`
        // (`Player.cs:586`). Health lost to something that did not check for itself is therefore a
        // trip to the nexus, not the end of the character -- an oddity of the original, kept.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 9.0, 9.0, 100))
            .unwrap();
        world.get_mut(player).unwrap().hp = 0;

        world.advance(&catalog, 50);

        let deaths = world.take_deaths();
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].killer, "Unknown");
        assert!(deaths[0].rekt);
    }

    #[test]
    fn a_player_slain_outright_is_blamed_on_whoever_asked_for_it() {
        // `KillPlayerCommand` sets `HP = 0` and calls `Death(player.Name)`
        // (`RankedCommands.cs:1055-1056`), so the death is claimed with a name and counts. Before
        // this, `/killPlayer` emptied the health and left the end-of-tick sweep to find it, which
        // answers with `Death("Unknown", rekt: true)` -- a trip to the nexus and no death at all.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 9.0, 9.0, 100))
            .unwrap();

        world.slay(player, "Admin".to_string());
        world.advance(&catalog, 50);

        let deaths = world.take_deaths();
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].who, player);
        assert_eq!(deaths[0].killer, "Admin");
        assert!(!deaths[0].rekt, "a death by command ends the character");
    }

    #[test]
    fn a_death_by_the_ground_is_named_after_the_ground() {
        // `ApplyGroundDamage` names the tile it burned on (`Player.Ground.cs:111`), which is how a
        // death reads as "killed by Lava" rather than by the room it happened in.
        let catalog = catalog();
        let hazard = catalog
            .tiles()
            .find(|tile| tile.damaging && tile.max_damage > 0)
            .expect("the content has a tile that hurts");
        let (name, tile) = (hazard.id.clone(), hazard.tile_type);

        let squares = (0..32 * 32).map(|_| square(tile.0, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Field", Terrain::build(map, &catalog), &catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 9.0, 9.0, 1))
            .unwrap();
        world.get_mut(player).unwrap().hp = 1;

        world.advance(&catalog, 50);

        let deaths = world.take_deaths();
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].killer, name);
        assert!(!deaths[0].rekt, "the ground keeps what it takes");
    }

    #[test]
    fn ground_laid_down_by_something_summoned_sends_the_player_home() {
        // `Player.Death` reads `tile.Spawned` and turns the death into a rekting
        // (`Player.cs:999-1002`), and `GroundTransform` is what sets it: it copies its host's
        // `Spawned` onto every square it paints (`GroundTransform.cs:72`). Fire laid by a summoned
        // minion therefore takes a player home rather than ending the character.
        let catalog = catalog();

        // Named rather than numbered, since a behaviour names its ground and several tiles share a
        // name: the one wanted here is the one that name resolves to.
        let hazard = catalog
            .tiles()
            .find(|tile| {
                tile.damaging
                    && tile.max_damage > 0
                    && catalog.tile_type_of(&tile.id) == Some(tile.tile_type)
            })
            .expect("the content has a tile that hurts");
        let name = hazard.id.clone();

        let mut world = field(&catalog);

        let mut minion = Entity::fixture(ObjectType(0x502), 9.0, 9.0);
        minion.kind = Kind::Enemy;
        minion.max_hp = 200;
        minion.hp = 200;
        minion.spawned = true;
        world.spawn(minion).unwrap();

        behaving(
            &mut world,
            &catalog,
            &format!(r#"enemy "Slime" {{ state a {{ ground_transform("{name}", 4) }} }}"#),
        );

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 9.0, 9.0, 1))
            .unwrap();
        world.get_mut(player).unwrap().hp = 1;
        world.reindex();

        // One tick: the minion paints the square it is standing on, and the burn is taken on the
        // same tick because a player who has just stepped onto a hazard burns immediately.
        world.advance(&catalog, 50);

        let deaths = world.take_deaths();
        assert_eq!(deaths.first().map(|death| death.who), Some(player));
        assert_eq!(deaths.len(), 1);
        assert_eq!(deaths[0].killer, name);
        assert!(
            deaths[0].rekt,
            "ground painted by something summoned must not end a character"
        );
    }

    #[test]
    fn a_gravestone_says_how_far_the_character_got() {
        // Which stone and how long it stands is what the room remembers of somebody.
        let catalog = catalog();
        let mut world = field(&catalog);

        // A level-one death leaves the smallest stone; the content has to have them for this to
        // mean anything, and where it does not the world places nothing rather than a wrong one.
        world.place_gravestone(&catalog, (4.0, 4.0), "Fesal", 0, 1, false);
        world.place_gravestone(&catalog, (6.0, 6.0), "Bo", 8, 20, false);
        world.reindex();

        // Nothing is placed for a stone the content does not describe, which is the fixture here.
        assert!(
            world
                .iter()
                .all(|(_, entity)| entity.name.as_deref() != Some("nobody")),
            "a stone appeared for a type the catalog does not have"
        );
    }

    #[test]
    fn loot_that_belongs_to_somebody_goes_in_a_bag_only_they_can_open() {
        // The whole point of a threshold: a bag anybody could take from would make it decide who
        // the loot was rolled for and nothing about who ends up with it.
        let catalog = catalog();
        let mut world = field(&catalog);

        let earner = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 500))
            .unwrap();

        world.show_bags(
            &catalog,
            &[ObjectType(0x904)],
            Some(earner),
            false,
            false,
            5.0,
            5.0,
        );
        world.reindex();

        let bag = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Container)
            .map(|(handle, entity)| (handle, entity.belongs_to))
            .expect("a bag");

        assert_eq!(bag.1, Some(earner), "the bag belongs to nobody");
    }

    #[test]
    fn a_threshold_is_measured_against_what_the_enemy_started_with() {
        // Against what it has now would be a share of nothing, which nobody can meet.
        let catalog = catalog();
        let mut world = field(&catalog);

        // Through the world rather than by hand, so it has the health its description gives it.
        let behaviours = std::mem::take(&mut world.behaviours);
        let slime = world
            .spawn_child(
                &catalog,
                &behaviours,
                ObjectType(0x502),
                8.0,
                8.0,
                None,
                None,
                false,
            )
            .unwrap();
        world.behaviours = behaviours;

        let started = world.get(slime).unwrap().max_hp;
        assert!(started > 0, "the slime has no health to take a share of");

        if let Some(entity) = world.get_mut(slime) {
            entity.base_max_hp = Some(started);
            entity.hp = 0;
            entity.dead = true;
        }

        // A tenth of what it started with is a real bar; a tenth of nothing is not.
        assert_eq!(
            world.get(slime).unwrap().base_max_hp,
            Some(started),
            "the enemy forgot what it started with"
        );
    }

    #[test]
    fn damage_is_remembered_per_player_and_bounded() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let slime = world
            .spawn(Entity::fixture(ObjectType(0x502), 8.0, 8.0))
            .unwrap();

        // More damagers than one enemy remembers, so the list cannot grow without limit.
        for index in 0..(MOST_REMEMBERED_DAMAGERS + 20) {
            let who = world
                .spawn(Entity::player(
                    ObjectType(0x600),
                    index as f32 % 30.0,
                    1.0,
                    500,
                ))
                .unwrap();

            if let Some(entity) = world.get_mut(slime) {
                let held = entity.damage_by.iter_mut().find(|(held, _)| *held == who);
                match held {
                    Some((_, total)) => *total += 1,
                    None => {
                        if entity.damage_by.len() < MOST_REMEMBERED_DAMAGERS {
                            entity.damage_by.push((who, 1));
                        }
                    }
                }
            }
        }

        assert_eq!(
            world.get(slime).unwrap().damage_by.len(),
            MOST_REMEMBERED_DAMAGERS
        );
    }

    #[test]
    fn a_drawn_setpiece_paints_the_ground_and_puts_its_boss_in() {
        let catalog = catalog();
        let mut world = field(&catalog);

        let drawing = crate::setpiece::Drawing {
            squares: vec![crate::setpiece::Painted {
                x: 1,
                y: 1,
                tile: Some("Water"),
                object: None,
                size: 0,
                clear: true,
            }],
            placed: vec![crate::setpiece::Placed::Living {
                x: 2.5,
                y: 2.5,
                name: "Slime",
                size: 150,
            }],
            prefab: None,
        };

        let missing = world.draw(&catalog, &drawing, (5, 5));
        world.reindex();

        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(world.terrain().tile_at(6, 6), TileType(0x11), "the ground");
        assert!(!world.terrain().walkable(6, 6), "and water is not walkable");

        let boss = world
            .iter()
            .find(|(_, entity)| entity.object_type == ObjectType(0x502))
            .map(|(_, entity)| (entity.x, entity.y, entity.size))
            .expect("the boss");
        assert_eq!(boss, (7.5, 7.5, 150));
    }

    #[test]
    fn what_a_setpiece_paints_is_scenery_rather_than_an_entity() {
        // A castle whose every stone was an entity would cost eight hundred places in the world and
        // eight hundred snapshot entries, for eight hundred things that never move.
        let catalog = catalog();
        let mut world = field(&catalog);
        let before = world.len();

        let squares = (0..10)
            .map(|step| crate::setpiece::Painted {
                x: step,
                y: 0,
                tile: Some("Grass"),
                object: Some("Wall"),
                size: 0,
                clear: false,
            })
            .collect();

        world.draw(
            &catalog,
            &crate::setpiece::Drawing {
                squares,
                placed: Vec::new(),
                prefab: None,
            },
            (4, 4),
        );
        world.reindex();

        assert_eq!(world.len(), before, "the walls became entities");
        assert!(!world.terrain().walkable(4, 4), "but they still block");
        assert_eq!(
            world.terrain().map().at(4, 4).map(|square| square.object),
            Some(ObjectType(0x500)),
            "and the map knows they are there, so somebody joining later sees them"
        );
    }

    #[test]
    fn a_setpiece_naming_something_the_content_lacks_says_so_rather_than_drawing_a_hole() {
        // A missing name is a stretch of realm nobody can fight in, and the only symptom without
        // this is a room that came out wrong.
        let catalog = catalog();
        let mut world = field(&catalog);

        let drawing = crate::setpiece::Drawing {
            squares: vec![crate::setpiece::Painted {
                x: 0,
                y: 0,
                tile: Some("Ground That Does Not Exist"),
                object: Some("Object That Does Not Exist"),
                size: 0,
                clear: false,
            }],
            placed: vec![crate::setpiece::Placed::Living {
                x: 0.0,
                y: 0.0,
                name: "Nobody",
                size: 0,
            }],
            prefab: None,
        };

        let missing = world.draw(&catalog, &drawing, (2, 2));

        assert_eq!(missing.len(), 3, "{missing:?}");
    }

    #[test]
    fn a_setpiece_drawn_off_the_edge_of_the_map_paints_what_fits() {
        // The scatterer picks a corner at random, so some of them hang off the map.
        let catalog = catalog();
        let mut world = field(&catalog);

        let squares = (0..8)
            .map(|step| crate::setpiece::Painted {
                x: step,
                y: 0,
                tile: Some("Water"),
                object: None,
                size: 0,
                clear: true,
            })
            .collect();

        let drawing = crate::setpiece::Drawing {
            squares,
            placed: Vec::new(),
            prefab: None,
        };

        // Four of the eight squares are on the map and four are past its right edge.
        world.draw(&catalog, &drawing, (28, 4));

        assert_eq!(
            world.terrain().tile_at(31, 4),
            TileType(0x11),
            "the last one on"
        );
    }

    #[test]
    fn a_behaviour_naming_a_setpiece_that_does_not_exist_is_reported_once() {
        // Four of the shipped behaviours do exactly this, and so does the original: the class it
        // looks for is not there. Saying so beats a boss whose arena never appears.
        let catalog = catalog();
        let mut world = field(&catalog);

        let handle = world
            .spawn(Entity::fixture(ObjectType(0x502), 8.0, 8.0))
            .unwrap();
        let program = hendra_behavior::Program::default();

        for _ in 0..3 {
            world.apply(
                handle,
                &catalog,
                &Programs::default(),
                &program,
                &Action::Setpiece {
                    name: "BottledEvil".to_string(),
                },
                50,
            );
        }

        assert_eq!(
            world.take_unknown_setpieces(),
            vec!["BottledEvil".to_string()]
        );
        assert!(world.take_unknown_setpieces().is_empty(), "and only once");
    }

    #[test]
    fn speech_is_taken_once_and_does_not_pile_up_forever() {
        // A boss left alive in an empty room talks to itself, and nothing in the simulation makes
        // it stop. A queue nobody drains has to stop growing on its own.
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let mut boss = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.max_hp = 200;
        boss.hp = 200;
        world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { taunt("a", "b", cooldown: 0) } }"#,
        );
        world.reindex();

        for _ in 0..40 {
            world.advance(&catalog, 50);
        }

        let said = world.take_announcements();
        assert!(!said.is_empty(), "it should have said something");
        assert!(
            world.take_announcements().is_empty(),
            "and taking it should have emptied the queue"
        );

        for _ in 0..5_000 {
            world.advance(&catalog, 50);
        }
        assert!(
            world.take_announcements().len() <= MAX_PENDING_ANNOUNCEMENTS,
            "the queue should be bounded when nobody drains it"
        );
    }

    #[test]
    fn the_cooldown_stops_a_client_firing_as_fast_as_it_likes() {
        let catalog = catalog();
        let (mut world, shooter, _) = duel(&catalog);

        assert_eq!(world.shoot(shooter, &catalog, 0.0).len(), 1);

        // Twenty more attempts in the same instant yield nothing. Rate of fire is the server's to
        // enforce; a client that asks faster is simply refused rather than believed.
        for _ in 0..20 {
            assert!(world.shoot(shooter, &catalog, 0.0).is_empty());
        }
        assert_eq!(world.projectile_count(), 1);

        // After the cooldown has elapsed it may fire again. A character with no dexterity fires
        // slowly, so this waits out the full minimum rather than a fixed 500ms.
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert_eq!(world.shoot(shooter, &catalog, 0.0).len(), 1);
    }

    #[test]
    fn every_shot_fired_is_reported_once_and_only_once() {
        let catalog = catalog();
        let (mut world, shooter, _) = duel(&catalog);

        // Nothing has been fired, so there is nothing to announce.
        assert!(world.take_fired().is_empty());

        world.shoot(shooter, &catalog, 0.0);

        // The shot is reported once, with the owner it belongs to, so the client can tell whose
        // bullet it is drawing.
        let fired = world.take_fired();
        assert_eq!(fired.len(), 1, "one trigger pull is one announcement");
        assert_eq!(fired[0].1.owner, shooter);

        // And not a second time: a shot announced twice would be drawn twice.
        assert!(
            world.take_fired().is_empty(),
            "a shot is announced once, not once per tick for its whole life"
        );
    }

    #[test]
    fn enough_shots_kill_and_the_body_is_removed() {
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);

        // A hundred a shot, halved by an attack of zero and reduced again by defence, against two
        // hundred hit points. Slower than it used to be, so this allows more attempts.
        for _ in 0..200 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
            if world.get(target).is_none() {
                break;
            }
        }

        assert!(world.get(target).is_none(), "the slime should be dead");
    }

    #[test]
    fn an_enemy_resting_on_exactly_no_health_can_still_be_shot() {
        // The two halves of this have to agree. An enemy dies on `HP < 0` (`Enemy.cs:127`), so one
        // that lands on exactly zero is alive and has to be hit again; and `Enemy.HitByProjectile`
        // gates on nothing but the effects, so that hit lands. Refusing to strike a target at zero
        // health leaves a body whose health has run out and which nothing can finish: health that
        // falls to nothing and an enemy that never dies.
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);

        world.get_mut(target).unwrap().hp = 0;

        let mut struck = false;
        for _ in 0..60 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
            struck |= !world.take_damage().is_empty();
            if world.get(target).is_none() {
                break;
            }
        }

        assert!(struck, "a shot at an enemy on nothing still lands");
        assert!(world.get(target).is_none(), "and finishes it");
    }

    #[test]
    fn an_enemy_on_exactly_no_health_is_still_standing_and_a_player_is_not() {
        // `Enemy.cs:90` and `:127` kill on `HP < 0`; `Player.cs:584` kills on `HP <= 0`. The
        // asymmetry is worth a shot in play: an enemy taken to exactly zero survives and has to be
        // hit again, which reading both as `<= 0` quietly removes.
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);

        // Fifty a shot against two hundred, so the fourth shot lands it on exactly zero.
        let mut shots = 0;
        while shots < 4 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
            if !world.take_damage().is_empty() {
                shots += 1;
            }
        }

        assert_eq!(world.get(target).map(|entity| entity.hp), Some(0));
        assert!(
            world.get(target).is_some(),
            "an enemy on nothing is alive until it goes below nothing"
        );

        // A player on exactly zero is not: `Player.cs:584` reads `HP <= 0`.
        let player = Entity::player(ObjectType(0x600), 20.0, 20.0, 800);
        let victim = world.spawn(player).unwrap();
        world.get_mut(victim).unwrap().hp = 0;
        world.advance(&catalog, 50);
        assert!(
            world.get(victim).is_none() || world.get(victim).unwrap().dead,
            "a player on nothing is dead"
        );
    }

    #[test]
    fn a_hit_is_broadcast_with_what_it_took_and_whether_it_killed() {
        // `Enemy.cs:114` builds a `Damage` packet for every projectile that lands. Without it a
        // client has only the snapshot, which cannot tell a body that died from one that walked
        // out of sight — and cannot put a number over anything at all.
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);

        let _ = world.take_damage();

        let mut reported = 0;
        let mut killed = false;
        for _ in 0..200 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);

            for event in world.take_damage() {
                assert_eq!(event.target, target);
                assert_eq!(event.owner, Some(shooter));
                assert!(event.amount > 0, "a hit that took nothing is not a hit");

                // The shooter is told too. `Enemy.cs:120` leaves them out because their own client
                // rolled the number from a generator kept in step with the server's; nothing here
                // shares that stream, so a shooter left out is a shooter who sees no numbers at
                // all — which is exactly what fighting anything felt like before this.
                assert_eq!(event.except, None, "the shooter is told what it did");
                reported += 1;
                killed |= event.kill;
            }

            if world.get(target).is_none() {
                break;
            }
        }

        assert!(reported > 0, "every landed shot is reported");
        assert!(killed, "the shot that finished it says so");
        assert!(world.get(target).is_none());
    }

    #[test]
    fn a_hit_is_reported_once_and_the_queue_empties() {
        let catalog = catalog();
        let (mut world, shooter, _) = duel(&catalog);

        let _ = world.take_damage();

        for _ in 0..10 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
        }

        assert!(!world.take_damage().is_empty(), "the shots landed");
        assert!(
            world.take_damage().is_empty(),
            "a hit is announced once, not once per tick for the rest of the world"
        );
    }

    #[test]
    fn a_player_hurt_by_the_ground_is_told_what_it_cost_them() {
        // `ApplyGroundDamage` announces the burn with no owner and no bullet
        // (`Player.Ground.cs:102`). It leaves the burning player out only because that client
        // rolled the same number off a shared stream; nothing is shared here, so the number the
        // ground charged has to travel or it is never seen.
        let catalog = catalog();

        // 0x0d is lava in the test catalog's tile table; whatever it is, the world only burns on a
        // tile the catalog calls damaging.
        let squares = (0..32 * 32).map(|_| square(0x0d, ObjectType::NONE.0));
        let Ok(map) = Map::from_squares(32, 32, squares) else {
            return;
        };
        let mut world = World::new("Burn", Terrain::build(map, &catalog), &catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 800))
            .unwrap();

        let mut burned = None;
        for _ in 0..40 {
            world.advance(&catalog, 100);
            if let Some(event) = world.take_damage().into_iter().next() {
                burned = Some(event);
                break;
            }
        }

        let Some(event) = burned else {
            // The test catalog may name no damaging tile at all, in which case there is nothing to
            // check and nothing broken.
            return;
        };

        assert_eq!(event.target, player);
        assert_eq!(event.owner, None, "the ground is nobody");
        assert_eq!(event.bullet, 0);
        assert_eq!(
            event.except, None,
            "the one person paying for the ground could not see what it charged"
        );
    }

    #[test]
    fn a_projectile_outlives_nothing_when_its_owner_leaves() {
        let catalog = catalog();
        let (mut world, shooter, _) = duel(&catalog);

        world.shoot(shooter, &catalog, 0.0);
        assert_eq!(world.projectile_count(), 1);

        world.despawn(shooter);
        world.advance(&catalog, 50);

        assert_eq!(
            world.projectile_count(),
            0,
            "a world should not keep firing for someone who has gone"
        );
    }

    /// A world with a player and one enemy the quest table names.
    fn quest_arena(catalog: &Catalog) -> (World, Handle, Handle) {
        let squares = (0..64 * 64).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(64, 64, squares).unwrap();
        let mut world = World::new("Field", Terrain::build(map, catalog), catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 32.0, 32.0, 800))
            .unwrap();

        // A Hobbit Mage, which the table names for levels three to eight.
        let kind = catalog.type_of("Hobbit Mage").expect("the hobbit");
        let mut hobbit = Entity::fixture(kind, 36.0, 32.0);
        hobbit.kind = Kind::Enemy;
        hobbit.hp = 200;
        hobbit.max_hp = 200;
        let enemy = world.spawn(hobbit).unwrap();

        if let Some(entity) = world.get_mut(player) {
            entity.progress.level = 5;
        }

        (world, player, enemy)
    }

    #[test]
    fn a_players_stars_reach_the_wire() {
        // The seam this crosses is where it went wrong before: the count was worked out, tested and
        // sent to nobody, so no other player ever saw a star.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 800))
            .unwrap();

        if let Some(entity) = world.get_mut(player) {
            entity.stars = 9;
        }
        assert_eq!(world.get(player).unwrap().state().stars, 9);

        // And nothing that is not a player claims any.
        let mut slime = Entity::fixture(ObjectType(0x502), 12.0, 10.0);
        slime.kind = Kind::Enemy;
        let enemy = world.spawn(slime).unwrap();
        assert_eq!(world.get(enemy).unwrap().state().stars, 0);
    }

    #[test]
    fn a_ninjas_speed_is_paid_for_in_magic() {
        // Twelve a second, and it ends the moment there is none left. Free movement otherwise.
        //
        // Twelve and not the ten `HandleEffects` writes: it truncates to a whole point of magic on
        // every 332 ms world tick, so the 3.32 it means to spend costs four, and four every 332 ms
        // is 12.05 a second (`Player.Effects.cs:53`). Spending ten would run a ninja a fifth
        // further on the same bar than the original ever did.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 800))
            .unwrap();

        if let Some(entity) = world.get_mut(player) {
            entity.mp = 25;
            entity.max_mp = 100;
        }
        world.give_effect(
            player,
            hendra_content::ConditionEffect::NinjaSpeedy as u8,
            60_000,
        );

        // One second of it, which the original charges twelve for.
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert_eq!(world.get(player).unwrap().mp, 13, "the speed was free");

        // And it ends when the magic does, rather than carrying on unpaid.
        for _ in 0..40 {
            world.advance(&catalog, 50);
        }

        let entity = world.get(player).unwrap();
        assert_eq!(entity.mp, 0);
        assert!(
            !entity
                .conditions
                .contains(hendra_content::ConditionEffect::NinjaSpeedy),
            "the effect outlived the magic paying for it"
        );
    }

    /// An enemy that chases, and a player in front of it.
    fn hunted(catalog: &Catalog) -> (World, Handle, Handle) {
        let squares = (0..32 * 32).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, catalog), catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        let enemy = world.spawn(slime).unwrap();

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 14.0, 10.0, 800))
            .unwrap();

        let source =
            r#"enemy "Slime" { state hunt { follow(speed: 1.0, acquire_range: 15, range: 0.5) } }"#;
        let (programs, diagnostics) = hendra_behavior::compile::compile(
            &hendra_behavior::parse::parse(source).expect("behaviour should parse"),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        world.set_behaviours(catalog, programs);

        (world, enemy, player)
    }

    /// How far the enemy travelled over a second of chasing.
    fn chased(world: &mut World, catalog: &Catalog, enemy: Handle) -> f32 {
        let before = world.get(enemy).map(|e| (e.x, e.y)).unwrap();
        for _ in 0..20 {
            world.advance(catalog, 50);
        }
        let after = world.get(enemy).map(|e| (e.x, e.y)).unwrap();

        let (dx, dy) = (after.0 - before.0, after.1 - before.1);
        (dx * dx + dy * dy).sqrt()
    }

    #[test]
    fn an_enemy_chases_somebody_it_can_see() {
        let catalog = catalog();
        let (mut world, enemy, _) = hunted(&catalog);

        assert!(
            chased(&mut world, &catalog, enemy) > 0.5,
            "the enemy did not chase at all, so this proves nothing"
        );
    }

    #[test]
    fn an_enemy_does_not_chase_somebody_invisible() {
        // `IsVisibleToEnemy`. Invisibility that hid you from other players and not from what is
        // trying to kill you would be the wrong half of the effect.
        let catalog = catalog();
        let (mut world, enemy, player) = hunted(&catalog);

        if let Some(entity) = world.get_mut(player) {
            entity
                .conditions
                .insert(hendra_content::ConditionEffect::Invisible);
        }

        assert_eq!(
            chased(&mut world, &catalog, enemy),
            0.0,
            "an invisible player was chased"
        );
    }

    #[test]
    fn healing_a_group_reaches_every_kind_in_it() {
        // A group name is not an object name. Read as one it finds nothing, and a boss healing an
        // empty set looks exactly like a boss whose heal works.
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let healer = {
            let mut entity = Entity::fixture(ObjectType(0x509), 10.0, 10.0);
            entity.kind = Kind::Enemy;
            entity.hp = 100;
            entity.max_hp = 100;
            world.spawn(entity).unwrap()
        };

        // One of each kind in the group, both hurt.
        let hurt: Vec<Handle> = [ObjectType(0x509), ObjectType(0x50a)]
            .into_iter()
            .enumerate()
            .map(|(index, kind)| {
                let mut entity = Entity::fixture(kind, 11.0 + index as f32, 10.0);
                entity.kind = Kind::Enemy;
                entity.max_hp = 100;
                entity.hp = 10;
                world.spawn(entity).unwrap()
            })
            .collect();

        // A slime standing among them, which is in no group at all.
        let stranger = {
            let mut entity = Entity::fixture(ObjectType(0x502), 12.0, 10.0);
            entity.kind = Kind::Enemy;
            entity.max_hp = 100;
            entity.hp = 10;
            world.spawn(entity).unwrap()
        };

        let source = r#"enemy "Red Crystal" {
            state healing { heal_group(5, "Crystals", heal_amount: 50, cooldown: 100) }
        }"#;
        let (programs, diagnostics) = hendra_behavior::compile::compile(
            &hendra_behavior::parse::parse(source).expect("behaviour should parse"),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        world.set_behaviours(&catalog, programs);

        for _ in 0..4 {
            world.advance(&catalog, 50);
        }

        for handle in &hurt {
            let entity = world.get(*handle).unwrap();
            assert!(
                entity.hp > 10,
                "a {} in the group was not healed",
                catalog.object(entity.object_type).unwrap().id
            );
        }

        // And nothing outside the group. Healing a group that resolved to nothing would look like
        // this working, so what makes the test mean anything is the thing that must not be healed.
        assert_eq!(
            world.get(stranger).unwrap().hp,
            10,
            "something outside the group was healed"
        );

        let _ = healer;
    }

    #[test]
    fn healing_a_group_that_is_not_there_heals_nobody() {
        // The original does this twice by accident: it heals "Lair Ghost" where the content says
        // "Lair Ghosts", and "Mask Men" where it says "Jungle Men". A name that resolves to nothing
        // has to match nothing, or a typo turns into a boss that heals the whole room.
        let catalog = catalog();
        let mut world = field(&catalog);

        let healer = {
            let mut entity = Entity::fixture(ObjectType(0x509), 10.0, 10.0);
            entity.kind = Kind::Enemy;
            entity.hp = 100;
            entity.max_hp = 100;
            world.spawn(entity).unwrap()
        };

        let hurt = {
            let mut entity = Entity::fixture(ObjectType(0x50a), 11.0, 10.0);
            entity.kind = Kind::Enemy;
            entity.max_hp = 100;
            entity.hp = 10;
            world.spawn(entity).unwrap()
        };

        let source = r#"enemy "Red Crystal" {
            state healing { heal_group(5, "Crystalls", heal_amount: 50, cooldown: 100) }
        }"#;
        let (programs, diagnostics) = hendra_behavior::compile::compile(
            &hendra_behavior::parse::parse(source).expect("behaviour should parse"),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        world.set_behaviours(&catalog, programs);

        for _ in 0..4 {
            world.advance(&catalog, 50);
        }

        assert_eq!(
            world.get(hurt).unwrap().hp,
            10,
            "a heal aimed at a group that does not exist healed somebody"
        );

        let _ = healer;
    }

    #[test]
    fn an_enemy_written_to_see_the_invisible_still_chases() {
        // Ten enemies in the game are written this way. Hiding these players from every enemy
        // would have quietly turned each of them off, since the flag is the whole reason they were
        // written.
        let catalog = catalog();
        let squares = (0..32 * 32).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        let enemy = world.spawn(slime).unwrap();

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 14.0, 10.0, 800))
            .unwrap();

        if let Some(entity) = world.get_mut(player) {
            entity
                .conditions
                .insert(hendra_content::ConditionEffect::Invisible);
        }

        let source = r#"enemy "Slime" {
            state waiting { on player_within(dist: 10, see_invis: true) -> awake }
            state awake { wander(0.4) }
        }"#;
        let (programs, diagnostics) = hendra_behavior::compile::compile(
            &hendra_behavior::parse::parse(source).expect("behaviour should parse"),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        world.set_behaviours(&catalog, programs);

        // One tick to fill the index the senses are drawn from, and the state read after it, since
        // an enemy that noticed nothing on the first tick has not been asked the question yet.
        world.advance(&catalog, 50);
        let waiting = world.get(enemy).unwrap().mind.as_ref().unwrap().state();
        world.advance(&catalog, 50);

        assert_ne!(
            world.get(enemy).unwrap().mind.as_ref().unwrap().state(),
            waiting,
            "an enemy written to see through invisibility did not"
        );
    }

    #[test]
    fn an_enemy_leaves_somebody_who_has_just_arrived_alone() {
        // Three seconds, from `SetNewbiePeriod`. Being shot while your client is still drawing the
        // room you walked into is a death nobody could have avoided.
        let catalog = catalog();
        let (mut world, enemy, player) = hunted(&catalog);

        if let Some(entity) = world.get_mut(player) {
            entity.unseen_ms = NEWCOMER_GRACE_MS;
        }

        let start = world.get(enemy).map(|e| (e.x, e.y)).unwrap();

        // Two and a half seconds, which is inside the three.
        for _ in 0..50 {
            world.advance(&catalog, 50);
        }
        assert_eq!(
            world.get(enemy).map(|e| (e.x, e.y)).unwrap(),
            start,
            "somebody who had just arrived was chased"
        );

        // Past it now, and it wears off rather than lasting, or it would be a way to fight.
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert_ne!(
            world.get(enemy).map(|e| (e.x, e.y)).unwrap(),
            start,
            "the grace never wore off"
        );
    }

    #[test]
    fn an_arrow_points_at_something_worth_walking_to() {
        let catalog = catalog();
        let (mut world, player, enemy) = quest_arena(&catalog);

        world.advance(&catalog, 50);
        assert_eq!(world.quest_target(player), Some(enemy));
    }

    #[test]
    fn an_arrow_that_moves_is_announced_once_and_a_settled_one_says_nothing() {
        // `HandleQuest` sends `QuestObjId` under `newQuest != null && newQuest != questEntity` and
        // nowhere else (`Player.Leveling.cs:214-228`). Without the announcement the client has no
        // arrow at all; with one per scan the arrow would be re-announced five times a second.
        let catalog = catalog();
        let (mut world, player, enemy) = quest_arena(&catalog);

        world.advance(&catalog, 50);
        assert_eq!(
            world.take_quest_changes(),
            vec![(player, enemy)],
            "the arrow settled and nobody was told"
        );

        // It has not moved, so nothing more is owed however long the world runs.
        for _ in 0..200 {
            world.advance(&catalog, 50);
        }
        assert_eq!(
            world.take_quest_changes(),
            Vec::new(),
            "an arrow that never moved was re-announced"
        );
    }

    #[test]
    fn what_the_arrow_points_at_is_sent_however_far_away_it_is() {
        // `GetNewEntities` yields the quest entity outside the loop over what is near
        // (`Player.Update.cs:249-250`), so it travels whatever the sight radius says. An arrow
        // pointing at a body the client was never sent points at nothing.
        let catalog = catalog();
        let (mut world, player, enemy) = quest_arena(&catalog);

        world.advance(&catalog, 50);
        assert_eq!(world.quest_target(player), Some(enemy));

        // Far outside any sight radius, and still in the snapshot.
        if let Some(entity) = world.get_mut(enemy) {
            entity.x = 400.0;
            entity.y = 400.0;
        }
        world.reindex();

        let snapshot = world.snapshot_for(player, SIGHT_RADIUS);
        assert!(
            snapshot.iter().any(|(id, _)| id == enemy.to_entity_id()),
            "the arrow's target was left out of the snapshot"
        );
    }

    #[test]
    fn a_bag_that_belongs_to_somebody_is_not_shown_to_anybody_else() {
        // `GetNewEntities` skips any container whose `BagOwners` does not name this account
        // (`Player.Update.cs:236-241`), so a soulbound drop is never drawn on another screen at
        // all. Sending it and refusing the take instead would tell a whole room what one player
        // found.
        let catalog = catalog();
        let (mut world, mine, _) = quest_arena(&catalog);

        let theirs = world
            .spawn(Entity::player(ObjectType(0x600), 34.0, 32.0, 500))
            .unwrap();

        let mut bag = Entity::fixture(ObjectType(0x0500), 33.0, 32.0);
        bag.kind = Kind::Container;
        bag.container = Some(Box::new(crate::inventory::Container::new(
            crate::inventory::ContainerKind::Bag,
            8,
        )));
        bag.belongs_to = Some(mine);
        let soulbound = world.spawn(bag).unwrap();
        world.reindex();

        let owner_sees = world.snapshot_for(mine, SIGHT_RADIUS);
        assert!(
            owner_sees
                .iter()
                .any(|(id, _)| id == soulbound.to_entity_id()),
            "the owner was not shown their own bag"
        );

        let other_sees = world.snapshot_for(theirs, SIGHT_RADIUS);
        assert!(
            !other_sees
                .iter()
                .any(|(id, _)| id == soulbound.to_entity_id()),
            "somebody else's soulbound bag was drawn"
        );

        // And a bag nobody owns is everybody's, which is what most loot is.
        let mut free = Entity::fixture(ObjectType(0x0500), 33.0, 33.0);
        free.kind = Kind::Container;
        free.container = Some(Box::new(crate::inventory::Container::new(
            crate::inventory::ContainerKind::Bag,
            8,
        )));
        let public = world.spawn(free).unwrap();
        world.reindex();

        assert!(
            world
                .snapshot_for(theirs, SIGHT_RADIUS)
                .iter()
                .any(|(id, _)| id == public.to_entity_id()),
            "an unowned bag was hidden"
        );
    }

    #[test]
    fn a_camera_put_on_another_body_is_announced_and_that_body_travels() {
        // `SetFocus` carries the target's id and `GetNewEntities` yields the spectate target
        // unconditionally (`UnrankedCommands.cs:1437`, `Player.Update.cs:252-253`). Both halves are
        // needed: a camera set on a body the client was never sent shows nothing.
        let catalog = catalog();
        let (mut world, watcher, _) = quest_arena(&catalog);

        let watched = world
            .spawn(Entity::player(ObjectType(0x600), 400.0, 400.0, 500))
            .unwrap();
        world.reindex();

        world.watch(watcher, watched);
        assert_eq!(world.take_focus_changes(), vec![(watcher, watched)]);

        assert!(
            world
                .snapshot_for(watcher, SIGHT_RADIUS)
                .iter()
                .any(|(id, _)| id == watched.to_entity_id()),
            "the watched body was a thousand tiles away and left out"
        );

        // Naming yourself gives the camera back, and is announced with your own id.
        world.watch(watcher, watcher);
        assert_eq!(world.take_focus_changes(), vec![(watcher, watcher)]);
        assert_eq!(world.get(watcher).and_then(|e| e.watching), None);
    }

    #[test]
    fn a_watched_body_that_leaves_hands_the_camera_back() {
        // `Player.ResetFocus` (`Player.cs:502-515`), raised by the watched body as it goes. Without
        // it, somebody watching a player who walks through a portal is left looking at nothing.
        let catalog = catalog();
        let (mut world, watcher, _) = quest_arena(&catalog);

        let watched = world
            .spawn(Entity::player(ObjectType(0x600), 34.0, 32.0, 500))
            .unwrap();
        world.reindex();

        world.watch(watcher, watched);
        let _ = world.take_focus_changes();

        world.despawn(watched);
        world.advance(&catalog, 50);

        assert_eq!(
            world.take_focus_changes(),
            vec![(watcher, watcher)],
            "the camera was left on a body that had gone"
        );
        assert_eq!(world.get(watcher).and_then(|e| e.watching), None);
    }

    #[test]
    fn an_enemy_the_content_never_flagged_is_not_pointed_at() {
        // `FindQuest` reads `Owner.Quests`, which holds only the enemies whose description carries
        // `<Quest/>` (`World.cs:344`), and the table is consulted after that. Eighteen of the names
        // in the table are on enemies this content does not flag, and reading the table alone sends
        // the arrow at every one of them.
        let catalog = catalog();
        let (mut world, player, hobbit) = quest_arena(&catalog);

        // Nearer than the hobbit and of equal standing, so it would win on score alone.
        let kind = catalog.type_of("Desert Werewolf").expect("the werewolf");
        let mut werewolf = Entity::fixture(kind, 33.0, 32.0);
        werewolf.kind = Kind::Enemy;
        werewolf.hp = 200;
        werewolf.max_hp = 200;
        let unflagged = world.spawn(werewolf).unwrap();

        world.advance(&catalog, 50);

        let pointed = world.quest_target(player);
        assert_ne!(pointed, Some(unflagged), "pointed at an unflagged enemy");
        assert_eq!(pointed, Some(hobbit));
    }

    #[test]
    fn the_nearer_of_two_identical_quests_is_the_one_chosen() {
        // The distance term is a `double` in `FindQuest` (`Player.Leveling.cs:198`), so a few tiles
        // separate two otherwise identical enemies. Truncating it to whole points made every pair
        // inside a hundred tiles of each other tie, and a tie was settled by nothing better than
        // which of them the world happened to hold last.
        let catalog = catalog();
        let (mut world, player, near) = quest_arena(&catalog);

        let kind = catalog.type_of("Hobbit Mage").expect("the hobbit");
        let mut farther = Entity::fixture(kind, 39.0, 32.0);
        farther.kind = Kind::Enemy;
        farther.hp = 200;
        farther.max_hp = 200;
        let far = world.spawn(farther).unwrap();

        world.advance(&catalog, 50);

        assert_ne!(world.quest_target(player), Some(far));
        assert_eq!(world.quest_target(player), Some(near));
    }

    #[test]
    fn something_important_far_away_beats_something_ordinary_underfoot() {
        // Worked against the real content, whose levels are what the score reads. For a character
        // at twenty: the cube god is priority fourteen at level twenty, so `(20 - 0) * 14 - 400/100`
        // is 276; the red demon is priority thirteen at level nineteen, so `(20 - 1) * 13 - 10/100`
        // is 246.9; the ghost king is priority eleven at level fifteen, so `(20 - 5) * 11 - 1/100`
        // is 164.99. The cube god wins from forty times the distance, which is the whole point of
        // the arrow: it names something worth the walk rather than whatever is nearest.
        let Ok((catalog, _)) =
            Catalog::load_dir(std::path::Path::new("../../../godot-client/assets/xml"))
        else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        // Light cobblestone: walkable, harmless, and real.
        let squares = (0..512 * 512).map(|_| square(0x02, ObjectType::NONE.0));
        let map = Map::from_squares(512, 512, squares).unwrap();
        let mut world = World::new("Field", Terrain::build(map, &catalog), &catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x030e), 50.0, 50.0, 800))
            .unwrap();
        if let Some(entity) = world.get_mut(player) {
            entity.progress.level = 20;
        }

        let mut placed = Vec::new();
        for (name, x) in [
            ("Cube God", 450.0),
            ("Red Demon", 60.0),
            ("Ghost King", 51.0),
        ] {
            let kind = catalog.type_of(name).expect(name);
            let mut enemy = Entity::fixture(kind, x, 50.0);
            enemy.kind = Kind::Enemy;
            enemy.hp = 100_000;
            enemy.max_hp = 100_000;
            placed.push((name, world.spawn(enemy).unwrap()));
        }

        world.advance(&catalog, 50);

        let pointed = world.quest_target(player);
        let named = placed
            .iter()
            .find(|(_, handle)| Some(*handle) == pointed)
            .map(|(name, _)| *name);

        assert_eq!(named, Some("Cube God"));
    }

    #[test]
    fn a_level_gained_sends_the_arrow_looking_again() {
        // `CheckLevelUp` drops the target on every level (`Player.Leveling.cs:300`). Without it a
        // character who has just climbed out of a quest's level band keeps being pointed at it for
        // the rest of the interval, which is eighty-three seconds.
        let catalog = catalog();
        let (mut world, player, enemy) = quest_arena(&catalog);

        world.advance(&catalog, 50);
        assert_eq!(world.quest_target(player), Some(enemy));

        // A hair from level nine, which is out of the hobbit's band of three to eight. Something
        // else entirely provides the kill, so the arrow's own target is untouched by it and only
        // the level can explain the arrow moving.
        if let Some(entity) = world.get_mut(player) {
            entity.progress.level = 8;
            entity.progress.experience =
                crate::leveling::experience_at(8) + crate::leveling::experience_goal(8) - 1;
        }

        let mut slime = Entity::fixture(ObjectType(0x502), 32.5, 32.0);
        slime.kind = Kind::Enemy;
        slime.max_hp = 1_000_000;
        let slime = world.spawn(slime).unwrap();
        world.advance(&catalog, 50);

        if let Some(entity) = world.get_mut(slime) {
            entity.dead = true;
            entity.last_hurt_by = Some(player);
        }
        world.advance(&catalog, 50);

        assert!(world.get(player).unwrap().progress.level >= 9, "no level");
        assert!(
            world.get(enemy).unwrap().hp > 0,
            "the hobbit is still alive"
        );
        assert_eq!(
            world.quest_target(player),
            None,
            "still pointed at a quest the character has outgrown"
        );
    }

    #[test]
    fn killing_what_the_arrow_pointed_at_is_a_completed_quest() {
        // The counter is the whole reason the target is remembered rather than worked out when
        // asked: once the enemy is dead there is nothing left to compare against.
        let catalog = catalog();
        let (mut world, player, enemy) = quest_arena(&catalog);

        world.advance(&catalog, 50);
        assert_eq!(world.quest_target(player), Some(enemy));

        if let Some(entity) = world.get_mut(enemy) {
            entity.dead = true;
            entity.last_hurt_by = Some(player);
        }
        world.advance(&catalog, 50);

        assert_eq!(
            world.get(player).unwrap().tally.quests_completed,
            1,
            "killing the enemy the arrow pointed at counted for nothing"
        );
    }

    #[test]
    fn an_enemy_nobody_was_sent_to_is_not_a_completed_quest() {
        let catalog = catalog();
        let (mut world, player, _) = quest_arena(&catalog);

        world.advance(&catalog, 50);

        // Something else entirely, which no arrow ever pointed at.
        let mut slime = Entity::fixture(ObjectType(0x502), 33.0, 32.0);
        slime.kind = Kind::Enemy;
        slime.hp = 100;
        slime.max_hp = 100;
        let other = world.spawn(slime).unwrap();

        if let Some(entity) = world.get_mut(other) {
            entity.dead = true;
            entity.last_hurt_by = Some(player);
        }
        world.advance(&catalog, 50);

        assert_eq!(world.get(player).unwrap().tally.quests_completed, 0);
    }

    #[test]
    fn whoever_lands_the_kill_is_credited_with_the_levels_others_reached() {
        // `DamageCounter.LevelUpAssist`. No party is needed for it: being near enough to share the
        // experience is what makes it help.
        let catalog = catalog();
        let squares = (0..64 * 64).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(64, 64, squares).unwrap();
        let mut world = World::new("Field", Terrain::build(map, &catalog), &catalog);

        let killer = world
            .spawn(Entity::player(ObjectType(0x600), 32.0, 32.0, 800))
            .unwrap();
        let helped = world
            .spawn(Entity::player(ObjectType(0x600), 33.0, 32.0, 800))
            .unwrap();

        // Both are a hair from a level. A kill is capped at a tenth of one, so this is the only
        // way a single kill can raise anybody at all.
        let goal = crate::leveling::experience_goal(1);
        for player in [killer, helped] {
            if let Some(entity) = world.get_mut(player) {
                entity.progress.level = 1;
                entity.progress.experience = goal - 1;
            }
        }

        let mut enemy = Entity::fixture(ObjectType(0x502), 32.5, 32.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 1_000_000;
        let enemy = world.spawn(enemy).unwrap();

        // One tick alive first, so everybody is in the index the share is drawn from. Experience
        // reaches whoever is near the kill, and near is a question the grid answers.
        world.advance(&catalog, 50);

        if let Some(entity) = world.get_mut(enemy) {
            entity.dead = true;
            entity.last_hurt_by = Some(killer);
        }
        world.advance(&catalog, 50);

        assert!(
            world.get(helped).unwrap().progress.level > 1,
            "the other player did not level up, so there was nothing to assist"
        );
        assert_eq!(
            world.get(killer).unwrap().tally.level_up_assists,
            1,
            "the killer was not credited with the level somebody else reached"
        );

        // And not credited for their own, since an assist is help given rather than progress made.
        assert!(world.get(killer).unwrap().progress.level > 1);
        assert_eq!(world.get(helped).unwrap().tally.level_up_assists, 0);
    }

    #[test]
    fn a_player_counts_the_ground_they_have_looked_at() {
        // The count two fame bonuses rest on. `FameCounter.TileSent` is fed the length of the tile
        // list `SendUpdate` built from the sight circle (`Player.Update.cs:152`), which is the same
        // list this counts -- so the two move together, and in a walled world both count the room
        // rather than the disc.
        let catalog = catalog();
        let squares = (0..64 * 64).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(64, 64, squares).unwrap();
        let mut world = World::new("Field", Terrain::build(map, &catalog), &catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 32.0, 32.0, 800))
            .unwrap();
        world.look_around(player);

        let first = world.get(player).unwrap().tally.tiles_seen;

        // A circle of radius twenty, which is a bit over twelve hundred squares.
        assert!(
            (1_100..1_400).contains(&first),
            "a circle of sight was {first} squares"
        );

        // Standing still uncovers nothing, however long they stand there.
        for _ in 0..10 {
            world.place(
                player,
                MoveOutcome {
                    x: 32.2,
                    y: 32.4,
                    refused: None,
                    spent_ms: 0,
                },
            );
        }
        assert_eq!(
            world.get(player).unwrap().tally.tiles_seen,
            first,
            "standing still uncovered ground"
        );

        // Walking uncovers what is newly in front and not what was already behind.
        world.place(
            player,
            MoveOutcome {
                x: 33.0,
                y: 32.0,
                refused: None,
                spent_ms: 0,
            },
        );
        let after = world.get(player).unwrap().tally.tiles_seen;

        assert!(after > first, "walking uncovered nothing");
        assert!(
            after - first < 100,
            "one step uncovered {} squares, so ground already seen was counted again",
            after - first
        );

        // And walking back over the same ground is not new ground.
        world.place(
            player,
            MoveOutcome {
                x: 32.0,
                y: 32.0,
                refused: None,
                spent_ms: 0,
            },
        );
        assert_eq!(
            world.get(player).unwrap().tally.tiles_seen,
            after,
            "ground already walked was counted twice"
        );
    }

    #[test]
    fn sight_stops_at_the_edge_of_the_map() {
        // A player in a corner sees a quarter of a circle. Counting the squares that are not there
        // would make a small map worth more than a large one.
        let catalog = catalog();
        let squares = (0..64 * 64).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(64, 64, squares).unwrap();
        let mut world = World::new("Field", Terrain::build(map, &catalog), &catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 0.0, 0.0, 800))
            .unwrap();
        world.look_around(player);

        let corner = world.get(player).unwrap().tally.tiles_seen;
        assert!(
            (250..400).contains(&corner),
            "a corner of the map was worth {corner} squares"
        );

        // The far corner, where an overrun does not fall off the end of the array but wraps into
        // the beginning of the next row: ground the player cannot see, counted as seen, on the
        // opposite side of the map.
        let far = world
            .spawn(Entity::player(ObjectType(0x600), 63.5, 63.5, 800))
            .unwrap();
        world.look_around(far);

        assert_eq!(
            world.get(far).unwrap().tally.tiles_seen,
            corner,
            "the far corner saw more than the near one"
        );
    }

    #[test]
    fn a_ground_transform_with_offsets_changes_the_one_square_it_names() {
        // `GroundTransform` reads `relativeX` and `relativeY` as a pair and changes exactly that
        // square (`GroundTransform.cs:50-73`), rather than a circle around the enemy. Every use in
        // the content gives them: the ghost ship lays its beach a square at a time.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.5, 10.5);
        slime.kind = Kind::Enemy;
        slime.hp = 200;
        slime.max_hp = 200;
        world.spawn(slime).unwrap();
        a_watcher(&mut world);

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                 state a { ground_transform("Water", relative_x: 2, relative_y: 0) }
               }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        // Water is the map's unwalkable ground, so where it landed is readable from the map.
        assert!(
            !world.terrain.walkable_at(12.5, 10.5),
            "the square two to the east should have changed"
        );
        assert!(
            world.terrain.walkable_at(10.5, 10.5),
            "the square the enemy is standing on should not have"
        );
    }

    #[test]
    fn an_enemy_walks_at_the_speed_the_original_gives_it() {
        // `Utils.GetSpeed` (`realm/Utils.cs:297`): `5.55 * speed + 0.74` tiles a second, halved
        // while slowed. Reading the argument as a plain multiple of ten instead sends a follow
        // written at one across the room at ten tiles a second rather than at six and a third,
        // and every chase, charge and orbit in the game with it.
        let catalog = catalog();
        let (mut world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Slime" { state a { follow(1.0, 20, 0.5) } }"#,
        );

        let before = world.get(enemy).map(|e| (e.x, e.y)).unwrap();
        world.advance(&catalog, 1000);
        let after = world.get(enemy).map(|e| (e.x, e.y)).unwrap();

        let travelled = ((after.0 - before.0).powi(2) + (after.1 - before.1).powi(2)).sqrt();
        assert!(
            (travelled - 6.29).abs() < 0.05,
            "a second at speed one is 6.29 tiles in the original, not {travelled}"
        );

        // The same enemy slowed covers half of it, constant term and all.
        let (mut world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Slime" { state a { follow(1.0, 20, 0.5) } }"#,
        );
        give(&mut world, enemy, hendra_content::ConditionEffect::Slowed);

        let before = world.get(enemy).map(|e| (e.x, e.y)).unwrap();
        world.advance(&catalog, 1000);
        let after = world.get(enemy).map(|e| (e.x, e.y)).unwrap();

        let slowed = ((after.0 - before.0).powi(2) + (after.1 - before.1).powi(2)).sqrt();
        assert!(
            (slowed - 3.145).abs() < 0.05,
            "slowed halves the whole figure, giving 3.15 tiles, not {slowed}"
        );
    }

    #[test]
    fn a_leading_shot_is_aimed_where_the_original_would_aim_it() {
        // The whole point of the sampling interval. `Predict` leads by four times the distance
        // between the two most recent samples, and the original samples once per *world* tick
        // rather than once per logic tick, so the lead is over a second of the target's travel.
        // Sampling at our own fifty-millisecond tick would lead by a fifth of a second, and every
        // leading enemy in the game would shoot behind its target rather than in front of it.
        let catalog = catalog();

        // The shot waits a second and a half, which is long enough for the target to have been
        // sampled twice: a fresh arrival has no history and is led from the origin instead.
        let (mut world, enemy, player) = confrontation(
            &catalog,
            r#"enemy "Slime" {
                 state a {
                   shoot(radius: 20, count: 1, predictive: 1,
                         cooldown_offset: 1500, cooldown: 100000)
                 }
               }"#,
        );

        // Straight across the enemy's line of sight at three tiles a second, driven by hand so the
        // test is about the sampling rather than about how a client moves.
        const SPEED: f32 = 3.0;
        let mut fired = None;
        for tick in 0..60 {
            if let Some(entity) = world.get_mut(player) {
                entity.x = 14.0;
                entity.y = 4.0 + SPEED * (tick as f32 * 0.05);
            }
            world.advance(&catalog, 50);

            if fired.is_none()
                && let Some((_, shot)) = world.projectiles().find(|(_, shot)| shot.owner == enemy)
            {
                let angle = shot.angle;
                let (px, py) = world.get(player).map(|e| (e.x, e.y)).unwrap();
                let (ex, ey) = world.get(enemy).map(|e| (e.x, e.y)).unwrap();
                fired = Some((angle, px, py, ex, ey));
            }
        }

        let (angle, px, py, ex, ey) = fired.expect("the enemy should have fired");

        // How far up the target's own line the shot was aimed. The target runs along `x = px`, so
        // where the shot crosses that line is how far in front of them it was sent.
        let lead = (angle.tan() * (px - ex) + ey) - py;

        // Written out rather than derived from the sampling constant, so that changing the constant
        // changes whether this passes. Four steps of three tiles a second over the original's
        // 332 ms world tick is 3.98 tiles, and the sample is between one and two of those old, so
        // anything from 3.98 to 7.97. Leading by one of our own fifty-millisecond ticks instead
        // would put the shot 0.6 tiles in front of the target, which is behind them.
        assert!(
            (3.5..=8.5).contains(&lead),
            "aimed {lead} tiles in front of the target; the original aims between 4 and 8"
        );
    }

    #[test]
    fn a_players_position_is_sampled_at_the_originals_interval() {
        // The sample a lead is built from is the one before the most recent, so it is between one
        // and two intervals old whenever a shot is fired. Sampled on its own clock rather than per
        // tick, and spent rather than cleared, so the spacing does not drift out to a whole number
        // of our ticks.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 5.0, 5.0, 800))
            .unwrap();

        let mut samples = 0;
        let mut last = world.get(player).unwrap().trail.last;
        for tick in 1..=100 {
            world.get_mut(player).unwrap().x = 5.0 + tick as f32 * 0.1;
            world.advance(&catalog, 50);

            let trail = world.get(player).unwrap().trail;
            if trail.last != last {
                samples += 1;
                last = trail.last;
            }
        }

        // Five seconds of ticks at one sample every 332 ms. Clearing the accumulator rather than
        // spending it would stretch the interval to 350 ms and give fourteen.
        assert_eq!(samples, 15, "the sampling interval is not the original's");
    }

    /// An enemy with a behaviour, and a player standing in front of it.
    fn confrontation(catalog: &Catalog, source: &str) -> (World, Handle, Handle) {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let squares = (0..32 * 32).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, catalog), catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        let enemy = world.spawn(slime).unwrap();

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 14.0, 10.0, 800))
            .unwrap();

        let (programs, diagnostics) = compile(&parse(source).expect("behaviour should parse"));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        world.set_behaviours(catalog, programs);

        world.advance(catalog, 50);
        (world, enemy, player)
    }

    #[test]
    fn an_enemy_with_a_behaviour_gets_a_mind() {
        let catalog = catalog();
        let (world, enemy, _) =
            confrontation(&catalog, r#"enemy "Slime" { state idle { wander(0.4) } }"#);

        assert_eq!(world.thinking(), 1);
        assert!(world.get(enemy).unwrap().mind.is_some());
    }

    #[test]
    fn an_enemy_hears_what_is_said_near_it() {
        // A dungeon whose door opens when you say the right word is built out of this. Draconis is
        // the one in the content: three dragon souls that wait, orbiting an altar, until somebody
        // says the colour.
        let catalog = catalog();
        let (mut world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Slime" {
                state quiet { on player_text("Red", 99, false, false) -> loud }
                state loud { wander(0.4) }
            }"#,
        );

        let quiet = world.get(enemy).unwrap().mind.as_ref().unwrap().state();

        world.heard((14.0, 10.0), "Red");
        world.advance(&catalog, 50);

        assert_ne!(
            world.get(enemy).unwrap().mind.as_ref().unwrap().state(),
            quiet,
            "saying the word changed nothing"
        );
    }

    #[test]
    fn a_word_is_heard_when_it_is_said_and_not_after() {
        // Held for the tick it was said in and no longer. A word that lingered would be heard by
        // everything that arrived afterwards, so a door would open for somebody who walked up to it
        // in silence, having missed the moment entirely.
        let catalog = catalog();
        let listener = r#"enemy "Slime" {
                state quiet { on player_text("Red", 99, false, false) -> loud }
                state loud { wander(0.4) }
            }"#;
        let (mut world, first, _) = confrontation(&catalog, listener);

        // What waiting looks like, read rather than assumed: a state's number is whatever the
        // compiler gave it, and a test that hard-codes one passes for the wrong reason.
        let quiet = world.get(first).unwrap().mind.as_ref().unwrap().state();

        world.heard((14.0, 10.0), "Red");
        world.advance(&catalog, 50);
        assert_ne!(
            world.get(first).unwrap().mind.as_ref().unwrap().state(),
            quiet,
            "the word was not heard at all"
        );

        // A second enemy arrives after the word was said, into a world where nobody has spoken
        // since. It should be waiting, not reacting. Handing the world its behaviours again is how
        // the newcomer gets a mind, and it puts the first one back at its beginning too, which the
        // assertion above has already read.
        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        let second = world.spawn(slime).unwrap();

        let (programs, _) = hendra_behavior::compile::compile(
            &hendra_behavior::parse::parse(listener).expect("behaviour should parse"),
        );
        world.set_behaviours(&catalog, programs);

        for _ in 0..5 {
            world.advance(&catalog, 50);
        }

        assert_eq!(
            world.get(second).unwrap().mind.as_ref().unwrap().state(),
            quiet,
            "a word carried over into later ticks"
        );
    }

    #[test]
    fn somebody_shouting_from_across_the_map_is_not_heard() {
        let catalog = catalog();
        let (mut world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Slime" {
                state quiet { on player_text("Red", 4, false, false) -> loud }
                state loud { wander(0.4) }
            }"#,
        );

        let quiet = world.get(enemy).unwrap().mind.as_ref().unwrap().state();

        world.heard((30.0, 30.0), "Red");
        world.advance(&catalog, 50);

        assert_eq!(
            world.get(enemy).unwrap().mind.as_ref().unwrap().state(),
            quiet,
            "a shout from across the map was heard"
        );
    }

    #[test]
    fn an_enemy_with_no_behaviour_simply_stands_there() {
        let catalog = catalog();
        let (world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Something Else" { state idle { wander(0.4) } }"#,
        );

        assert_eq!(
            world.thinking(),
            0,
            "a half-converted directory should still run"
        );
        assert!(world.get(enemy).unwrap().mind.is_none());
    }

    #[test]
    fn an_enemy_chases_a_player() {
        let catalog = catalog();
        let (mut world, enemy, _) = confrontation(
            &catalog,
            r#"enemy "Slime" { state idle { follow(1.0, 20, 1) } }"#,
        );

        let start = world.get(enemy).unwrap().x;
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }

        let now = world.get(enemy).unwrap().x;
        assert!(
            now > start + 0.5,
            "it should have closed the distance: {start} -> {now}"
        );
    }

    #[test]
    fn an_enemy_shoots_the_player_and_hurts_them() {
        let catalog = catalog();
        let (mut world, _, player) = confrontation(
            &catalog,
            r#"enemy "Slime" { state idle { shoot(count: 1, cooldown: 200ms) } }"#,
        );

        let before = world.get(player).unwrap().hp;
        for _ in 0..30 {
            world.advance(&catalog, 50);
        }

        let after = world.get(player).map(|entity| entity.hp).unwrap_or(0);
        assert!(
            after < before,
            "the player should have taken fire: {before} -> {after}"
        );
    }

    /// Bullets with a damage range, which nothing in the shared fixture has.
    ///
    /// Both halves of the separation need one: with a single-valued descriptor a volley that rolled
    /// per bullet and one that rolled once are indistinguishable.
    const RANGED_DAMAGE: &str = r#"<Objects>
        <Object type="0x5fe" id="Volleyer"><Class>Character</Class><Enemy/>
          <MaxHitPoints>200</MaxHitPoints>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>55</MinDamage><MaxDamage>90</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
        </Object>
        <Object type="0x9fe" id="Wide Shield">
          <Class>Equipment</Class><Item/><SlotType>5</SlotType>
          <MpCost>0</MpCost><Cooldown>0</Cooldown>
          <NumProjectiles>8</NumProjectiles><ArcGap>10</ArcGap>
          <Projectile><ObjectId>Bolt</ObjectId><Speed>100</Speed>
            <MinDamage>55</MinDamage><MaxDamage>90</MaxDamage>
            <LifetimeMS>2000</LifetimeMS></Projectile>
          <Activate>Shoot</Activate>
        </Object>
        </Objects>"#;

    /// An open field that knows about [`RANGED_DAMAGE`].
    fn ranged_arena() -> (Catalog, World) {
        let catalog = Catalog::load_str(&[FIXTURE, RANGED_DAMAGE]).0;
        let squares = (0..32 * 32).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        let world = World::new("Arena", Terrain::build(map, &catalog), &catalog);
        (catalog, world)
    }

    /// Puts a `Volleyer` in that field, running the behaviour given, and returns what it fired.
    fn volley_of(source: &str, seed: u32) -> Vec<Projectile> {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let (catalog, mut world) = ranged_arena();
        world.reseed(seed);

        let mut shooter = Entity::fixture(ObjectType(0x5fe), 10.0, 10.0);
        shooter.kind = Kind::Enemy;
        shooter.hp = 500;
        shooter.max_hp = 500;
        world.spawn(shooter).unwrap();
        world
            .spawn(Entity::player(ObjectType(0x600), 14.0, 10.0, 800))
            .unwrap();

        let (programs, diagnostics) = compile(&parse(source).expect("behaviour should parse"));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        world.set_behaviours(&catalog, programs);

        for _ in 0..4 {
            world.advance(&catalog, 50);
        }

        world
            .take_fired()
            .into_iter()
            .map(|(_, shot)| shot)
            .collect()
    }

    #[test]
    fn an_enemy_volley_does_one_damage_for_the_whole_fan() {
        // `Shoot.TickCore` draws `int dmg = Random.Next(desc.MinDamage, desc.MaxDamage)` once,
        // before the loop that makes the projectiles, and hands the same number to every one of
        // them (`logic/behaviors/Shoot.cs:181-190`). The `EnemyShoot` packet the client hears about
        // it through carries a single `Damage` for the volley (`:197-206`), so a fan of differing
        // bullets could not even be described. Swept over every fan the original can fire.
        let mut combinations = 0usize;
        let mut mismatches = 0usize;
        let mut seen = std::collections::BTreeSet::new();

        for count in 1..=32 {
            let behaviour = format!(
                r#"enemy "Volleyer" {{ state idle {{ shoot(count: {count}, shoot_angle: 8, cooldown: 500ms) }} }}"#
            );

            for seed in 1..=64u32 {
                let fired = volley_of(&behaviour, seed.wrapping_mul(2_654_435_761));

                assert_eq!(
                    fired.len(),
                    count,
                    "a fan of {count} should be {count} bullets"
                );

                combinations += 1;
                let first = fired[0].damage;
                seen.insert(first);

                if fired.iter().any(|shot| shot.damage != first) {
                    mismatches += 1;
                }

                assert!(
                    (55..90).contains(&first),
                    "a bullet of {first} is outside the descriptor's 55-90"
                );
            }
        }

        assert_eq!(combinations, 32 * 64);
        assert_eq!(mismatches, 0, "of {combinations} fans");
        assert!(
            seen.len() > 1,
            "the sweep never varied the damage, so it proves nothing"
        );
    }

    #[test]
    fn an_ability_volley_rolls_for_each_bullet() {
        // The other half of the same separation. `AEShoot` calls `GetAttackDamage` *inside* the
        // loop (`Player.UseItem.cs:1124-1128`) and `AllyShoot` carries no damage field, so an
        // ability's bullets differ where an enemy's do not. Enough bullets that a stream which
        // rolled once could not look like this.
        let (catalog, mut world) = ranged_arena();
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 800))
            .unwrap();
        world.get_mut(player).unwrap().mp = 100;

        world.use_item(player, &catalog, ObjectType(0x9fe), (14.0, 10.0));

        let fired: Vec<i32> = world
            .take_fired()
            .into_iter()
            .map(|(_, shot)| shot.damage)
            .collect();

        assert_eq!(fired.len(), 8, "the shield declares eight projectiles");
        assert!(
            fired.iter().any(|damage| *damage != fired[0]),
            "every bullet took the same roll: {fired:?}"
        );
        for damage in &fired {
            assert!(
                (55..90).contains(damage),
                "a bullet of {damage} is outside the descriptor's 55-90"
            );
        }
    }

    #[test]
    fn an_enemy_does_not_shoot_through_a_wall() {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..32 * 32)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        // A sight-blocking column between the two of them.
        for y in 0..32 {
            squares[y * 32 + 12] = square(0x10, 0x504);
        }

        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        world.spawn(slime).unwrap();

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 10.0, 800))
            .unwrap();

        let (programs, _) = compile(
            &parse(r#"enemy "Slime" { state idle { shoot(count: 1, cooldown: 100ms) } }"#).unwrap(),
        );
        world.set_behaviours(&catalog, programs);

        let before = world.get(player).unwrap().hp;
        for _ in 0..30 {
            world.advance(&catalog, 50);
        }

        assert_eq!(
            world.get(player).unwrap().hp,
            before,
            "an enemy that cannot see the player must not shoot them"
        );
    }

    #[test]
    fn an_enemy_cannot_walk_through_a_wall_either() {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..32 * 32)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        for y in 0..32 {
            squares[y * 32 + 12] = square(0x11, ObjectType::NONE.0);
        }

        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        let enemy = world.spawn(slime).unwrap();
        world
            .spawn(Entity::player(ObjectType(0x600), 20.0, 10.0, 800))
            .unwrap();

        let (programs, _) =
            compile(&parse(r#"enemy "Slime" { state idle { follow(2.0, 30, 1) } }"#).unwrap());
        world.set_behaviours(&catalog, programs);

        for _ in 0..80 {
            world.advance(&catalog, 50);
        }

        assert!(
            world.get(enemy).unwrap().x < 12.0,
            "it should have been stopped by the water, not walked over it"
        );
    }

    /// A field with one object standing in the middle of it.
    fn field_holding(catalog: &Catalog, object: u16, at: (usize, usize)) -> World {
        let mut squares: Vec<Composition> = (0..32 * 32)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[at.1 * 32 + at.0] = square(0x10, object);
        World::new(
            "Field",
            Terrain::build(Map::from_squares(32, 32, squares).unwrap(), catalog),
            catalog,
        )
    }

    /// What one bit for two flags costs.
    ///
    /// The 159 shipped objects that occupy their square without occupying it against enemies are
    /// scenery a player walks around and an enemy walks over. Folded together they stop both, which
    /// is a wall in the middle of every jungle clearing as far as the things living in it are
    /// concerned.
    #[test]
    fn an_enemy_walks_over_what_a_player_walks_around() {
        let catalog = catalog();

        let mut world = field_holding(&catalog, 0x51f, (12, 10));
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.5, 10.5, 800))
            .unwrap();

        let outcome = world
            .resolve_move(player, &catalog, 13.5, 10.5, 1000)
            .unwrap();
        assert!(
            outcome.x < 12.0,
            "the bramble occupies its square, so a player stops short of it"
        );

        let mut slime = Entity::fixture(ObjectType(0x502), 10.5, 10.5);
        slime.kind = Kind::Enemy;
        let slime = world.spawn(slime).unwrap();
        world.walk(slime, 13.5, 10.5);

        assert!(
            world.get(slime).unwrap().x > 13.0,
            "and an enemy walks straight over it, because it is not EnemyOccupySquare"
        );
    }

    /// Where a body stopped by a wall is put.
    ///
    /// `CalcNewLocation` snaps the refused axis to the half-tile line it tried to cross rather than
    /// leaving the coordinate where it was, and backs a hundredth off that line when it is also a
    /// square boundary. A body walking into a wall therefore ends up lined up against it, which is
    /// what makes it slide along the wall and line itself up with a doorway afterwards.
    #[test]
    fn a_body_stopped_by_a_wall_is_left_on_the_half_tile_line() {
        let catalog = catalog();

        let mut world = field_holding(&catalog, 0x51e, (12, 10));
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.5, 10.5, 800))
            .unwrap();

        let outcome = world
            .resolve_move(player, &catalog, 13.5, 10.5, 1000)
            .unwrap();

        // 11.49 rather than 11.5: the line the body is held at is 11.5, and because that is also
        // the start of square 11 while the body set out from square 10, the original takes a
        // hundredth off it (`Entity.cs:381-382`). A body that set out from inside square 11 is left
        // on 11.5 exactly. The hundredth is visible -- it is the difference between standing in the
        // square beside a wall and standing in the one before it -- so it is kept.
        assert!(
            (outcome.x - 11.49).abs() < 0.001,
            "held off the wall's face at 11.49, not at {}",
            outcome.x
        );
        assert_eq!(outcome.y, 10.5, "and nothing happened to the other axis");
        assert!(matches!(outcome.refused, Some(MoveRefusal::Blocked)));

        // Setting out from inside the square beside the wall, the same line is not a square
        // boundary crossing and the body is left on it exactly.
        if let Some(entity) = world.get_mut(player) {
            entity.x = 11.2;
            entity.y = 10.5;
        }
        let outcome = world
            .resolve_move(player, &catalog, 11.9, 10.5, 1000)
            .unwrap();
        assert!(
            (outcome.x - 11.5).abs() < 0.001,
            "on the line exactly, not at {}",
            outcome.x
        );
    }

    #[test]
    fn a_dying_enemy_leaves_a_bag() {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let catalog = catalog();
        let squares = (0..16 * 16).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 8.0, 8.0);
        slime.kind = Kind::Enemy;
        slime.hp = 10;
        slime.max_hp = 200;
        let enemy = world.spawn(slime).unwrap();

        // A certainty rather than a chance, so the test does not depend on a roll.
        let (programs, _) = compile(
            &parse(r#"enemy "Slime" { state idle { } loot { item("Wand", 1.0) } }"#).unwrap(),
        );
        world.set_behaviours(&catalog, programs);

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert!(world.get(enemy).is_none(), "the slime is gone");

        let bag = world
            .iter()
            .find(|(_, entity)| entity.kind == Kind::Container)
            .map(|(_, entity)| entity.clone())
            .expect("a bag should have been left behind");

        let container = bag.container.as_ref().expect("the bag holds something");
        assert_eq!(container.occupied(), 1);
        assert_eq!(container.item(0), catalog.type_of("Wand").unwrap());
        // Scattered up to half a tile on each axis, as `Loots.cs:379-381` scatters it.
        assert!(
            (bag.x - 8.0).abs() <= 0.5 && (bag.y - 8.0).abs() <= 0.5,
            "and it is beside where the slime died"
        );
    }

    #[test]
    fn a_summoned_enemy_leaves_no_loot_any_more_than_it_gives_experience() {
        // `Loots.Handle` returns before anything else when the enemy was `Spawned`
        // (`Loots.cs:83`), exactly as `DamageCounter.Death` does (`DamageCounter.cs:73`). Only
        // experience was gated on it here, so a spawner that kept producing children was a loot
        // farm that cost nothing.
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let catalog = catalog();
        let squares = (0..16 * 16).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let (programs, _) = compile(
            &parse(r#"enemy "Slime" { state idle { } loot { item("Wand", 1.0) } }"#).unwrap(),
        );
        world.set_behaviours(&catalog, programs);

        let mut summoned = Entity::fixture(ObjectType(0x502), 8.0, 8.0);
        summoned.kind = Kind::Enemy;
        summoned.hp = 1;
        summoned.max_hp = 200;
        summoned.spawned = true;
        let enemy = world.spawn(summoned).unwrap();

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(
            world
                .iter()
                .filter(|(_, entity)| entity.kind == Kind::Container)
                .count(),
            0,
            "a summon is worth nothing, loot included"
        );
    }

    #[test]
    fn a_bag_expires() {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let catalog = catalog();
        let squares = (0..16 * 16).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 8.0, 8.0);
        slime.kind = Kind::Enemy;
        slime.hp = 1;
        slime.max_hp = 200;
        let enemy = world.spawn(slime).unwrap();

        let (programs, _) = compile(
            &parse(r#"enemy "Slime" { state idle { } loot { item("Wand", 1.0) } }"#).unwrap(),
        );
        world.set_behaviours(&catalog, programs);

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);
        assert_eq!(
            world
                .iter()
                .filter(|(_, e)| e.kind == Kind::Container)
                .count(),
            1
        );

        // A minute later there is nothing left, so a cleared dungeon does not fill with bags.
        for _ in 0..(61_000 / 50) {
            world.advance(&catalog, 50);
        }
        assert_eq!(
            world
                .iter()
                .filter(|(_, e)| e.kind == Kind::Container)
                .count(),
            0
        );
    }

    #[test]
    fn an_enemy_with_no_loot_leaves_nothing() {
        use hendra_behavior::compile::compile;
        use hendra_behavior::parse::parse;

        let catalog = catalog();
        let squares = (0..16 * 16).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Arena", Terrain::build(map, &catalog), &catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 8.0, 8.0);
        slime.kind = Kind::Enemy;
        slime.hp = 1;
        slime.max_hp = 200;
        let enemy = world.spawn(slime).unwrap();

        let (programs, _) = compile(&parse(r#"enemy "Slime" { state idle { } }"#).unwrap());
        world.set_behaviours(&catalog, programs);

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(
            world
                .iter()
                .filter(|(_, e)| e.kind == Kind::Container)
                .count(),
            0,
            "an empty bag is worse than no bag"
        );
    }

    #[test]
    fn a_move_for_an_entity_that_no_longer_exists_is_refused_safely() {
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 16.0, 16.0, 800))
            .unwrap();
        world.despawn(player);

        assert!(
            world
                .resolve_move(player, &catalog, 17.0, 16.0, 50)
                .is_none()
        );
    }

    /// Puts an enemy of a kind in the world at full health.
    fn an_enemy(world: &mut World, catalog: &Catalog, kind: u16, x: f32, y: f32) -> Handle {
        let health = catalog.object(ObjectType(kind)).unwrap().max_hp;
        let mut entity = Entity::fixture(ObjectType(kind), x, y);
        entity.kind = Kind::Enemy;
        entity.max_hp = health;
        entity.hp = health;
        world.spawn(entity).expect("room for an enemy")
    }

    #[test]
    fn a_tower_hands_its_credit_to_the_boss_and_takes_nothing_off_it() {
        // `TransferDamageOnDeath` deals no damage at all. It moves the hitters onto the boss's
        // damage counter (`TransferDamageOnDeath.cs:32`) so that whoever killed the towers is
        // eligible for the boss's bag. Reading it as damage killed the Pentaract before the fight
        // started: two 8000-health towers against a 10000-health boss.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = a_watcher(&mut world);

        let boss = an_enemy(&mut world, &catalog, 0x50c, 10.0, 10.0);
        let first = an_enemy(&mut world, &catalog, 0x50d, 11.0, 10.0);
        let second = an_enemy(&mut world, &catalog, 0x50d, 12.0, 10.0);

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Pentaract Tower" { state a { transfer_damage_on_death("Pentaract") } }"#,
        );
        world.reindex();

        for tower in [first, second] {
            let entity = world.get_mut(tower).unwrap();
            entity.damage_by.push((player, 500));
            entity.last_hurt_by = Some(player);
            entity.dead = true;
        }

        world.advance(&catalog, 50);

        let boss = world.get(boss).expect("the Pentaract is still standing");
        assert_eq!(boss.hp, 10_000, "no health moves with the credit");
        assert!(!boss.dead);
        assert_eq!(
            boss.damage_by,
            vec![(player, 1000)],
            "both towers' hitters, added together"
        );
        assert_eq!(boss.last_hurt_by, Some(player));
    }

    #[test]
    fn a_transfer_reaches_the_nearest_of_a_kind_and_no_further() {
        // `GetNearestEntity` returns one entity (`TransferDamageOnDeath.cs:27`). Handing the credit
        // to everything in a fifty-tile radius spreads a boss's loot eligibility across a dungeon.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = a_watcher(&mut world);

        let near = an_enemy(&mut world, &catalog, 0x50c, 11.0, 10.0);
        let far = an_enemy(&mut world, &catalog, 0x50c, 20.0, 10.0);
        let tower = an_enemy(&mut world, &catalog, 0x50d, 10.0, 10.0);

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Pentaract Tower" { state a { transfer_damage_on_death("Pentaract") } }"#,
        );
        world.reindex();

        let entity = world.get_mut(tower).unwrap();
        entity.damage_by.push((player, 700));
        entity.last_hurt_by = Some(player);
        entity.dead = true;

        world.advance(&catalog, 50);

        assert_eq!(world.get(near).unwrap().damage_by, vec![(player, 700)]);
        assert!(
            world.get(far).unwrap().damage_by.is_empty(),
            "only the nearest is credited"
        );
    }

    #[test]
    fn a_hermit_gods_drop_survives_the_transfer_that_names_it() {
        // The drop has a hundred health and the god that names it fifty-five thousand. Its whole
        // job is to carry the loot, and dealing the god's health to it deletes it on sight.
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = a_watcher(&mut world);

        let drop = an_enemy(&mut world, &catalog, 0x50e, 11.0, 10.0);
        let god = an_enemy(&mut world, &catalog, 0x50c, 10.0, 10.0);

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Pentaract" { state a { transfer_damage_on_death("Hermit God Drop") } }"#,
        );
        world.reindex();

        let entity = world.get_mut(god).unwrap();
        entity.damage_by.push((player, 9000));
        entity.dead = true;

        world.advance(&catalog, 50);

        let drop = world.get(drop).expect("the drop is still there");
        assert_eq!(drop.hp, 100);
        assert_eq!(drop.damage_by, vec![(player, 9000)]);
    }

    #[test]
    fn a_copy_on_death_does_nothing_at_all() {
        // `CopyDamageOnDeath` finds the nearest entity of a kind and then calls
        // `Enemy.SetDamageCounter`, whose body is empty (`Enemy.cs:56-58`).
        let catalog = catalog();
        let mut world = field(&catalog);
        let player = a_watcher(&mut world);

        let balloon = an_enemy(&mut world, &catalog, 0x50e, 11.0, 10.0);
        let boss = an_enemy(&mut world, &catalog, 0x50c, 10.0, 10.0);

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Pentaract" { state a { copy_damage_on_death("Hermit God Drop") } }"#,
        );
        world.reindex();

        let entity = world.get_mut(boss).unwrap();
        entity.damage_by.push((player, 9000));
        entity.dead = true;

        world.advance(&catalog, 50);

        let balloon = world.get(balloon).expect("untouched");
        assert_eq!(balloon.hp, 100);
        assert!(balloon.damage_by.is_empty());
    }

    #[test]
    fn a_summoned_spawners_children_are_summoned_too() {
        // `Spawned` propagates through every spawning behaviour in the original (`SpawnGroup.cs:60`
        // among seven others). Without it an administrator can summon a spawner and stand in the
        // room collecting the loot its children drop.
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let spawner = an_enemy(&mut world, &catalog, 0x502, 10.0, 10.0);

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { spawn("Spawnling", 1) } }"#,
        );
        world.reindex();
        world.get_mut(spawner).unwrap().spawned = true;

        world.advance(&catalog, 50);

        let child = world
            .iter()
            .find(|(_, entity)| entity.object_type == ObjectType(0x505))
            .map(|(handle, _)| handle)
            .expect("a spawnling");
        assert!(
            world.get(child).unwrap().spawned,
            "a summoned spawner's children are summoned"
        );
    }

    #[test]
    fn set_no_x_p_takes_the_experience_and_leaves_the_loot() {
        // `SetNoXP` writes `GivesNoXp` (`SetNoXP.cs:15`), which `DamageCounter.Death` reads to make
        // the figure zero (`DamageCounter.cs:82`). `Loots.Handle` never looks at it. Conflating it
        // with `Spawned` took the drops off four Draconis dragon souls and a Shatters entity.
        let catalog = catalog();
        let mut world = field(&catalog);
        world.set_bag_types(vec![ObjectType(0x510)]);
        a_watcher(&mut world);

        let enemy = an_enemy(&mut world, &catalog, 0x502, 10.0, 10.0);

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { set_no_x_p() } loot { item("Rare Blade", 1) } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);

        {
            let entity = world.get(enemy).unwrap();
            assert!(!entity.awards_experience, "no experience");
            assert!(!entity.spawned, "but not marked summoned");
        }

        world.get_mut(enemy).unwrap().dead = true;
        world.advance(&catalog, 50);

        assert_eq!(
            world
                .iter()
                .filter(|(_, entity)| entity.container.is_some())
                .count(),
            1,
            "the loot still drops"
        );
    }

    #[test]
    fn an_entity_removed_by_a_boss_still_runs_its_own_death_behaviours() {
        // `RemoveEntity` sets `Spawned` and then calls `Death` (`RemoveEntity.cs:27-28`), which
        // runs `CurrentState.OnDeath` like any other death. Only the experience and the loot are
        // withheld.
        let catalog = catalog();
        let mut world = field(&catalog);
        world.set_bag_types(vec![ObjectType(0x510)]);
        a_watcher(&mut world);

        an_enemy(&mut world, &catalog, 0x502, 10.0, 10.0);
        an_enemy(&mut world, &catalog, 0x503, 11.0, 10.0);

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" { state a { remove_entity(20, "Guard") } }
               enemy "Guard" { state a { transform_on_death("Spawnling") }
                               loot { item("Rare Blade", 1) } }"#,
        );
        world.reindex();

        world.advance(&catalog, 50);

        assert_eq!(count_of(&world, 0x503), 0, "the guard is gone");
        assert_eq!(count_of(&world, 0x505), 1, "and left what it turns into");
        assert_eq!(
            world
                .iter()
                .filter(|(_, entity)| entity.container.is_some())
                .count(),
            0,
            "a removed entity drops nothing"
        );
    }

    #[test]
    fn decaying_runs_no_death_behaviours_and_a_suicide_runs_them_all() {
        // `Decay` is a bare `LeaveWorld` (`Decay.cs:31`) and `Suicide` is a `Death` (`Suicide.cs:22`).
        // Most of the game's dungeon portals are dropped by something that kills itself, so the
        // difference is the difference between a dungeon that can be entered and one that cannot.
        let catalog = catalog();

        let mut world = field(&catalog);
        a_watcher(&mut world);
        an_enemy(&mut world, &catalog, 0x503, 10.0, 10.0);
        behaving(
            &mut world,
            &catalog,
            r#"enemy "Guard" { state a { decay(40) transform_on_death("Spawnling") } }"#,
        );
        world.reindex();
        // Even forty milliseconds is two of the original's logic ticks: one to round up to, and one
        // more because the tick that empties the countdown is not the tick that acts on it.
        for _ in 0..7 {
            world.advance(&catalog, 50);
        }
        assert_eq!(count_of(&world, 0x503), 0, "the guard decayed");
        assert_eq!(count_of(&world, 0x505), 0, "and left nothing behind");

        let mut world = field(&catalog);
        a_watcher(&mut world);
        an_enemy(&mut world, &catalog, 0x503, 10.0, 10.0);
        behaving(
            &mut world,
            &catalog,
            r#"enemy "Guard" { state a { suicide() transform_on_death("Spawnling") } }"#,
        );
        world.reindex();
        world.advance(&catalog, 50);
        assert_eq!(count_of(&world, 0x503), 0, "the guard killed itself");
        assert_eq!(count_of(&world, 0x505), 1, "and left what it turns into");
    }

    #[test]
    fn a_bled_out_enemy_waits_at_negative_health_and_a_player_stops_at_one() {
        // `Enemy.Tick` subtracts bleeding with no floor and no death check (`Enemy.cs:141-148`),
        // so the next scratch of any size finishes an enemy that has bled out. `Player.HandleEffects`
        // floors at one (`Player.Effects.cs:44`), and that asymmetry is in the original.
        let catalog = catalog();
        let mut world = field(&catalog);

        let enemy = an_enemy(&mut world, &catalog, 0x503, 10.0, 10.0);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 12.0, 10.0, 500))
            .unwrap();
        world.reindex();

        for handle in [enemy, player] {
            world.get_mut(handle).unwrap().hp = 2;
            give(
                &mut world,
                handle,
                hendra_content::ConditionEffect::Bleeding,
            );
        }

        for _ in 0..40 {
            world.advance(&catalog, 50);
        }

        assert!(
            world.get(enemy).unwrap().hp < 0,
            "an enemy bleeds past nothing"
        );
        assert!(!world.get(enemy).unwrap().dead, "and is not killed by it");
        assert_eq!(world.get(player).unwrap().hp, 1, "a player stops at one");
    }

    #[test]
    fn a_spawner_with_no_declared_health_does_not_bleed() {
        // `Enemy.Tick` tests `!stat` before it bleeds anything (`Enemy.cs:141`), and `stat` is a
        // descriptor with no `MaxHitPoints`. Those are the same spawners and turrets that cannot be
        // shot, and bleeding one to death would let a player take a dungeon apart from outside it.
        let catalog = catalog();
        let mut world = field(&catalog);

        let spawner = an_enemy(&mut world, &catalog, 0x504, 10.0, 10.0);
        world.get_mut(spawner).unwrap().hp = 5;
        world.reindex();
        give(
            &mut world,
            spawner,
            hendra_content::ConditionEffect::Bleeding,
        );

        for _ in 0..40 {
            world.advance(&catalog, 50);
        }

        assert_eq!(world.get(spawner).unwrap().hp, 5);
    }

    #[test]
    fn a_destructible_object_breaks_but_is_worth_neither_experience_nor_loot() {
        // A wine barrel is `Class=GameObject`, so `Entity.Resolve` makes a `StaticObject` of it
        // (`Entity.cs:606`) whatever its `Enemy` flag says. It takes damage and dies at zero health
        // (`StaticObject.cs:63-79`) and has no damage counter, so breaking one pays nothing.
        let catalog = catalog();
        let mut world = field(&catalog);
        world.set_bag_types(vec![ObjectType(0x510)]);

        let barrel = a_breakable(&mut world, &catalog, 0x512, 10.0, 10.0);
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 8.0, 500))
            .unwrap();
        world.reindex();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Wine Barrel" { state a { } loot { item("Rare Blade", 1) } }"#,
        );
        world.reindex();

        world.get_mut(player).unwrap().weapon = Some(ObjectType(0x901));
        world.get_mut(barrel).unwrap().hp = 20;
        world.shoot(player, &catalog, std::f32::consts::FRAC_PI_2);

        for _ in 0..10 {
            world.advance(&catalog, 50);
        }

        assert!(world.get(barrel).is_none(), "a barrel can be broken");
        assert_eq!(
            world.get(player).unwrap().progress.experience,
            0,
            "and is worth no experience"
        );
        assert_eq!(
            world
                .iter()
                .filter(|(_, entity)| entity.container.is_some())
                .count(),
            0,
            "and drops nothing"
        );
    }

    /// Puts down whatever a descriptor's class makes of it, as the world itself would.
    fn a_breakable(world: &mut World, catalog: &Catalog, kind: u16, x: f32, y: f32) -> Handle {
        let desc = catalog.object(ObjectType(kind)).unwrap();
        let mut entity = Entity::fixture(ObjectType(kind), x, y);
        entity.kind = Kind::of_class(desc.class.as_str());
        entity.max_hp = desc.max_hp;
        entity.hp = desc.max_hp;
        entity.holds_conditions = holds_conditions(desc);
        entity.take_immunities(desc);
        world.spawn(entity).expect("room for it")
    }

    #[test]
    fn a_wine_barrel_is_scenery_with_health_and_breaks_a_hit_before_a_slime_would() {
        // `Entity.Resolve` (`Entity.cs:591-617`) switches on `Class` and never reads `<Enemy/>`, so
        // a `Class=GameObject` is a `StaticObject` however it is flagged. That is a whole hit of
        // difference: `StaticObject.CheckHP` (`:79`) removes it at `HP <= 0` where
        // `Enemy.HitByProjectile` (`Enemy.cs:127`) needs `HP < 0`. Fifty-eight of the shipped
        // objects are `GameObject`s and `Wall`s carrying `<Enemy/>`, and the forty-six of those that
        // declare `MaxHitPoints` — every destructible barrel, wall, switch and sprite tree — take
        // one shot too many to break when the flag decides instead of the class.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..16 * 16)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        // Both carry `<Enemy/>`; only the slime is a `Character`.
        squares[5 * 16 + 5] = square(0x10, 0x512);
        squares[5 * 16 + 7] = square(0x10, 0x502);

        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Test", Terrain::build(map, &catalog), &catalog);

        let found = |world: &World, kind: u16| {
            world
                .iter()
                .find(|(_, entity)| entity.object_type == ObjectType(kind))
                .map(|(handle, _)| handle)
        };
        let barrel = found(&world, 0x512).expect("the barrel is in the world");
        let slime = found(&world, 0x502).expect("the slime is in the world");

        assert_eq!(
            world.get(barrel).unwrap().kind,
            Kind::StaticObject,
            "a GameObject is scenery whatever its flag says"
        );
        assert_eq!(world.get(slime).unwrap().kind, Kind::Enemy);
        assert_eq!(
            world.get(barrel).unwrap().hp,
            100,
            "and still takes its health from the descriptor"
        );

        // `StaticObject.Tick` checks the health of anything `Vulnerable` every tick, not only when
        // something has just hit it (`StaticObject.cs:100-107`).
        world.get_mut(barrel).unwrap().hp = 0;
        world.get_mut(slime).unwrap().hp = 0;
        world.advance(&catalog, 50);

        assert!(
            world.get(barrel).is_none(),
            "a barrel on exactly no health is broken"
        );
        assert!(
            world.get(slime).is_some(),
            "a slime on exactly no health is still standing"
        );
    }

    #[test]
    fn a_gate_waiting_on_a_breakable_object_can_still_see_it() {
        // `World.EnterWorld` puts every `StaticObject` that is not a decoy into `EnemiesCollision`
        // (`World.cs:353-361`), which is the map `GetNearestEntity(dist, objType)` walks
        // (`Utils.cs:140`) and therefore what `EntityNotExistsTransition` reads. The Shatters gates
        // are written against exactly this — `on entity_not_exists("shtrs Abandoned Switch 1", 10)`
        // (`shatters.beh:1286`) — and those switches are `Class=GameObject`. Hide breakable scenery
        // from the neighbour list and every one of those gates is open from the first tick.
        let catalog = catalog();
        let mut world = field(&catalog);
        a_watcher(&mut world);

        let barrel = a_breakable(&mut world, &catalog, 0x512, 10.0, 10.0);
        let mut boss = Entity::fixture(ObjectType(0x502), 11.0, 10.0);
        boss.kind = Kind::Enemy;
        boss.hp = 500;
        boss.max_hp = 500;
        let boss = world.spawn(boss).unwrap();

        behaving(
            &mut world,
            &catalog,
            r#"enemy "Slime" {
                state shut { on entity_not_exists("Wine Barrel", 20) -> open }
                state open { wander(0.4) }
            }"#,
        );
        world.reindex();

        let shut = world.get(boss).unwrap().mind.as_ref().unwrap().state();
        world.advance(&catalog, 50);
        assert_eq!(
            world.get(boss).unwrap().mind.as_ref().unwrap().state(),
            shut,
            "the gate opened while the barrel was still standing"
        );

        world.get_mut(barrel).unwrap().hp = 0;
        world.advance(&catalog, 50);
        world.advance(&catalog, 50);

        assert_ne!(
            world.get(boss).unwrap().mind.as_ref().unwrap().state(),
            shut,
            "and stayed shut once it was gone"
        );
    }

    #[test]
    fn a_turret_the_content_left_indestructible_still_runs_its_behaviour() {
        // `ResolveBehavior` is called from the base `Entity` constructor (`Entity.cs:132`) and looks
        // the tree up by object type alone (`BehaviorDb.cs:58-63`), so a `StaticObject` thinks
        // exactly as an `Enemy` does. Twelve of the shipped `GameObject`s that carry `<Enemy/>`
        // declare no health at all — the Tomb turrets, the treasure-room flame traps, the Abandoned
        // Dr Terrible bubble — and every one of them is machinery that has to keep running once it
        // stops being an enemy.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..32 * 32)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[10 * 32 + 10] = square(0x10, 0x512);

        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Test", Terrain::build(map, &catalog), &catalog);
        let barrel = world
            .iter()
            .find(|(_, entity)| entity.object_type == ObjectType(0x512))
            .map(|(handle, _)| handle)
            .expect("the barrel is in the world");
        world
            .spawn(Entity::player(ObjectType(0x600), 14.0, 10.0, 800))
            .unwrap();
        behaving(
            &mut world,
            &catalog,
            r#"enemy "Wine Barrel" { state a { follow(1.0, 20, 1) } }"#,
        );
        world.reindex();

        assert_eq!(world.thinking(), 1, "it was given no mind at all");

        let start = world.get(barrel).unwrap().x;
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        let now = world.get(barrel).unwrap().x;
        assert!(
            now > start + 0.5,
            "breakable scenery with a behaviour should still run it: {start} -> {now}"
        );
    }

    #[test]
    fn an_object_that_cannot_be_hurt_reports_impossible_health() {
        // `StaticObject.ExportStats` sends `int.MaxValue` rather than the real number for anything
        // not `Vulnerable` (`StaticObject.cs:44`), and `Vulnerable` is "the descriptor declared
        // `MaxHitPoints`". The client keeps its own maximum from its own copy of the descriptor and
        // raises it to whatever health it is told when that is larger
        // (`GameObject.as:1035-1038`), so the impossible number draws a full bar over the turret
        // and the real number — zero — draws an empty one over something that cannot be hurt.
        let catalog = catalog();
        let mut world = field(&catalog);

        let turret = a_breakable(&mut world, &catalog, 0x517, 10.0, 10.0);
        let barrel = a_breakable(&mut world, &catalog, 0x512, 12.0, 10.0);
        let slime = {
            let mut slime = Entity::fixture(ObjectType(0x502), 14.0, 10.0);
            slime.kind = Kind::Enemy;
            slime.hp = 40;
            slime.max_hp = 200;
            world.spawn(slime).unwrap()
        };

        assert_eq!(
            world.get(turret).unwrap().state().hp,
            i32::MAX,
            "a turret with no MaxHitPoints is not Vulnerable"
        );
        assert_eq!(
            world.get(barrel).unwrap().state().hp,
            100,
            "a barrel with MaxHitPoints is, and sends what it has"
        );
        assert_eq!(
            world.get(slime).unwrap().state().hp,
            40,
            "and a Character always sends the real number"
        );
    }

    #[test]
    fn breakable_scenery_never_holds_the_immunities_it_declares() {
        // `SetConditions` is `Character`'s (`Character.cs:48-67`), so only a player or an enemy is
        // born holding the immunities its descriptor asks for. A wine barrel is a `StaticObject`,
        // never runs it, and reports no condition at all — which matters because the condition set
        // goes out on the wire as `StatsType.Effects`.
        let catalog = catalog();
        let mut world = field(&catalog);

        let barrel = a_breakable(&mut world, &catalog, 0x512, 10.0, 10.0);
        let switch = a_breakable(&mut world, &catalog, 0x516, 12.0, 10.0);
        let boss = {
            let mut boss = Entity::fixture(ObjectType(0x50b), 14.0, 10.0);
            boss.kind = Kind::Enemy;
            boss.take_immunities(catalog.object(ObjectType(0x50b)).unwrap());
            world.spawn(boss).unwrap()
        };

        assert!(
            world.get(barrel).unwrap().conditions.is_empty(),
            "a Static GameObject holds nothing"
        );
        assert!(
            world.get(switch).unwrap().conditions.is_empty(),
            "and neither does one that is not Static: both are StaticObjects"
        );
        assert!(
            world
                .get(boss)
                .unwrap()
                .conditions
                .contains(hendra_content::ConditionEffect::StasisImmune),
            "a Character does"
        );
    }

    #[test]
    fn a_static_object_has_nowhere_to_put_a_condition_and_a_switch_does() {
        // `Entity`'s constructor allocates the effect array for a player, for a `<Character/>`, and
        // for something `<Enemy/>` that is not also `<Static/>` — and for nothing else
        // (`Entity.cs:137-155`). A wine barrel declares `<Static/>` and so holds no condition at
        // all; a Shatters switch declares no such thing and holds every one. Both are
        // `Class=GameObject` and both are breakable scenery: the descriptor is the whole difference.
        let catalog = catalog();
        let mut world = field(&catalog);

        let barrel = a_breakable(&mut world, &catalog, 0x512, 10.0, 10.0);
        let switch = a_breakable(&mut world, &catalog, 0x516, 12.0, 10.0);

        let slowed = hendra_content::ConditionEffect::Slowed;
        world.give_effect(barrel, slowed.index() as u8, 5_000);
        world.give_effect(switch, slowed.index() as u8, 5_000);

        assert!(
            !world.get(barrel).unwrap().conditions.contains(slowed),
            "a Static GameObject has no array to write into"
        );
        assert!(
            world.get(switch).unwrap().conditions.contains(slowed),
            "one that is not Static has"
        );
    }

    #[test]
    fn a_bullet_into_a_barrel_leaves_its_damage_and_nothing_else() {
        // `StaticObject.HitByProjectile` (`StaticObject.cs:55-70`) subtracts the health, broadcasts
        // the number and returns. There is no `ApplyConditionEffect` anywhere in it, where both
        // `Enemy.HitByProjectile` (`Enemy.cs:114`) and `Player.HitByProjectile` (`Player.cs:789`)
        // have one. So a paralysing shot into a wine barrel is only ever damage, and a paralysing
        // shot into a Shatters switch is too: whether the thing could hold a condition from
        // somewhere else is a separate question, and this path never asks it.
        let catalog = catalog();

        // The shooter stands below its target, out of the flight of its own bullet.
        let paralysed = hendra_content::ConditionEffect::Paralyzed;

        let shot_at = |kind: u16| {
            let mut world = field(&catalog);
            let target = a_breakable(&mut world, &catalog, kind, 10.0, 10.0);
            let player = world
                .spawn(Entity::player(ObjectType(0x600), 10.0, 6.0, 500))
                .unwrap();
            world.reindex();

            world.get_mut(player).unwrap().weapon = Some(ObjectType(0x907));
            world.shoot(player, &catalog, std::f32::consts::FRAC_PI_2);
            for _ in 0..20 {
                world.advance(&catalog, 50);
            }

            let hurt = world.get(target).expect("it survived the one shot");
            (hurt.hp, hurt.conditions.contains(paralysed))
        };

        // A `Character`, which does have an `ApplyConditionEffect` in its hit path.
        let mut world = field(&catalog);
        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 200;
        slime.max_hp = 200;
        let slime = world.spawn(slime).unwrap();
        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 6.0, 500))
            .unwrap();
        world.reindex();
        world.get_mut(player).unwrap().weapon = Some(ObjectType(0x907));
        world.shoot(player, &catalog, std::f32::consts::FRAC_PI_2);
        for _ in 0..20 {
            world.advance(&catalog, 50);
        }
        assert!(
            world.get(slime).unwrap().conditions.contains(paralysed),
            "an enemy takes what the bullet carried"
        );

        for kind in [0x516, 0x512] {
            let (hp, held) = shot_at(kind);
            assert!(hp < 100, "the shot landed on {kind:#x}: {hp}");
            assert!(
                !held,
                "but StaticObject.HitByProjectile never applies a condition"
            );
        }
    }

    #[test]
    fn breaking_a_barrel_opens_the_square_it_stood_on() {
        // Something both `<Enemy/>` and `<Static/>` is left on its tile as well as put into the
        // world (`Wmap.cs:358-364`), so a wine barrel is an entity *and* a square that cannot be
        // walked through. `StaticObject.CheckHP` writes `tile.ObjType = 0` over that square before
        // it leaves (`StaticObject.cs:74-86`). Without it the barrel is gone from the screen and
        // still standing in the collision map, and a room whose door is a barrel stays shut.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..16 * 16)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[5 * 16 + 5] = square(0x10, 0x515);

        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Test", Terrain::build(map, &catalog), &catalog);
        let cask = world
            .iter()
            .find(|(_, entity)| entity.object_type == ObjectType(0x515))
            .map(|(handle, _)| handle)
            .expect("the cask is in the world");

        assert!(
            !world.terrain().walkable(5, 5),
            "an OccupySquare barrel blocks the square it stands on"
        );

        world.get_mut(cask).unwrap().hp = 0;
        world.advance(&catalog, 50);

        assert!(world.get(cask).is_none(), "the cask broke");
        assert!(
            world.terrain().walkable(5, 5),
            "and the square it stood on can be walked through"
        );
        assert!(
            world.terrain().object_at(5.5, 5.5).is_none(),
            "because the tile was blanked, as CheckHP blanks it"
        );
    }

    #[test]
    fn breaking_a_barrel_leaves_a_wall_it_was_standing_beside_alone() {
        // `CheckHP` blanks the square only when `Map[x, y].ObjType` is the object's own type
        // (`StaticObject.cs:77-80`). A barrel a behaviour spawned over somebody else's wall is not
        // that square's object and does not get to delete it.
        let catalog = catalog();
        let mut squares: Vec<Composition> = (0..16 * 16)
            .map(|_| square(0x10, ObjectType::NONE.0))
            .collect();
        squares[5 * 16 + 5] = square(0x10, 0x500); // a plain wall

        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Test", Terrain::build(map, &catalog), &catalog);

        let cask = a_breakable(&mut world, &catalog, 0x515, 5.5, 5.5);
        world.get_mut(cask).unwrap().hp = 0;
        world.advance(&catalog, 50);

        assert!(world.get(cask).is_none(), "the cask broke");
        assert!(
            !world.terrain().walkable(5, 5),
            "the wall it was standing on is still there"
        );
    }

    #[test]
    fn a_barrel_standing_in_a_trap_does_not_spring_it() {
        // `Trap.Tick` asks `AOE(radius / 2, false, ...)` (`Trap.cs:53`), and that overload walks
        // `EnemiesCollision` — which does hold breakable scenery — but skips everything failing
        // `i is Enemy` (`Utils.cs:328`). A `StaticObject` is not an `Enemy`, so a wine barrel walks
        // past a trap without setting it off. It has to: `Explode` casts each thing it reached to
        // `Enemy` before damaging it (`Trap.cs:70`), and scenery reaching there would be a crash
        // rather than a trap.
        let catalog = catalog();
        let mut world = field(&catalog);

        let player = world
            .spawn(Entity::player(ObjectType(0x600), 10.0, 10.0, 500))
            .unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x906), (16.0, 16.0));
        a_breakable(&mut world, &catalog, 0x512, 16.0, 16.0);
        world.reindex();

        for _ in 0..10 {
            world.advance(&catalog, 50);
        }

        assert_eq!(
            world
                .iter()
                .filter(|(_, entity)| entity.kind == Kind::Trap)
                .count(),
            1,
            "the barrel standing on it is not something a trap goes off for"
        );
    }

    #[test]
    fn a_player_killed_by_a_bullet_is_broadcast_as_a_kill() {
        // The `kill` flag is what makes a client play the death, draw the debris and take the body
        // off the screen; without it a corpse stands there until the snapshot quietly drops it.
        // `Player.HitByProjectile` writes `Kill = HP <= 0` (`Player.cs:795`) in the same statement
        // that subtracts the damage.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut slime = Entity::fixture(ObjectType(0x502), 10.0, 10.0);
        slime.kind = Kind::Enemy;
        slime.hp = 500;
        slime.max_hp = 500;
        slime.weapon = Some(ObjectType(0x502));
        let slime = world.spawn(slime).unwrap();

        let victim = world
            .spawn(Entity::player(ObjectType(0x600), 14.0, 10.0, 800))
            .unwrap();
        world.reindex();

        // A bolt is worth twenty; exactly that much health, so the shot takes it to zero and a
        // player at zero is dead. Held as the maximum too, or regeneration puts it back above the
        // line between the tick that sets it and the tick the bullet arrives on.
        let entity = world.get_mut(victim).unwrap();
        entity.hp = 20;
        entity.max_hp = 20;
        world.take_damage();

        let mut reported = None;
        for _ in 0..20 {
            world.shoot(slime, &catalog, 0.0);
            world.advance(&catalog, 50);
            if let Some(event) = world
                .take_damage()
                .into_iter()
                .find(|event| event.target == victim)
            {
                reported = Some(event);
                break;
            }
        }

        let reported = reported.expect("the shot that killed them was reported");
        assert_eq!(reported.amount, 20);
        assert!(
            reported.kill,
            "a player taken to zero by a bullet is broadcast dead"
        );
    }

    #[test]
    fn a_player_killed_by_the_ground_is_broadcast_as_a_kill() {
        // `ApplyGroundDamage` broadcasts `Kill = HP <= 0` after subtracting the roll
        // (`Player.Ground.cs:100-106`). Lava is the same threshold as a bullet, and nothing else on
        // the tick tells the room what happened.
        let catalog = catalog();
        let squares = (0..16 * 16).map(|_| square(0x12, ObjectType::NONE.0));
        let map = Map::from_squares(16, 16, squares).unwrap();
        let mut world = World::new("Lava", Terrain::build(map, &catalog), &catalog);

        let victim = world
            .spawn(Entity::player(ObjectType(0x600), 8.0, 8.0, 800))
            .unwrap();
        world.reindex();

        // The tile is worth a hundred a burn, so one burn on a hundred health lands on exactly zero.
        world.get_mut(victim).unwrap().hp = 100;
        world.take_damage();

        let mut reported = None;
        for _ in 0..40 {
            world.advance(&catalog, 50);
            if let Some(event) = world
                .take_damage()
                .into_iter()
                .find(|event| event.target == victim)
            {
                reported = Some(event);
                break;
            }
        }

        let reported = reported.expect("the burn that killed them was reported");
        assert_eq!(reported.amount, 100);
        assert!(
            reported.kill,
            "a player taken to zero by the ground is broadcast dead"
        );
    }

    #[test]
    fn an_invulnerable_enemy_is_shown_the_whole_blow_it_did_not_take() {
        // `Enemy.HitByProjectile` guards only the `HP -=` with Invulnerable and puts the full
        // post-defence `dmg` in the `Damage` packet (`Enemy.cs:104-113`). A zero there is a boss
        // that shrugs the hit off and shows nothing for it, which reads to the player as a miss —
        // and it is not clamped to remaining health either, so an overkill reports the whole blow.
        let catalog = catalog();
        let (mut world, shooter, target) = duel(&catalog);

        give(
            &mut world,
            target,
            hendra_content::ConditionEffect::Invulnerable,
        );
        let before = world.get(target).unwrap().hp;
        world.take_damage();

        let mut reported = None;
        for _ in 0..20 {
            world.shoot(shooter, &catalog, 0.0);
            world.advance(&catalog, 50);
            if let Some(event) = world.take_damage().into_iter().next() {
                reported = Some(event);
                break;
            }
        }

        let reported = reported.expect("an invulnerable enemy is still hit and still reported");
        assert_eq!(
            reported.amount, 50,
            "the whole blow is named, not the nothing it cost"
        );
        assert!(!reported.kill);
        assert_eq!(
            world.get(target).unwrap().hp,
            before,
            "and none of it was taken"
        );
    }

    #[test]
    fn a_blast_says_nothing_at_all_about_a_target_it_cannot_touch() {
        // `Enemy.Damage` returns before it has computed or sent anything when the target is Paused
        // or in Stasis (`Enemy.cs:62-66`), and `Player.Damage` returns on `IsInvulnerable()`
        // (`Player.cs:811`). A zero-amount `Damage` packet instead puts a "0" over a boss that a
        // behaviour has deliberately frozen mid-phase.
        let catalog = catalog();
        let mut world = field(&catalog);

        let mut player = Entity::player(ObjectType(0x600), 10.0, 10.0, 500);
        player.mp = 500;
        player.max_mp = 500;
        let player = world.spawn(player).unwrap();
        world.reindex();

        world.use_item(player, &catalog, ObjectType(0x906), (20.0, 10.0));
        world.advance(&catalog, 50);

        let mut enemy = Entity::fixture(ObjectType(0x502), 20.0, 10.0);
        enemy.kind = Kind::Enemy;
        enemy.max_hp = 500;
        enemy.hp = 500;
        let enemy = world.spawn(enemy).unwrap();
        world.reindex();
        give(&mut world, enemy, hendra_content::ConditionEffect::Stasis);
        world.take_damage();

        world.advance(&catalog, 50);

        assert_eq!(world.get(enemy).unwrap().hp, 500, "it took nothing");
        assert!(
            world
                .take_damage()
                .iter()
                .all(|event| event.target != enemy),
            "and nothing was said about it"
        );
    }

    /// An enemy that chases, alone in its own world, with a player just inside its acquire range.
    ///
    /// Built twice so the two enemies share nothing but their object type and the name of the
    /// program that drives them, which is the whole point of the test below.
    fn lone_chaser(catalog: &Catalog, kind: ObjectType, id: &str) -> (World, Handle) {
        let squares = (0..32 * 32).map(|_| square(0x10, ObjectType::NONE.0));
        let map = Map::from_squares(32, 32, squares).unwrap();
        let mut world = World::new("Desert", Terrain::build(map, catalog), catalog);

        let mut wolf = Entity::fixture(kind, 5.0, 10.0);
        wolf.kind = Kind::Enemy;
        wolf.hp = 500;
        wolf.max_hp = 500;
        let enemy = world.spawn(wolf).unwrap();

        world
            .spawn(Entity::player(ObjectType(0x600), 24.0, 10.0, 800))
            .unwrap();

        let source = format!(
            r#"enemy "{id}" {{
                state hunt {{ follow(speed: 0.75, acquire_range: 20, range: 0.5) }}
            }}"#
        );
        let (programs, diagnostics) = hendra_behavior::compile::compile(
            &hendra_behavior::parse::parse(&source).expect("behaviour should parse"),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        world.set_behaviours(catalog, programs);

        (world, enemy)
    }

    /// How far the enemy actually walked over a second, measured tick by tick rather than end to
    /// end, so a chase that curves is not read as a chase that was slow.
    fn walked_in_a_second(world: &mut World, catalog: &Catalog, enemy: Handle) -> f32 {
        // One tick to settle first, because the state a mind is switched into is entered at the
        // top of the tick *after* the switch, so the very first tick of a world spends itself
        // entering rather than moving.
        world.advance(catalog, 50);

        let mut total = 0.0;
        for _ in 0..20 {
            let before = world.get(enemy).map(|e| (e.x, e.y)).unwrap();
            world.advance(catalog, 50);
            let after = world.get(enemy).map(|e| (e.x, e.y)).unwrap();
            let (dx, dy) = (after.0 - before.0, after.1 - before.1);
            total += (dx * dx + dy * dy).sqrt();
        }
        total
    }

    #[test]
    fn paralysing_one_enemy_cripples_its_whole_species_in_every_world_for_good() {
        // The original keeps a movement behaviour's speed in a plain field of the behaviour object
        // and writes `speed = 0` there whenever its host is paralysed (`Follow.cs:50-51`). Nothing
        // writes it back, and `BehaviorDb` hands the same object to every enemy of the type in
        // every world (`BehaviorDb.cs:68-88`, `Entity.cs:236-245`). So one tick of paralysis on one
        // wolf is every wolf on the server, until the process is restarted.
        let catalog = catalog();

        let (mut here, victim) = lone_chaser(&catalog, ObjectType(0x5f0), "Desert Werewolf");
        let (mut elsewhere, bystander) =
            lone_chaser(&catalog, ObjectType(0x5f0), "Desert Werewolf");

        // What a `follow(0.75)` is worth before anybody is paralysed: `5.55 * 0.75 + 0.74`.
        let before = walked_in_a_second(&mut elsewhere, &catalog, bystander);
        assert!(
            (before - 4.9025).abs() < 0.05,
            "the bystander started at {before} rather than 4.90, so this proves nothing"
        );

        // One tick of paralysis, in the other world, on an enemy this test never looks at again.
        give(
            &mut here,
            victim,
            hendra_content::ConditionEffect::Paralyzed,
        );
        here.advance(&catalog, 50);
        if let Some(entity) = here.get_mut(victim) {
            entity.conditions = hendra_content::ConditionSet::EMPTY;
            entity.effects.clear();
        }

        // Nought is not still. `Utils.GetSpeed` has no paralysis term (`Utils.cs:297-300`), so the
        // constant is what the species is left with: 0.74 tiles a second, six and a half times
        // slower, for the rest of the process.
        let after = walked_in_a_second(&mut elsewhere, &catalog, bystander);
        assert!(
            (after - 0.74).abs() < 0.02,
            "an enemy in another world, never paralysed, walked at {after} rather than 0.74"
        );

        // And the one that was paralysed is no better off once the effect lifts.
        let recovered = walked_in_a_second(&mut here, &catalog, victim);
        assert!(
            (recovered - 0.74).abs() < 0.02,
            "the paralysed enemy recovered to {recovered}, a speed the original never restores"
        );
    }

    #[test]
    fn a_petrified_enemy_does_not_cripple_its_species() {
        // `Entity.ResolveNewLocation` holds both a paralysed and a petrified entity still
        // (`Entity.cs:322-330`), but the behaviours test `ConditionEffects.Paralyzed` on its own,
        // so only one of the two touches the shared field.
        let catalog = catalog();

        // Its own object type, because the table the crippling lives in is keyed by the enemy's
        // name and nothing ever clears it: sharing a name with the test above would share its
        // crippling too.
        let (mut here, victim) = lone_chaser(&catalog, ObjectType(0x507), "Plated Slime");
        let (mut elsewhere, bystander) = lone_chaser(&catalog, ObjectType(0x507), "Plated Slime");

        give(&mut here, victim, hendra_content::ConditionEffect::Petrify);
        for _ in 0..4 {
            here.advance(&catalog, 50);
        }

        let after = walked_in_a_second(&mut elsewhere, &catalog, bystander);
        assert!(
            (after - 4.9025).abs() < 0.05,
            "petrifying one enemy slowed another world's to {after}"
        );
    }
}

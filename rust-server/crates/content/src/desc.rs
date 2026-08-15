//! Typed descriptors read off the content tree.
//!
//! These are the runtime shape of the game's content: what an object is, what an item does, what a
//! projectile carries, what a tile costs to walk on. They are plain data with no
//! behaviour attached. The simulation reads them, the importer writes them, and the baked format
//! is their serialised form.

use crate::effect::{AppliedEffect, ConditionSet};
use crate::xml::Node;

/// An object type, as it travels on the wire and indexes the catalog.
///
/// Types are dense enough over the used range that the catalog can index by them directly, and
/// small enough that every entity can carry one without thought.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectType(pub u16);

/// A ground tile type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileType(pub u16);

impl ObjectType {
    /// The absence of an item. The wire uses -1 and the catalog never holds it.
    pub const NONE: ObjectType = ObjectType(0xffff);

    /// Content that has not been numbered yet.
    ///
    /// Distinct from [`ObjectType::NONE`], which means "no object at all". This one means "an
    /// object whose number the catalog has not assigned", and it never survives loading.
    pub const UNASSIGNED: ObjectType = ObjectType(0xfffe);

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    pub fn is_assigned(self) -> bool {
        self != Self::UNASSIGNED
    }
}

impl TileType {
    /// Ground that has not been numbered yet.
    pub const UNASSIGNED: TileType = TileType(0xfffe);

    pub fn is_assigned(self) -> bool {
        self != Self::UNASSIGNED
    }
}

/// How big an object is, and whether the size varies per spawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeRange {
    pub min: i32,
    pub max: i32,
    pub step: i32,
}

impl Default for SizeRange {
    fn default() -> Self {
        SizeRange {
            min: 100,
            max: 100,
            step: 0,
        }
    }
}

impl SizeRange {
    pub fn is_fixed(self) -> bool {
        self.min == self.max
    }
}

/// One projectile an object or item can fire.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectileDesc {
    /// Index within the owner, matching the bullet type on the wire.
    pub bullet_type: u8,

    /// The object whose artwork and size the projectile borrows. Kept as a name here and resolved
    /// to an [`ObjectType`] once the whole catalog is loaded.
    pub object_id: String,
    pub object_type: ObjectType,

    pub lifetime_ms: i32,

    /// Tiles per 10,000 ms, the unit every trajectory calculation expects.
    pub speed: f32,

    /// `None` means "inherit the owner's size".
    pub size: Option<i32>,

    pub min_damage: i32,
    pub max_damage: i32,

    pub multi_hit: bool,
    pub passes_cover: bool,
    pub armor_piercing: bool,
    pub particle_trail: bool,
    pub wavy: bool,
    pub parametric: bool,
    pub boomerang: bool,

    pub amplitude: f32,
    pub frequency: f32,
    pub magnitude: f32,

    pub effects: Vec<AppliedEffect>,
}

impl ProjectileDesc {
    pub fn parse(node: &Node, fallback_index: u8) -> ProjectileDesc {
        // Damage is written either as one `<Damage>` or as a min/max pair.
        let (min_damage, max_damage) = match node.int("Damage") {
            Some(fixed) => (fixed as i32, fixed as i32),
            None => (
                node.int("MinDamage").unwrap_or(0) as i32,
                node.int("MaxDamage").unwrap_or(0) as i32,
            ),
        };

        ProjectileDesc {
            bullet_type: node
                .attr_int("id")
                .map(|id| id as u8)
                .unwrap_or(fallback_index),
            object_id: node.field("ObjectId").unwrap_or_default().to_owned(),
            object_type: ObjectType::NONE,
            lifetime_ms: node.int("LifetimeMS").unwrap_or(0) as i32,
            speed: node.float("Speed").unwrap_or(0.0) as f32,
            size: node.int("Size").map(|v| v as i32),
            min_damage,
            max_damage,
            multi_hit: node.has("MultiHit"),
            passes_cover: node.has("PassesCover"),
            armor_piercing: node.has("ArmorPiercing"),
            particle_trail: node.has("ParticleTrail"),
            wavy: node.has("Wavy"),
            parametric: node.has("Parametric"),
            boomerang: node.has("Boomerang"),
            amplitude: node.float("Amplitude").unwrap_or(0.0) as f32,
            frequency: node.float("Frequency").unwrap_or(1.0) as f32,
            magnitude: node.float("Magnitude").unwrap_or(3.0) as f32,
            effects: node
                .children_named("ConditionEffect")
                .filter_map(AppliedEffect::parse)
                .collect(),
        }
    }

    /// Damage for one shot, given a roll in `0.0..1.0`.
    ///
    /// Half-open, as both of the original's rolls are: `Shoot.cs` uses .NET's `Random.Next(min,
    /// max)` and `wRandom.NextIntRange` is `min + Gen() % (max - min)`, and neither can return the
    /// maximum. A 55-90 projectile rolls 55 to 89.
    pub fn roll_damage(&self, roll: f32) -> i32 {
        if self.max_damage <= self.min_damage {
            return self.min_damage;
        }
        let span = (self.max_damage - self.min_damage) as f32;
        self.min_damage + (roll * span) as i32
    }
}

/// An `<Activate>` or `<ActivateOnEquip>` entry.
///
/// There are around eighty distinct activation kinds and they share no common argument shape, so
/// the name and its attributes are kept verbatim and interpreted by whatever implements the
/// effect. That keeps an unimplemented activation a runtime no-op rather than a load failure, and
/// means adding one touches only the code that runs it.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivateDesc {
    pub name: String,
    pub args: Vec<(String, String)>,
}

impl ActivateDesc {
    pub fn parse(node: &Node) -> ActivateDesc {
        ActivateDesc {
            name: node.text.clone(),
            args: node.attrs.clone(),
        }
    }

    pub fn arg(&self, name: &str) -> Option<&str> {
        self.args
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.arg(name).and_then(crate::xml::parse_int)
    }

    pub fn float(&self, name: &str) -> Option<f64> {
        self.arg(name).and_then(crate::xml::parse_float)
    }

    pub fn flag(&self, name: &str) -> bool {
        matches!(self.arg(name), Some("true") | Some("1"))
    }
}

/// A stat granted by wearing an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatBoost {
    pub stat: u8,
    pub amount: i32,
}

/// The item half of an object: everything true of it while it sits in a slot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemDesc {
    pub slot_type: i32,
    pub tier: Option<i32>,
    pub bag_type: i32,
    pub fame_bonus: i32,
    pub feed_power: i32,

    pub rate_of_fire: f32,
    pub num_projectiles: i32,
    pub arc_gap: f32,

    pub mp_cost: i32,
    pub mp_end_cost: i32,
    pub cooldown: f32,
    pub doses: i32,

    pub usable: bool,
    pub consumable: bool,
    pub potion: bool,
    pub soulbound: bool,
    pub secret: bool,
    pub resurrects: bool,
    pub undead: bool,

    pub successor_id: Option<String>,

    /// The dye this item puts on, and which of the two layers it dyes.
    ///
    /// `Item.Texture1`/`Texture2` (`common/resources/XmlDescriptors.cs:530-531`, `:666-674`), read
    /// as hexadecimal. `AEDye` copies whichever is non-zero onto the player
    /// (`Player.UseItem.cs:583-589`), so a clothing dye declares the first and an accessory dye the
    /// second, and the layer a dye touches is decided by which element the content wrote.
    ///
    /// The top byte is a type and the low twenty-four its argument: `1` a solid `0xRRGGBB`, and
    /// `4`, `5`, `9` or `10` an index into the `textile{n}x{n}` sheet (`TextureRedrawer.as:161-186`).
    pub tex1: i32,
    pub tex2: i32,

    pub stat_boosts: Vec<StatBoost>,
    pub activate: Vec<ActivateDesc>,
    pub activate_on_equip: Vec<ActivateDesc>,
}

impl ItemDesc {
    fn parse(node: &Node) -> ItemDesc {
        let on_equip: Vec<ActivateDesc> = node
            .children_named("ActivateOnEquip")
            .map(ActivateDesc::parse)
            .collect();

        // A stat boost is spelled as an IncrementStat activation; lifting it out here means the
        // stat manager never has to walk the activation list.
        //
        // The written number is translated to a stat here rather than where the boost is applied,
        // so that nothing downstream ever holds a content number. Applying it at the point of use
        // is what let two call sites translate differently and neither notice.
        let stat_boosts = on_equip
            .iter()
            .filter(|a| a.name == "IncrementStat")
            .filter_map(|a| {
                Some(StatBoost {
                    stat: crate::player::Stat::from_content_number(a.int("stat")?)?.index() as u8,
                    amount: a.int("amount")? as i32,
                })
            })
            .collect();

        ItemDesc {
            slot_type: node.int("SlotType").unwrap_or(0) as i32,
            tier: node.int("Tier").map(|v| v as i32),
            bag_type: node.int("BagType").unwrap_or(0) as i32,
            fame_bonus: node.int("FameBonus").unwrap_or(0) as i32,
            feed_power: node.int("feedPower").unwrap_or(0) as i32,
            rate_of_fire: node.float("RateOfFire").unwrap_or(1.0) as f32,
            num_projectiles: node.int("NumProjectiles").unwrap_or(1) as i32,
            arc_gap: node.float("ArcGap").unwrap_or(11.25) as f32,
            mp_cost: node.int("MpCost").unwrap_or(0) as i32,
            mp_end_cost: node.int("MpEndCost").unwrap_or(0) as i32,
            cooldown: node.float("Cooldown").unwrap_or(0.5) as f32,
            doses: node.int("Doses").unwrap_or(0) as i32,
            usable: node.has("Usable"),
            consumable: node.has("Consumable"),
            potion: node.has("Potion"),
            soulbound: node.has("Soulbound"),
            secret: node.has("Secret"),
            resurrects: node.has("Resurrects"),
            undead: node.has("Undead"),
            successor_id: node.field("SuccessorId").map(str::to_owned),

            // Written with an `0x` prefix throughout the shipped content, which `int` already
            // reads as hexadecimal -- and the original reads them as hexadecimal whether or not
            // the prefix is there (`Convert.ToInt32(n.Value, 16)`).
            tex1: node.int("Tex1").unwrap_or(0) as i32,
            tex2: node.int("Tex2").unwrap_or(0) as i32,

            stat_boosts,
            activate: node
                .children_named("Activate")
                .map(ActivateDesc::parse)
                .collect(),
            activate_on_equip: on_equip,
        }
    }
}

/// How many of one enemy appear together when a realm places it.
///
/// The count is drawn from a normal distribution and clamped, so a group varies in size but never
/// becomes one lone straggler or a hundred at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnCount {
    pub mean: i32,
    pub std_dev: i32,
    pub min: i32,
    pub max: i32,
}

impl SpawnCount {
    fn parse(node: &crate::xml::Node) -> SpawnCount {
        SpawnCount {
            mean: node.int("Mean").unwrap_or(1) as i32,
            std_dev: node.int("StdDev").unwrap_or(0) as i32,
            min: node.int("Min").unwrap_or(1) as i32,
            max: node.int("Max").unwrap_or(1) as i32,
        }
    }

    /// The size of one group, given a standard normal sample.
    pub fn size(&self, normal: f32) -> usize {
        let drawn = self.mean as f32 + self.std_dev as f32 * normal;
        (drawn as i32)
            .clamp(self.min.min(self.max), self.max.max(self.min))
            .max(1) as usize
    }
}

/// Everything the simulation knows about one object type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectDesc {
    /// The runtime number, assigned at load. Not durable; see [`crate::identity`].
    pub object_type: ObjectType,

    /// What a saved inventory refers to. Survives renaming and renumbering.
    pub uuid: uuid::Uuid,

    pub id: String,
    pub display_id: Option<String>,
    pub dungeon_name: Option<String>,
    pub group: Option<String>,
    pub class: String,

    pub player: bool,
    pub character: bool,
    pub enemy: bool,
    pub god: bool,
    pub cube: bool,
    pub quest: bool,
    pub oryx: bool,
    pub hero: bool,

    pub occupy_square: bool,
    pub full_occupy: bool,
    pub enemy_occupy_square: bool,
    pub static_object: bool,
    pub blocks_sight: bool,
    pub flying: bool,
    pub connects: bool,

    pub protect_from_ground_damage: bool,
    pub protect_from_sink: bool,

    /// Whether everything this drops goes in a troll's white bag.
    ///
    /// `<TrollWhiteBag/>` (`common/resources/XmlDescriptors.cs:1042`), which `Loots.ShowBags` reads
    /// as a floor on the bag colour rather than as the colour itself (`Loots.cs:311`). Nothing in
    /// the shipped content sets it; it is carried because the loot pipeline reads it and a
    /// descriptor flag that silently does not exist is worse than one nobody uses.
    pub troll_white_bag: bool,

    pub max_hp: i32,
    pub defense: i32,
    pub level: Option<i32>,
    pub exp_multiplier: Option<f32>,
    pub size: SizeRange,

    /// The immunity markers this object is born holding, as `<StunImmune/>` and its seven siblings.
    ///
    /// The markers themselves, not the effects they block: `StunImmune` rather than `Stunned`.
    /// That is the encoding the original uses — `Character.SetConditions` (`Character.cs:48-67`)
    /// turns each flag into a permanent condition effect of the same name, and `Entity.ApplyCondition`
    /// (`Entity.cs:738-772`) then refuses `Stunned` to anything holding `StunImmune`. Storing the
    /// blocked effect instead would make a boss permanently stunned the moment this set were copied
    /// onto an entity.
    pub immunities: ConditionSet,

    pub terrain: Option<String>,
    pub spawn_probability: f32,
    pub per_realm_max: Option<i32>,

    /// How many of this appear at once when a realm places it. Absent means one.
    pub spawn_count: Option<SpawnCount>,

    /// What this sells to a guild, and what it costs.
    ///
    /// The hall upgrades are the only ones: three objects that raise a guild's hall a level each,
    /// with the price and the resulting hall named in the content rather than here.
    pub guild_item: Option<String>,
    pub guild_item_param: Option<String>,
    pub price: Option<i32>,

    pub projectiles: Vec<ProjectileDesc>,

    /// Present when the object can be held in a slot.
    pub item: Option<ItemDesc>,
}

impl Default for ObjectType {
    fn default() -> Self {
        ObjectType::NONE
    }
}

impl ObjectDesc {
    pub fn parse(node: &Node) -> Option<ObjectDesc> {
        let id = node.attr("id")?.to_owned();

        // A written number is honoured so the legacy files keep working unedited. Without one the
        // catalog assigns it, which is what lets new content be written without picking a free hex
        // value by hand.
        let object_type = node
            .attr_int("type")
            .map(|written| ObjectType(written as u16))
            .unwrap_or(ObjectType::UNASSIGNED);
        let uuid = crate::identity::identity(node.attr("uuid"), &id);

        let size = match node.int("Size") {
            Some(fixed) => SizeRange {
                min: fixed as i32,
                max: fixed as i32,
                step: 0,
            },
            None => SizeRange {
                min: node.int("MinSize").unwrap_or(100) as i32,
                max: node.int("MaxSize").unwrap_or(100) as i32,
                step: node.int("SizeStep").unwrap_or(0) as i32,
            },
        };

        let mut immunities = ConditionSet::EMPTY;
        for (flag, effect) in IMMUNITY_FLAGS {
            if node.has(flag) {
                immunities.insert(effect);
            }
        }

        let projectiles = node
            .children_named("Projectile")
            .enumerate()
            .map(|(index, shot)| ProjectileDesc::parse(shot, index as u8))
            .collect();

        let display_id = node.field("DisplayId").map(str::to_owned);

        Some(ObjectDesc {
            object_type,
            uuid,
            class: node.field("Class").unwrap_or_default().to_owned(),
            character: node.field("Class") == Some("Character"),
            player: node.has("Player"),
            enemy: node.has("Enemy"),
            god: node.has("God"),
            cube: node.has("Cube"),
            quest: node.has("Quest"),
            oryx: node.has("Oryx"),
            hero: node.has("Hero"),
            occupy_square: node.has("OccupySquare"),
            full_occupy: node.has("FullOccupy"),
            enemy_occupy_square: node.has("EnemyOccupySquare"),
            static_object: node.has("Static"),
            blocks_sight: node.has("BlocksSight"),
            flying: node.has("Flying"),
            connects: node.has("Connects"),
            protect_from_ground_damage: node.has("ProtectFromGroundDamage"),
            protect_from_sink: node.has("ProtectFromSink"),
            troll_white_bag: node.has("TrollWhiteBag"),
            max_hp: node.int("MaxHitPoints").unwrap_or(0) as i32,
            defense: node.int("Defense").unwrap_or(0) as i32,
            level: node.int("Level").map(|v| v as i32),
            exp_multiplier: node.float("XpMult").map(|v| v as f32),
            size,
            immunities,
            terrain: node.field("Terrain").map(str::to_owned),
            spawn_probability: node.float("SpawnProbability").unwrap_or(0.0) as f32,
            per_realm_max: node.int("PerRealmMax").map(|v| v as i32),
            spawn_count: node.child("Spawn").map(SpawnCount::parse),
            guild_item: node.field("GuildItem").map(str::to_owned),
            guild_item_param: node.field("GuildItemParam").map(str::to_owned),
            price: node.int("Price").map(|price| price as i32),
            projectiles,
            item: node.has("Item").then(|| ItemDesc::parse(node)),
            // `DungeonName` falls back to `DisplayId`, matching how portals are labelled.
            dungeon_name: node
                .field("DungeonName")
                .map(str::to_owned)
                .or_else(|| display_id.clone()),
            group: node.field("Group").map(str::to_owned),
            display_id,
            id,
        })
    }

    /// The name shown to players, which is the display id when one is given.
    ///
    /// A display id written as `{dungeon.some_key}` is a localisation key, not a name. The client
    /// that resolved those keys is gone and the tables never shipped with the content, so a key is
    /// treated as absent and the object id is shown instead. Roughly one display id in three across
    /// the files is a key, and rendering them raw puts `{shatters.shtrs_Fire_Mage}` over an enemy's
    /// head.
    pub fn name(&self) -> &str {
        match self.display_id.as_deref() {
            Some(display) if !display.starts_with('{') => display,
            _ => &self.id,
        }
    }

    pub fn is_item(&self) -> bool {
        self.item.is_some()
    }
}

/// The `<XxxImmune/>` flags, and the marker condition each one puts on the entity that declares it.
const IMMUNITY_FLAGS: [(&str, crate::effect::ConditionEffect); 8] = {
    use crate::effect::ConditionEffect::*;
    [
        ("ArmorBreakImmune", ArmorBreakImmune),
        ("CurseImmune", CurseImmune),
        ("DazedImmune", DazedImmune),
        ("ParalyzeImmune", ParalyzeImmune),
        ("PetrifyImmune", PetrifyImmune),
        ("SlowedImmune", SlowedImmune),
        ("StasisImmune", StasisImmune),
        ("StunImmune", StunImmune),
    ]
};

/// A character skin: what a player may look like instead of their class's own artwork.
///
/// `SkinDesc` in the original (`XmlDescriptors.cs:755-798`). A skin belongs to one class and to no
/// other, and that pairing is the only thing standing between a wardrobe and a wizard wearing a
/// priest. There are 191 of them in the content.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinDesc {
    pub object_type: ObjectType,
    pub id: String,

    /// The class this skin dresses. A skin is refused to anything else.
    pub class: ObjectType,

    /// What the player is scaled to while wearing it. 100 is the ordinary size.
    pub size: i32,

    /// The one character name allowed to wear it, where the content names one.
    ///
    /// Somebody else asking for it is not refused: they are put back into no skin at all, which is
    /// what `ReSkinCommand` does (`RankedCommands.cs:1222-1226`).
    pub player_exclusive: Option<String>,

    /// The level the class must reach before it can be worn.
    pub unlock_level: i32,

    /// What it costs to buy outright, in credits. A skin with no price written costs a thousand.
    pub cost: i32,

    /// Whether the skin is rented rather than kept, which the character-list document reports so a
    /// wardrobe can mark it as limited.
    pub expires: bool,

    /// Whether the skin is withheld from sale. A restricted skin is still described, so a player
    /// wearing one is drawn, but nobody may buy it.
    pub restricted: bool,
}

/// What a skin costs when the content names no price.
///
/// `XmlDescriptors.cs:788-789`. Every skin in the shipped files takes this default, so it is the
/// price the character-list document quotes for all 191 of them.
pub const SKIN_COST: i64 = 1000;

impl SkinDesc {
    /// Reads a skin, or `None` if the element is not one.
    ///
    /// The original's test is the presence of `<PlayerClassType>` rather than the class name, and
    /// it is the stricter of the two: a handful of `<Class>Skin</Class>` objects in the files carry
    /// no class at all and are not skins anybody can wear.
    pub fn parse(node: &Node, object_type: ObjectType) -> Option<SkinDesc> {
        let class = node.int("PlayerClassType")?;

        Some(SkinDesc {
            object_type,
            id: node.attr("id").unwrap_or_default().to_owned(),
            class: ObjectType(class as u16),
            // Written as an attribute of the object rather than as a child element, and defaulting
            // to the ordinary size when absent.
            size: node.attr_int("size").unwrap_or(100) as i32,
            player_exclusive: node.field("PlayerExclusive").map(str::to_owned),
            unlock_level: node.int("UnlockLevel").unwrap_or(0) as i32,
            // A thousand when the content names no price, which is where every skin in the shipped
            // files lands: `XmlDescriptors.cs:788-789`.
            cost: node.int("Cost").unwrap_or(SKIN_COST) as i32,
            expires: node.has("Expires"),
            restricted: node.has("Restricted"),
        })
    }
}

/// A ground tile type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TileDesc {
    /// The runtime number, assigned at load.
    pub tile_type: TileType,

    /// What a saved map refers to.
    pub uuid: uuid::Uuid,

    pub id: String,

    /// Movement multiplier. 1.0 is normal ground.
    pub speed: f32,

    pub no_walk: bool,
    pub sink: bool,
    pub push: bool,

    /// Whether standing on this hurts at all.
    ///
    /// Derived rather than written: the original sets it when either damage element is present,
    /// and there is no `<Damaging/>` element in any content file.
    pub damaging: bool,

    /// Damage dealt per second while stood on.
    pub min_damage: i32,
    pub max_damage: i32,

    pub blend_priority: i32,
}

impl Default for TileType {
    fn default() -> Self {
        TileType(0xff)
    }
}

impl TileDesc {
    /// The tile meaning "nothing here", which the map format writes for unreachable squares.
    pub const EMPTY: TileType = TileType(0xff);

    pub fn parse(node: &Node) -> Option<TileDesc> {
        Some(TileDesc {
            tile_type: node
                .attr_int("type")
                .map(|written| TileType(written as u16))
                .unwrap_or(TileType::UNASSIGNED),
            uuid: crate::identity::identity(node.attr("uuid"), node.attr("id")?),
            id: node.attr("id")?.to_owned(),
            speed: node.float("Speed").unwrap_or(1.0) as f32,
            no_walk: node.has("NoWalk"),
            sink: node.has("Sink"),
            push: node.has("Push"),
            damaging: node.int("MinDamage").is_some() || node.int("MaxDamage").is_some(),
            min_damage: node.int("MinDamage").unwrap_or(0) as i32,
            max_damage: node.int("MaxDamage").unwrap_or(0) as i32,
            blend_priority: node.int("BlendPriority").unwrap_or(0) as i32,
        })
    }

    pub fn hurts(&self) -> bool {
        self.damaging && self.max_damage > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::ConditionEffect;

    #[test]
    fn a_dye_carries_its_colour_as_hexadecimal_and_says_which_layer_it_paints() {
        // `Item.Texture1`/`Texture2` (`XmlDescriptors.cs:666-674`), read with
        // `Convert.ToInt32(value, 16)`. Read as decimal instead, `0x01F0F8FF` would be a nonsense
        // number and every dye in the game would paint the same wrong colour -- so this asserts
        // the number, not merely that something parsed.
        let clothing = parse_one(
            r#"<Object type="0x1000" id="Alice Blue Clothing Dye">
                 <Class>Dye</Class><Item/><SlotType>10</SlotType>
                 <Tex1>0x01F0F8FF</Tex1>
                 <Activate>Dye</Activate><Consumable/>
               </Object>"#,
        );
        let item = clothing.item.as_ref().expect("a dye is an item");
        assert_eq!(item.tex1, 0x01F0_F8FF);
        assert_eq!(item.tex2, 0, "a clothing dye leaves the accessory alone");

        // The accessory dye of the same colour writes the other element, which is the only thing
        // that tells the two apart.
        let accessory = parse_one(
            r#"<Object type="0x1100" id="Alice Blue Accessory Dye">
                 <Class>Dye</Class><Item/><SlotType>10</SlotType>
                 <Tex2>0x01F0F8FF</Tex2>
                 <Activate>Dye</Activate><Consumable/>
               </Object>"#,
        );
        let item = accessory.item.as_ref().expect("a dye is an item");
        assert_eq!(item.tex1, 0);
        assert_eq!(item.tex2, 0x01F0_F8FF);

        // A textile is the same field with a different type byte, and the index below it has to
        // survive intact or a patterned dye picks the wrong cell of the sheet.
        let textile = parse_one(
            r#"<Object type="0x1200" id="Cloth Textile"><Item/><SlotType>10</SlotType>
                 <Tex1>0x09000007</Tex1><Activate>Dye</Activate>
               </Object>"#,
        );
        assert_eq!(textile.item.as_ref().unwrap().tex1, 0x0900_0007);

        // And anything that is not a dye carries none, which is nearly every item.
        let wand = parse_one(
            r#"<Object type="0x0a22" id="Wand"><Item/><SlotType>8</SlotType></Object>"#,
        );
        let item = wand.item.as_ref().unwrap();
        assert_eq!((item.tex1, item.tex2), (0, 0));
    }

    fn parse_one(text: &str) -> ObjectDesc {
        ObjectDesc::parse(&Node::parse(text).unwrap()).expect("descriptor should parse")
    }

    #[test]
    fn a_weapon_carries_its_projectile() {
        let desc = parse_one(
            r#"<Object type="0xb0b" id="Sword of Acclaim">
                 <Class>Equipment</Class>
                 <Item/>
                 <SlotType>1</SlotType>
                 <Tier>12</Tier>
                 <RateOfFire>1</RateOfFire>
                 <Projectile>
                   <ObjectId>Purple Bolt</ObjectId>
                   <Speed>100</Speed>
                   <MinDamage>220</MinDamage>
                   <MaxDamage>275</MaxDamage>
                   <LifetimeMS>350</LifetimeMS>
                 </Projectile>
                 <BagType>4</BagType>
                 <FameBonus>4</FameBonus>
               </Object>"#,
        );

        assert_eq!(desc.object_type, ObjectType(0xb0b));
        assert!(desc.is_item());

        let item = desc.item.as_ref().unwrap();
        assert_eq!(item.slot_type, 1);
        assert_eq!(item.tier, Some(12));
        assert_eq!(item.fame_bonus, 4);

        assert_eq!(desc.projectiles.len(), 1);
        let shot = &desc.projectiles[0];
        assert_eq!(shot.object_id, "Purple Bolt");
        assert_eq!(shot.bullet_type, 0);
        assert_eq!(shot.min_damage, 220);
        assert_eq!(shot.max_damage, 275);
        assert_eq!(shot.lifetime_ms, 350);
        // Half-open, as the original's rolls are: the top of the range is never reached.
        assert_eq!(shot.roll_damage(0.0), 220);
        assert_eq!(shot.roll_damage(0.999), 274);
    }

    #[test]
    fn a_localisation_key_is_not_used_as_a_name() {
        // `Item.GetDisplayName` and `XmlData.AddObjects` in the original both check the first
        // character for `{` and fall back to the object id. Around a third of the display ids in
        // the shipped files are keys, so honouring one as a name is not an edge case.
        let key = parse_one(
            r#"<Object type="0x9ba" id="shtrs Fire Mage">
                 <Class>Character</Class>
                 <DisplayId>{shatters.shtrs_Fire_Mage}</DisplayId>
               </Object>"#,
        );
        assert_eq!(key.name(), "shtrs Fire Mage");

        let named = parse_one(
            r#"<Object type="0x9bb" id="shtrs Bridge Sentinel">
                 <Class>Character</Class>
                 <DisplayId>The Forgotten Sentinel</DisplayId>
               </Object>"#,
        );
        assert_eq!(named.name(), "The Forgotten Sentinel");

        let bare =
            parse_one(r#"<Object type="0x9bc" id="Sprite God"><Class>Character</Class></Object>"#);
        assert_eq!(bare.name(), "Sprite God");
    }

    #[test]
    fn stat_boosts_are_lifted_out_of_the_equip_activations() {
        let desc = parse_one(
            r#"<Object type="0xc1f" id="Chasuble of Holy Light">
                 <Class>Equipment</Class>
                 <Item/>
                 <SlotType>14</SlotType>
                 <ActivateOnEquip stat="21" amount="10">IncrementStat</ActivateOnEquip>
                 <ActivateOnEquip stat="3" amount="50">IncrementStat</ActivateOnEquip>
               </Object>"#,
        );

        let item = desc.item.as_ref().unwrap();

        // The written numbers are 21 and 3 and the stats are defence and max magic, which are
        // positions 3 and 1. Reading the numbers as positions is what made every mana bonus in
        // the game raise defence.
        assert_eq!(
            item.stat_boosts,
            vec![
                StatBoost {
                    stat: 3,
                    amount: 10
                },
                StatBoost {
                    stat: 1,
                    amount: 50
                },
            ]
        );
    }

    #[test]
    fn activations_keep_their_arguments_verbatim() {
        let desc = parse_one(
            r#"<Object type="0xc09" id="Tome of Purification">
                 <Class>Equipment</Class>
                 <Item/>
                 <SlotType>4</SlotType>
                 <Usable/>
                 <MpCost>120</MpCost>
                 <Activate amount="200" range="6" useWisMod="true">HealNova</Activate>
                 <Activate range="6">RemoveNegativeConditions</Activate>
               </Object>"#,
        );

        let item = desc.item.as_ref().unwrap();
        assert!(item.usable);
        assert_eq!(item.mp_cost, 120);
        assert_eq!(item.activate.len(), 2);

        let nova = &item.activate[0];
        assert_eq!(nova.name, "HealNova");
        assert_eq!(nova.int("amount"), Some(200));
        assert_eq!(nova.float("range"), Some(6.0));
        assert!(nova.flag("useWisMod"));
        assert!(!nova.flag("missing"));
    }

    #[test]
    fn immunity_flags_collapse_into_one_mask() {
        let desc = parse_one(
            r#"<Object type="0x01" id="Boss">
                 <Class>Character</Class>
                 <Enemy/>
                 <StunImmune/>
                 <ParalyzeImmune/>
                 <MaxHitPoints>50000</MaxHitPoints>
                 <Defense>40</Defense>
               </Object>"#,
        );

        assert!(desc.enemy);
        assert!(desc.character);
        assert_eq!(desc.max_hp, 50000);
        assert_eq!(desc.defense, 40);
        // The markers, not the effects they block. An entity given this set holds `StunImmune`;
        // giving it `Stunned` would be the opposite of what the flag says.
        assert!(desc.immunities.contains(ConditionEffect::StunImmune));
        assert!(desc.immunities.contains(ConditionEffect::ParalyzeImmune));
        assert!(!desc.immunities.contains(ConditionEffect::Stunned));
        assert!(!desc.immunities.contains(ConditionEffect::Paralyzed));
        assert!(!desc.immunities.contains(ConditionEffect::SlowedImmune));
    }

    #[test]
    fn a_skin_names_the_one_class_it_belongs_to() {
        let node = Node::parse(
            r#"<Object type="0x0344" id="Merlin" size="120">
                 <Skin/>
                 <Class>Skin</Class>
                 <PlayerClassType>0x030e</PlayerClassType>
                 <UnlockLevel>10</UnlockLevel>
               </Object>"#,
        )
        .unwrap();

        let skin = SkinDesc::parse(&node, ObjectType(0x0344)).expect("a skin");
        assert_eq!(skin.class, ObjectType(0x030e));
        assert_eq!(skin.unlock_level, 10);
        assert_eq!(skin.size, 120);
        assert_eq!(skin.player_exclusive, None);
    }

    #[test]
    fn a_skin_without_a_class_is_not_a_skin_anybody_can_wear() {
        // The original's test is the presence of `PlayerClassType`, not the class name, and it
        // returns null without one.
        let node = Node::parse(
            r#"<Object type="0x0400" id="Placeholder">
                 <Class>Skin</Class>
               </Object>"#,
        )
        .unwrap();

        assert!(SkinDesc::parse(&node, ObjectType(0x0400)).is_none());
    }

    #[test]
    fn a_tile_reads_its_walkability_and_damage() {
        let node = Node::parse(
            r#"<Ground type="0x00" id="Black Water">
                 <NoWalk/>
                 <Speed>1</Speed>
               </Ground>"#,
        )
        .unwrap();

        let tile = TileDesc::parse(&node).unwrap();
        assert_eq!(tile.tile_type, TileType(0));
        assert_eq!(tile.id, "Black Water");
        assert!(tile.no_walk);
        assert!(!tile.hurts());
    }
}

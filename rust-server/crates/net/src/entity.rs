//! Entity state and the field mask that keeps snapshots small.
//!
//! An entity has a dozen or so transmittable fields, and on any given tick almost none of them
//! change. A monster that is walking changes its position and nothing else; one that is standing
//! still changes nothing at all. Writing a fixed record per entity spends bytes in proportion to
//! how many entities exist; writing a mask and only the fields behind it spends bytes in proportion
//! to how much actually happened.
//!
//! The legacy protocol did the former, which is why its tick packets grew with crowd size even when
//! the crowd was idle.

use crate::codec::{CodecError, Reader, Writer, quantize};

/// How many stats an entity record carries.
///
/// `StatsManager.NumStatTypes` (`realm/StatsManager.cs:11`). Named here rather than borrowed from
/// the content crate because this one is compiled into the client as well and holds no dependency
/// on it; the two are the same eleven and a mismatch would show up as a decode failure at once.
pub const STAT_COUNT: usize = 11;

/// A server-assigned entity handle, unique within a world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EntityId(pub u32);

/// Which fields a record carries.
///
/// Written as a varint, so the common case of an entity that only moved costs a single byte
/// regardless of how many fields exist in total. Adding a field later costs nothing until something
/// sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FieldMask(pub u32);

impl FieldMask {
    pub const EMPTY: FieldMask = FieldMask(0);

    pub const POSITION: FieldMask = FieldMask(1 << 0);
    pub const HP: FieldMask = FieldMask(1 << 1);
    pub const MAX_HP: FieldMask = FieldMask(1 << 2);
    pub const MP: FieldMask = FieldMask(1 << 3);
    pub const MAX_MP: FieldMask = FieldMask(1 << 4);
    pub const CONDITIONS: FieldMask = FieldMask(1 << 5);
    pub const SIZE: FieldMask = FieldMask(1 << 6);
    pub const NAME: FieldMask = FieldMask(1 << 7);
    pub const OBJECT_TYPE: FieldMask = FieldMask(1 << 8);
    pub const TEXTURE: FieldMask = FieldMask(1 << 9);
    pub const STATS: FieldMask = FieldMask(1 << 10);
    pub const STARS: FieldMask = FieldMask(1 << 11);
    pub const OXYGEN: FieldMask = FieldMask(1 << 12);
    pub const PROGRESS: FieldMask = FieldMask(1 << 13);
    pub const CONTENTS: FieldMask = FieldMask(1 << 14);
    pub const SKIN: FieldMask = FieldMask(1 << 15);
    pub const MERCHANDISE: FieldMask = FieldMask(1 << 16);
    pub const PURSE: FieldMask = FieldMask(1 << 17);
    pub const GUILD: FieldMask = FieldMask(1 << 18);
    pub const BOOSTS: FieldMask = FieldMask(1 << 19);
    pub const GLOW: FieldMask = FieldMask(1 << 20);

    /// The rarely-changing booleans, gathered into one word. See [`EntityState::flags`].
    pub const FLAGS: FieldMask = FieldMask(1 << 21);

    pub const DYES: FieldMask = FieldMask(1 << 22);
    pub const CONNECTION: FieldMask = FieldMask(1 << 23);
    pub const BOOST_TIME: FieldMask = FieldMask(1 << 24);

    /// Every field, for an entity the receiver has never seen.
    pub const ALL: FieldMask = FieldMask(0x1ffffff);

    pub fn has(self, field: FieldMask) -> bool {
        self.0 & field.0 != 0
    }

    pub fn with(self, field: FieldMask) -> FieldMask {
        FieldMask(self.0 | field.0)
    }

    pub fn set(&mut self, field: FieldMask, when: bool) {
        if when {
            self.0 |= field.0;
        }
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Everything about an entity that travels to a client.
///
/// Deliberately a flat struct of plain fields rather than a map of typed values: comparing two of
/// these is a handful of integer comparisons, and encoding one touches no allocation. The name is
/// the only heap field, and it is the only one that essentially never changes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EntityState {
    pub object_type: u16,
    pub x: f32,
    pub y: f32,
    pub hp: i32,
    pub max_hp: i32,
    pub mp: i32,
    pub max_mp: i32,

    /// Condition effects, as the bitmask described in `hendra-content`. Kept as a bare `u128` so
    /// this crate stays free of a content dependency. The client's extension has no use for loot
    /// tables or spawn rules.
    pub conditions: u128,

    /// Rendered size in percent, where 100 is the object's natural size.
    pub size: u16,

    pub name: Option<Box<str>>,

    /// Which sprite to draw, for entities that change appearance without changing type.
    pub texture: u8,

    /// The skin a player is wearing, as the object type of the skin itself. Zero for the class's
    /// own sprite.
    ///
    /// `StatsType.Skin = 81` (`realm/Stats.cs:81`), written by `ExportStats` (`Player.cs:303`) and
    /// read by the client as its `TEXTURE_STAT` (`StatData.as:90`) to pick a whole different
    /// animated sheet for the body (`ReskinHandler.as:21-32`).
    ///
    /// Not the same thing as [`Self::texture`], which is the alt-texture index a boss uses to
    /// change phase: that picks a frame within the sprite the object type already names, where this
    /// replaces the sprite.
    pub skin: u16,

    /// The eleven stats, in the order the game numbers them.
    ///
    /// Eleven rather than the eight a class declares, because `Player.ExportStats` sends eleven:
    /// `DamageMin` and `DamageMax` hold the equipped weapon's damage and `Luck` the private-drop
    /// bonus (`Player.cs:331-341`, `StatsManager.cs:11`). The last three are the only stats a
    /// client cannot work out for itself from the item it drew.
    ///
    /// Only a player's own entity carries meaningful values; everything else sends zeroes, which
    /// cost one byte each as varints and never change, so they never appear in a delta.
    pub stats: [i32; STAT_COUNT],

    /// What equipment and timed boosts add on top of the base, one entry per stat.
    ///
    /// `Player.ExportStats` sends all eleven of `Stats.Boost` beside the eleven totals
    /// (`Player.cs:342-352`), and they are what the character sheet draws in green: the total alone
    /// cannot say how much of a wizard's attack is the wizard and how much is the ring. Positive and
    /// negative both travel, since a cursed item takes a stat down and the sheet shows that in red.
    ///
    /// Zero for anything that is not a player, so the mask bit is never raised for one.
    pub boosts: [i32; STAT_COUNT],

    /// The guild this player belongs to, and their rank in it.
    ///
    /// `StatsType.Guild` and `StatsType.GuildRank` (`Player.cs:291-292`). The name is what the
    /// client writes under a player's own (`GuildText.as:25-48`, `PlayerToolTip.as:39-40`) and the
    /// rank is what colours it; matching a name against one's own is also how the original decides
    /// who is a guildmate (`Player.setGuildName`, `Player.as:340-360`), so a player who is never
    /// told cannot see their own guild standing beside them.
    ///
    /// `None` for anybody in no guild, and for everything that is not a player.
    pub guild: Option<Box<str>>,

    /// Rank within [`Self::guild`], as the original numbers them: 0 initiate, 10 member,
    /// 20 officer, 30 leader, 40 founder. Zero when there is no guild.
    pub guild_rank: i16,

    /// How many stars this player has earned, which is what everybody else sees beside their name.
    ///
    /// The sum over every class of what its best fame is worth, so it is a record of an account
    /// rather than of the character being looked at. Zero for anything that is not a player.
    pub stars: u8,

    /// How much air a player has left, from 100 down to 0.
    ///
    /// Only a drowning world spends it, and everywhere else it sits at full and never appears in a
    /// delta. A player at zero is taking damage every tick, so the bar is the only warning they
    /// get before it starts.
    pub oxygen: u8,

    /// What level this character has reached. Zero for anything that is not a player.
    pub level: i16,

    /// Experience earned since the current level began, which is what the bar fills with.
    ///
    /// The running total minus the total the level started at, as the original sends
    /// (`Player.cs:287`, `Experience - GetLevelExp(Level)`). A client that received the lifetime
    /// total would have to know the curve to draw a bar out of it.
    pub experience: i32,

    /// Experience the current level needs before the next one, the bar's ceiling.
    pub experience_goal: i32,

    /// Fame this character has banked, which is its lifetime experience divided by a thousand.
    pub fame: i32,

    /// The account's own purse: gold, spendable fame and prestige.
    ///
    /// `Player.ExportStats` sends all three beside the character's own fame (`Player.cs:291-299`),
    /// and they are three different things: [`Self::fame`] is what this character has earned and
    /// `current_fame` is what the account can spend. Every shop in the game prices in one of these,
    /// so a client that receives none of them shows an empty purse and refuses to buy anything.
    ///
    /// Zero for anything that is not a player, and only ever meaningful on a player's own entity.
    pub credits: i32,
    pub current_fame: i32,
    pub prestige: i32,

    /// What is inside this entity, for a loot bag, a chest or a vault.
    ///
    /// `Container.ExportStats` (`realm/entities/Container.cs:76-90`) writes the eight slots into the
    /// entity's stats, which is the only way a client knows a bag on the ground is worth walking to.
    /// `None` for everything that holds nothing, which is almost every entity, and a field nothing
    /// sets costs nothing: the mask bit is never raised and the slots never travel.
    ///
    /// Empty slots are `u16::MAX` rather than zero, since zero is a real object type.
    pub contents: Option<Box<[u16; 8]>>,

    /// What this entity is selling, for a merchant standing in a shop.
    ///
    /// `SellableObject.ExportStats` and `Merchant.ExportStats` write these five into the entity's
    /// stats (`realm/entities/vendors/SellableObject.cs:72-78`,
    /// `realm/entities/vendors/Merchant.cs:46-52`). Without them a client has no way to know a
    /// vendor sells anything: the original draws a merchant as the item rather than as itself, and
    /// the panel that buys is the one the merchandise type opens.
    ///
    /// `None` for everything that sells nothing, which is almost every entity, and a field nothing
    /// sets never raises its mask bit.
    pub merchandise: Option<Merchandise>,

    /// What colour this body is haloed in, as a packed `0xRRGGBB`. Zero is no halo, which is
    /// everybody.
    ///
    /// `StatsType.Glow` (`Stats.cs:60`), exported on every update (`Player.cs:302`) and read by the
    /// client as its `GLOW_COLOR_STAT`, which hands it to `GameObject.setGlow`
    /// (`GameServerConnectionConcrete.as:1539-1540`) and from there to `GlowRedrawer`, which paints
    /// a coloured ring around the sprite (`GlowRedrawer.as:19-46`). It is what `/glow` sets, and
    /// without it on the wire that command changes nothing anybody can see.
    pub glow: i32,

    /// Whether this player is an administrator.
    ///
    /// `StatsType.Admin` (`Player.cs:359`, `Account.Admin ? 1 : 0`). The client draws it into the
    /// name plate rather than writing it out: the star beside a name is coloured by the rating for
    /// everybody else and by one colour of its own for an administrator
    /// (`Player.makeNameBitmapData` -> `FameUtil.numStarsToIcon(numStars_, admin_)`,
    /// `Player.as:750-756`).
    pub admin: bool,

    /// Whether this character owns the eight extra carried slots.
    ///
    /// `StatsType.HasBackpack` (`Player.cs:355`). The client hides the second row entirely without
    /// it, so a character that paid for a backpack cannot reach the slots it bought — and the
    /// server refuses a move into them for anybody who has not (`InvSwapHandler.cs:177`), so the two
    /// ends have to agree about who has one.
    pub has_backpack: bool,

    /// Whether this account has picked its own name rather than been given one.
    ///
    /// `StatsType.NameChosen` (`Player.cs:299-300`), read off the account rather than the body so
    /// that a name claimed mid-session counts at once. It colours the name over the head:
    /// `Player.getNameColor` answers `NAME_CHOSEN_COLOR` for a player who has one and plain white
    /// for a player who has not (`Player.as:757-764`), which is how the original marks an account
    /// still wearing a name off the generated list.
    pub name_chosen: bool,

    /// Whether this portal refuses to be entered.
    ///
    /// `StatsType.PortalUsable` inverted (`Portal.cs:57`). Inverted so that the default — every
    /// other entity in the world, none of which is a portal — reads as the original's default of
    /// `Usable = true` (`Portal.cs:15`) rather than as a world full of dead doors.
    ///
    /// `PortalPanel` hides its Enter button while it is set (`PortalPanel.as:92-97`), so a client
    /// never told is a client offering a door the server will refuse (`UsePortalHandler.cs:56`).
    pub portal_unusable: bool,

    /// The two dyes worn over this body's artwork, as the original packs them.
    ///
    /// `StatsType.Texture1` and `Texture2` (`Player.cs:301-302`), set by using a dye item
    /// (`Player.UseItem.AEDye`, `Player.UseItem.cs:583-589`) and kept on the character
    /// (`Player.cs:373-374`). The top byte is a type and the low twenty-four its argument: `0`
    /// nothing, `1` a solid `0xRRGGBB`, and `4`, `5`, `9` or `10` an index into the
    /// `textile{n}x{n}` sheet of that size (`TextureRedrawer.getTexture`,
    /// `TextureRedrawer.as:161-186`). The first dyes the cloth and the second the accessory,
    /// chosen by the sprite's own mask.
    ///
    /// Zero for anybody wearing none, which is nearly everybody, so the pair never appears in a
    /// delta.
    pub tex1: i32,
    pub tex2: i32,

    /// Which neighbours this piece of scenery joins onto, as `ConnectionInfo.Bits`.
    ///
    /// `StatsType.ObjectConnection` (`ConnectedObject.cs:114-118`). Four bytes, one per side, each
    /// `1` or `2`, which the client turns into one of six connector shapes and a rotation —
    /// `ConnectedObject.getConnectedResults(connectType_)` (`ConnectedObject.as:98`). Without it
    /// every cave wall and every fence in the game draws as unjoined posts.
    ///
    /// Zero for everything that is not a `ConnectedWall` or a `CaveWall` (`Entity.cs:600-602`).
    pub connection: u32,

    /// What is left of this player's three timed boosts, in seconds.
    ///
    /// `StatsType.XPBoostTime`, `LDBoostTime` and `LTBoostTime`, which `ExportStats` sends as
    /// milliseconds divided by a thousand (`Player.cs:356-358`). Seconds rather than milliseconds
    /// because that is the resolution the original chose and the client counts down in whole
    /// seconds either way.
    ///
    /// The separate `StatsType.XPBoost` flag is not carried: the original derives it from this very
    /// number, `(XPBoostTime != 0) ? 1 : 0` (`Player.cs:355`), so a client holding the clock can
    /// work the flag out, and a client holding both could be told two different things.
    pub experience_boost_seconds: i32,
    pub loot_drop_boost_seconds: i32,
    pub loot_tier_boost_seconds: i32,

    /// The fame the next class quest asks for.
    ///
    /// `StatsType.FameGoal` (`Player.cs:293`), `GetFameGoal(BestFame)` off this class's best run
    /// (`Player.cs:537`, `Player.Leveling.cs:19`). It is the ceiling the fame bar fills towards, so
    /// without it the bar has a numerator and no denominator.
    pub fame_goal: i32,
}

/// What a merchant is offering.
///
/// The five stats `SellableObject` and `Merchant` export, gathered into one field: they are set
/// together when a stall is stood and change together when it rotates, so a mask bit each would
/// cost more than the values do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Merchandise {
    /// The object type of the thing for sale, which is also the sprite the merchant is drawn as.
    pub item: u16,

    pub price: i32,

    /// `CurrencyType`: 0 gold, 1 fame.
    pub currency: u8,

    /// How many are left, or -1 for a shop whose stock never runs out.
    pub count: i32,

    /// The stars a customer needs before this one will trade at all.
    pub rank: i16,
}

/// What an empty container slot is on the wire.
pub const NO_ITEM: u16 = u16::MAX;

/// Bit positions within the packed flags word.
///
/// Four booleans that between them move perhaps twice in a session: an account does not become an
/// administrator mid-world, a backpack is bought between worlds, a name is claimed once, and a
/// portal closes when its dungeon does. One varint carries all four, so they cost a byte on the
/// first sighting and nothing afterwards.
mod flag {
    pub const ADMIN: u64 = 1 << 0;
    pub const BACKPACK: u64 = 1 << 1;
    pub const NAME_CHOSEN: u64 = 1 << 2;
    pub const PORTAL_UNUSABLE: u64 = 1 << 3;
}

impl EntityState {
    /// The four rarely-changing booleans, packed into one word.
    fn flags(&self) -> u64 {
        let mut bits = 0;
        if self.admin {
            bits |= flag::ADMIN;
        }
        if self.has_backpack {
            bits |= flag::BACKPACK;
        }
        if self.name_chosen {
            bits |= flag::NAME_CHOSEN;
        }
        if self.portal_unusable {
            bits |= flag::PORTAL_UNUSABLE;
        }
        bits
    }

    /// Which fields differ from `baseline`.
    ///
    /// Positions are compared *after* quantisation. Two coordinates that differ by less than a
    /// transmission step encode to identical bytes, so treating them as a change would spend a byte
    /// to tell the client something it already believes. Floating-point drift in the simulation
    /// makes that case common rather than rare.
    pub fn changes_from(&self, baseline: &EntityState) -> FieldMask {
        let mut mask = FieldMask::EMPTY;

        mask.set(
            FieldMask::POSITION,
            quantize(self.x) != quantize(baseline.x) || quantize(self.y) != quantize(baseline.y),
        );
        mask.set(FieldMask::HP, self.hp != baseline.hp);
        mask.set(FieldMask::MAX_HP, self.max_hp != baseline.max_hp);
        mask.set(FieldMask::MP, self.mp != baseline.mp);
        mask.set(FieldMask::MAX_MP, self.max_mp != baseline.max_mp);
        mask.set(
            FieldMask::CONDITIONS,
            self.conditions != baseline.conditions,
        );
        mask.set(FieldMask::SIZE, self.size != baseline.size);
        mask.set(FieldMask::NAME, self.name != baseline.name);
        mask.set(
            FieldMask::OBJECT_TYPE,
            self.object_type != baseline.object_type,
        );
        mask.set(FieldMask::TEXTURE, self.texture != baseline.texture);
        mask.set(FieldMask::STATS, self.stats != baseline.stats);
        mask.set(FieldMask::BOOSTS, self.boosts != baseline.boosts);

        // One bit for the pair. A guild is joined, left or promoted as a single act, and no player
        // ever holds a rank in a guild they are not in.
        mask.set(
            FieldMask::GUILD,
            self.guild != baseline.guild || self.guild_rank != baseline.guild_rank,
        );
        mask.set(FieldMask::STARS, self.stars != baseline.stars);
        mask.set(FieldMask::OXYGEN, self.oxygen != baseline.oxygen);

        // One bit for all four, as the stats have one bit for all eight. A level-up moves every one
        // of them at once, and the only one that ever moves alone is the experience, which moves
        // once per kill on a single entity.
        mask.set(
            FieldMask::PROGRESS,
            self.level != baseline.level
                || self.experience != baseline.experience
                || self.experience_goal != baseline.experience_goal
                || self.fame != baseline.fame
                || self.fame_goal != baseline.fame_goal,
        );

        // One bit for all three, as the levelling above has one for its four: a purchase moves the
        // purse and nothing else on the entity, and the three move together whenever anything
        // moves them at all (`Merchant.TransactionItemComplete`, `Merchant.cs:192-195`).
        mask.set(
            FieldMask::PURSE,
            self.credits != baseline.credits
                || self.current_fame != baseline.current_fame
                || self.prestige != baseline.prestige,
        );

        mask.set(FieldMask::CONTENTS, self.contents != baseline.contents);
        mask.set(FieldMask::SKIN, self.skin != baseline.skin);
        mask.set(
            FieldMask::MERCHANDISE,
            self.merchandise != baseline.merchandise,
        );
        mask.set(FieldMask::GLOW, self.glow != baseline.glow);
        mask.set(FieldMask::FLAGS, self.flags() != baseline.flags());

        // One bit for the pair: a dye is applied to one or the other, but the two are read together
        // by the one redraw that puts them on (`TextureRedrawer.resize`), and neither moves again
        // until another dye is used.
        mask.set(
            FieldMask::DYES,
            self.tex1 != baseline.tex1 || self.tex2 != baseline.tex2,
        );

        mask.set(FieldMask::CONNECTION, self.connection != baseline.connection);

        // The three clocks together. They tick in whole seconds, so this raises a bit once a second
        // for a player carrying a boost and never for anybody else.
        mask.set(
            FieldMask::BOOST_TIME,
            self.experience_boost_seconds != baseline.experience_boost_seconds
                || self.loot_drop_boost_seconds != baseline.loot_drop_boost_seconds
                || self.loot_tier_boost_seconds != baseline.loot_tier_boost_seconds,
        );

        mask
    }

    /// Writes the fields named by `mask`.
    ///
    /// When `baseline` is `Some`, the position is written as a delta against it; otherwise it is
    /// absolute. The decoder makes the same choice from the same information, which is what keeps
    /// the two in step without a flag on the wire.
    pub fn encode(&self, mask: FieldMask, baseline: Option<&EntityState>, w: &mut Writer<'_>) {
        w.varint(mask.0 as u64);

        if mask.has(FieldMask::OBJECT_TYPE) {
            w.varint(self.object_type as u64);
        }
        if mask.has(FieldMask::POSITION) {
            match baseline {
                Some(from) => {
                    w.position_delta(self.x, from.x);
                    w.position_delta(self.y, from.y);
                }
                None => {
                    w.position(self.x);
                    w.position(self.y);
                }
            }
        }
        if mask.has(FieldMask::HP) {
            w.varint_signed(self.hp as i64);
        }
        if mask.has(FieldMask::MAX_HP) {
            w.varint_signed(self.max_hp as i64);
        }
        if mask.has(FieldMask::MP) {
            w.varint_signed(self.mp as i64);
        }
        if mask.has(FieldMask::MAX_MP) {
            w.varint_signed(self.max_mp as i64);
        }
        if mask.has(FieldMask::CONDITIONS) {
            w.condition_mask(self.conditions);
        }
        if mask.has(FieldMask::SIZE) {
            w.varint(self.size as u64);
        }
        if mask.has(FieldMask::NAME) {
            match &self.name {
                Some(name) => {
                    w.bool(true);
                    w.string(name);
                }
                None => w.bool(false),
            }
        }
        if mask.has(FieldMask::TEXTURE) {
            w.varint(self.texture as u64);
        }
        if mask.has(FieldMask::STATS) {
            // All eleven together. They change as a group at a level-up or a weapon swap, and a
            // mask bit each would cost more than the values do.
            for stat in self.stats {
                w.varint_signed(stat as i64);
            }
        }
        if mask.has(FieldMask::BOOSTS) {
            // All eleven together, like the totals above: a weapon swap or a boost lapsing moves
            // several at once and none of them ever moves alone.
            for boost in self.boosts {
                w.varint_signed(boost as i64);
            }
        }
        if mask.has(FieldMask::GUILD) {
            match &self.guild {
                Some(guild) => {
                    w.bool(true);
                    w.string(guild);
                }
                None => w.bool(false),
            }
            w.varint_signed(self.guild_rank as i64);
        }
        if mask.has(FieldMask::STARS) {
            w.varint(self.stars as u64);
        }
        if mask.has(FieldMask::OXYGEN) {
            w.varint(self.oxygen as u64);
        }
        if mask.has(FieldMask::PROGRESS) {
            w.varint_signed(self.level as i64);
            w.varint_signed(self.experience as i64);
            w.varint_signed(self.experience_goal as i64);
            w.varint_signed(self.fame as i64);
            w.varint_signed(self.fame_goal as i64);
        }
        if mask.has(FieldMask::PURSE) {
            w.varint_signed(self.credits as i64);
            w.varint_signed(self.current_fame as i64);
            w.varint_signed(self.prestige as i64);
        }
        if mask.has(FieldMask::CONTENTS) {
            match &self.contents {
                Some(slots) => {
                    w.bool(true);
                    for item in slots.iter() {
                        w.varint(*item as u64);
                    }
                }
                None => w.bool(false),
            }
        }
        if mask.has(FieldMask::SKIN) {
            w.varint(self.skin as u64);
        }
        if mask.has(FieldMask::MERCHANDISE) {
            match &self.merchandise {
                Some(stall) => {
                    w.bool(true);
                    w.varint(stall.item as u64);
                    w.varint_signed(stall.price as i64);
                    w.varint(stall.currency as u64);
                    w.varint_signed(stall.count as i64);
                    w.varint_signed(stall.rank as i64);
                }
                None => w.bool(false),
            }
        }
        if mask.has(FieldMask::GLOW) {
            w.varint_signed(self.glow as i64);
        }
        if mask.has(FieldMask::FLAGS) {
            w.varint(self.flags());
        }
        if mask.has(FieldMask::DYES) {
            w.varint_signed(self.tex1 as i64);
            w.varint_signed(self.tex2 as i64);
        }
        if mask.has(FieldMask::CONNECTION) {
            w.varint(self.connection as u64);
        }
        if mask.has(FieldMask::BOOST_TIME) {
            w.varint_signed(self.experience_boost_seconds as i64);
            w.varint_signed(self.loot_drop_boost_seconds as i64);
            w.varint_signed(self.loot_tier_boost_seconds as i64);
        }
    }

    /// Reads a record, starting from `baseline` and overwriting only what the mask names.
    ///
    /// Fields absent from the mask keep their baseline values, which is the whole point: the sender
    /// omitted them precisely because they had not changed.
    pub fn decode(
        baseline: Option<&EntityState>,
        r: &mut Reader<'_>,
    ) -> Result<EntityState, CodecError> {
        let mask = FieldMask(r.varint_u32()?);
        let mut state = baseline.cloned().unwrap_or_default();

        if mask.has(FieldMask::OBJECT_TYPE) {
            state.object_type =
                u16::try_from(r.varint()?).map_err(|_| CodecError::InvalidValue {
                    what: "object type",
                    value: 0,
                })?;
        }
        if mask.has(FieldMask::POSITION) {
            match baseline {
                Some(from) => {
                    state.x = r.position_delta(from.x)?;
                    state.y = r.position_delta(from.y)?;
                }
                None => {
                    state.x = r.position_value()?;
                    state.y = r.position_value()?;
                }
            }
        }
        if mask.has(FieldMask::HP) {
            state.hp = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::MAX_HP) {
            state.max_hp = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::MP) {
            state.mp = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::MAX_MP) {
            state.max_mp = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::CONDITIONS) {
            state.conditions = r.condition_mask()?;
        }
        if mask.has(FieldMask::SIZE) {
            state.size = u16::try_from(r.varint()?).unwrap_or(u16::MAX);
        }
        if mask.has(FieldMask::NAME) {
            state.name = if r.bool()? {
                Some(r.string()?.into())
            } else {
                None
            };
        }
        if mask.has(FieldMask::TEXTURE) {
            state.texture = u8::try_from(r.varint()?).map_err(|_| CodecError::InvalidValue {
                what: "texture",
                value: 0,
            })?;
        }
        if mask.has(FieldMask::STATS) {
            for stat in state.stats.iter_mut() {
                *stat = r.varint_signed()? as i32;
            }
        }
        if mask.has(FieldMask::BOOSTS) {
            for boost in state.boosts.iter_mut() {
                *boost = r.varint_signed()? as i32;
            }
        }
        if mask.has(FieldMask::GUILD) {
            state.guild = if r.bool()? {
                Some(r.string()?.into())
            } else {
                None
            };
            state.guild_rank = r.varint_signed()? as i16;
        }
        if mask.has(FieldMask::STARS) {
            state.stars = u8::try_from(r.varint()?).map_err(|_| CodecError::InvalidValue {
                what: "stars",
                value: 0,
            })?;
        }
        if mask.has(FieldMask::OXYGEN) {
            state.oxygen = u8::try_from(r.varint()?).map_err(|_| CodecError::InvalidValue {
                what: "oxygen",
                value: 0,
            })?;
        }
        if mask.has(FieldMask::PROGRESS) {
            state.level = r.varint_signed()? as i16;
            state.experience = r.varint_signed()? as i32;
            state.experience_goal = r.varint_signed()? as i32;
            state.fame = r.varint_signed()? as i32;
            state.fame_goal = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::PURSE) {
            state.credits = r.varint_signed()? as i32;
            state.current_fame = r.varint_signed()? as i32;
            state.prestige = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::CONTENTS) {
            state.contents = if r.bool()? {
                let mut slots = [NO_ITEM; 8];
                for slot in slots.iter_mut() {
                    *slot = u16::try_from(r.varint()?).map_err(|_| CodecError::InvalidValue {
                        what: "container slot",
                        value: 0,
                    })?;
                }
                Some(Box::new(slots))
            } else {
                None
            };
        }
        if mask.has(FieldMask::SKIN) {
            state.skin = u16::try_from(r.varint()?).map_err(|_| CodecError::InvalidValue {
                what: "skin",
                value: 0,
            })?;
        }
        if mask.has(FieldMask::MERCHANDISE) {
            state.merchandise = if r.bool()? {
                Some(Merchandise {
                    item: u16::try_from(r.varint()?).map_err(|_| CodecError::InvalidValue {
                        what: "merchandise type",
                        value: 0,
                    })?,
                    price: r.varint_signed()? as i32,
                    currency: u8::try_from(r.varint()?).map_err(|_| CodecError::InvalidValue {
                        what: "merchandise currency",
                        value: 0,
                    })?,
                    count: r.varint_signed()? as i32,
                    rank: r.varint_signed()? as i16,
                })
            } else {
                None
            };
        }
        if mask.has(FieldMask::GLOW) {
            state.glow = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::FLAGS) {
            let bits = r.varint()?;
            state.admin = bits & flag::ADMIN != 0;
            state.has_backpack = bits & flag::BACKPACK != 0;
            state.name_chosen = bits & flag::NAME_CHOSEN != 0;
            state.portal_unusable = bits & flag::PORTAL_UNUSABLE != 0;
        }
        if mask.has(FieldMask::DYES) {
            state.tex1 = r.varint_signed()? as i32;
            state.tex2 = r.varint_signed()? as i32;
        }
        if mask.has(FieldMask::CONNECTION) {
            state.connection = r.varint_u32()?;
        }
        if mask.has(FieldMask::BOOST_TIME) {
            state.experience_boost_seconds = r.varint_signed()? as i32;
            state.loot_drop_boost_seconds = r.varint_signed()? as i32;
            state.loot_tier_boost_seconds = r.varint_signed()? as i32;
        }

        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn what_a_container_holds_travels_and_only_when_it_changes() {
        // `Container.ExportStats` writes the eight slots into the entity's stats
        // (`realm/entities/Container.cs:76-90`). Without them a client draws every bag on the
        // ground as empty, and a player has no way to know which one is worth walking back for.
        let before = walking();
        assert_eq!(before.contents, None, "most entities hold nothing");

        let mut after = before.clone();
        after.contents = Some(Box::new([0x0a22, NO_ITEM, 0x0904, NO_ITEM, NO_ITEM, NO_ITEM, NO_ITEM, NO_ITEM]));

        let mask = after.changes_from(&before);
        assert!(mask.has(FieldMask::CONTENTS));

        let mut buffer = Vec::new();
        after.encode(mask, Some(&before), &mut Writer::new(&mut buffer));
        let read = EntityState::decode(Some(&before), &mut Reader::new(&buffer)).unwrap();
        assert_eq!(read.contents, after.contents);

        // And an unchanged bag costs nothing, which is what keeps a room full of them off the wire.
        assert_eq!(read.changes_from(&after), FieldMask::EMPTY);
    }

    #[test]
    fn what_a_merchant_sells_travels_and_only_when_it_changes() {
        // The five stats `SellableObject` and `Merchant` export
        // (`realm/entities/vendors/SellableObject.cs:72-78`, `.../Merchant.cs:46-52`). Without
        // them a client draws every vendor as a placeholder and the panel that buys never opens,
        // which leaves every shop in the game reachable only by typing.
        let before = walking();
        assert_eq!(before.merchandise, None, "most entities sell nothing");

        let mut after = before.clone();
        after.merchandise = Some(Merchandise {
            item: 0x0a22,
            price: 500,
            currency: 1,
            count: 3,
            rank: 2,
        });

        let mask = after.changes_from(&before);
        assert!(mask.has(FieldMask::MERCHANDISE));

        let mut buffer = Vec::new();
        after.encode(mask, Some(&before), &mut Writer::new(&mut buffer));
        let read = EntityState::decode(Some(&before), &mut Reader::new(&buffer)).unwrap();
        assert_eq!(read.merchandise, after.merchandise);

        // A rotation moves the price and the count as well as the item, and a stall that stands
        // still costs nothing.
        assert_eq!(read.changes_from(&after), FieldMask::EMPTY);

        let mut rotated = after.clone();
        rotated.merchandise = Some(Merchandise {
            item: 0x0904,
            price: 25,
            currency: 1,
            count: 1,
            rank: 2,
        });
        assert!(rotated.changes_from(&after).has(FieldMask::MERCHANDISE));
    }

    #[test]
    fn a_shop_that_never_runs_out_says_so_with_a_negative_count() {
        // `Merchant._count` starts at -1 and `MerchantLists` shops leave it there, which is what
        // tells the client to draw no stock line at all rather than "0 left".
        let before = walking();
        let mut after = before.clone();
        after.merchandise = Some(Merchandise {
            item: 0x0a22,
            price: 500,
            currency: 1,
            count: -1,
            rank: 0,
        });

        let mask = after.changes_from(&before);
        let mut buffer = Vec::new();
        after.encode(mask, Some(&before), &mut Writer::new(&mut buffer));
        let read = EntityState::decode(Some(&before), &mut Reader::new(&buffer)).unwrap();
        assert_eq!(read.merchandise.unwrap().count, -1);
    }

    #[test]
    fn an_emptied_bag_is_told_apart_from_one_that_never_held_anything() {
        // Taking the last item out is a change the client has to see, and `None` and eight empty
        // slots have to encode differently or a bag would keep showing what was taken from it.
        let mut before = walking();
        before.contents = Some(Box::new([0x0904; 8]));

        let mut after = before.clone();
        after.contents = None;

        let mask = after.changes_from(&before);
        assert!(mask.has(FieldMask::CONTENTS));

        let mut buffer = Vec::new();
        after.encode(mask, Some(&before), &mut Writer::new(&mut buffer));
        let read = EntityState::decode(Some(&before), &mut Reader::new(&buffer)).unwrap();
        assert_eq!(read.contents, None);
    }

    #[test]
    fn what_equipment_adds_travels_apart_from_the_total() {
        // `Player.ExportStats` sends the eleven boosts beside the eleven totals
        // (`Player.cs:342-352`), and the character sheet draws them in green. Without them the
        // sheet cannot say how much of an attack is the ring, so taking an item off is a surprise.
        let mut before = walking();
        before.stats = [100, 100, 12, 0, 12, 15, 10, 10, 55, 90, 0];
        assert_eq!(before.boosts, [0; STAT_COUNT], "nothing worn adds nothing");

        // A ring that adds attack and a cursed one that takes defence away: both directions travel.
        let mut wearing = before.clone();
        wearing.stats[2] += 6;
        wearing.stats[3] -= 4;
        wearing.boosts[2] = 6;
        wearing.boosts[3] = -4;

        let mask = wearing.changes_from(&before);
        assert!(mask.has(FieldMask::BOOSTS));

        let (read, _) = round_trip(&wearing, Some(&before));
        assert_eq!(read.boosts[2], 6);
        assert_eq!(read.boosts[3], -4);

        // A stat that moved with nothing worn moving is a level-up, and costs no boost bytes.
        let mut levelled = wearing.clone();
        levelled.stats[2] += 1;
        assert!(!levelled.changes_from(&wearing).has(FieldMask::BOOSTS));
        assert!(levelled.changes_from(&wearing).has(FieldMask::STATS));
    }

    #[test]
    fn a_guild_and_its_rank_travel_together_and_can_be_left() {
        // `StatsType.Guild` and `StatsType.GuildRank` (`Player.cs:291-292`), which is what the
        // client writes under a player's own name.
        let unaffiliated = walking();
        assert_eq!(unaffiliated.guild, None);

        let mut joined = unaffiliated.clone();
        joined.guild = Some("The Deep".into());
        joined.guild_rank = 0;

        let mask = joined.changes_from(&unaffiliated);
        assert!(mask.has(FieldMask::GUILD));

        let (read, _) = round_trip(&joined, Some(&unaffiliated));
        assert_eq!(read.guild.as_deref(), Some("The Deep"));
        assert_eq!(read.guild_rank, 0);

        // A promotion is the same bit, because no rank exists apart from a guild.
        let mut promoted = joined.clone();
        promoted.guild_rank = 20;
        assert!(promoted.changes_from(&joined).has(FieldMask::GUILD));
        let (read, _) = round_trip(&promoted, Some(&joined));
        assert_eq!(read.guild_rank, 20);

        // And leaving has to clear it, or the name stays written over a head that left the guild.
        let (read, _) = round_trip(&unaffiliated, Some(&promoted));
        assert_eq!(read.guild, None);

        // Standing about is not a guild change, whatever else moves.
        let mut walked = promoted.clone();
        walked.x += 1.0;
        assert!(!walked.changes_from(&promoted).has(FieldMask::GUILD));
    }

    #[test]
    fn a_halo_travels_and_can_be_taken_off_again() {
        // `StatsType.Glow` (`Player.cs:302`), which the client hands to `GameObject.setGlow` and
        // `GlowRedrawer` paints around the sprite (`GlowRedrawer.as:19-46`). Without it on the wire
        // the `/glow` command sets a colour nobody can see.
        let plain = walking();
        assert_eq!(plain.glow, 0, "nobody glows by default");

        let mut lit = plain.clone();
        lit.glow = 0x00ff_7f00;

        let mask = lit.changes_from(&plain);
        assert!(mask.has(FieldMask::GLOW));

        let (read, _) = round_trip(&lit, Some(&plain));
        assert_eq!(read.glow, 0x00ff_7f00);

        // And `/glow 0` has to put it out, rather than leaving the last colour lit forever.
        let (read, _) = round_trip(&plain, Some(&lit));
        assert_eq!(read.glow, 0);

        // Standing about is not a colour change.
        let mut walked = lit.clone();
        walked.x += 1.0;
        assert!(!walked.changes_from(&lit).has(FieldMask::GLOW));
    }

    #[test]
    fn every_packed_flag_travels_and_none_of_them_bleeds_into_another() {
        // `StatsType.Admin` (`Player.cs:359`), `HasBackpack` (`:355`), `NameChosen` (`:299`) and
        // `PortalUsable` (`Portal.cs:57`) share one mask bit, so what matters is that a record
        // setting one does not set any of the others. Swept over all sixteen combinations, since a
        // bit written at the wrong shift shows up only against the right neighbour.
        let plain = walking();
        assert_eq!(
            (
                plain.admin,
                plain.has_backpack,
                plain.name_chosen,
                plain.portal_unusable
            ),
            (false, false, false, false),
            "an ordinary body carries no marks, and a portal defaults to usable"
        );

        let mut checked = 0usize;
        for bits in 0..16u8 {
            let mut marked = plain.clone();
            marked.admin = bits & 1 != 0;
            marked.has_backpack = bits & 2 != 0;
            marked.name_chosen = bits & 4 != 0;
            marked.portal_unusable = bits & 8 != 0;

            let mask = marked.changes_from(&plain);
            assert_eq!(mask.has(FieldMask::FLAGS), bits != 0);

            let (read, _) = round_trip(&marked, Some(&plain));
            assert_eq!(read.admin, marked.admin, "admin, bits {bits:04b}");
            assert_eq!(read.has_backpack, marked.has_backpack, "backpack, {bits:04b}");
            assert_eq!(read.name_chosen, marked.name_chosen, "name, {bits:04b}");
            assert_eq!(
                read.portal_unusable, marked.portal_unusable,
                "portal, {bits:04b}"
            );

            // None of them moves within a world, so none costs anything after the first sighting.
            assert!(!read.changes_from(&marked).has(FieldMask::FLAGS));
            checked += 1;
        }
        assert_eq!(checked, 16);
    }

    #[test]
    fn a_dye_travels_and_carries_its_type_byte_intact() {
        // `StatsType.Texture1`/`Texture2` (`Player.cs:301-302`). The top byte picks how the low
        // twenty-four are read -- `1` a solid colour, `4`/`5`/`9`/`10` a textile index
        // (`TextureRedrawer.as:161-186`) -- so a field that truncated to three bytes would turn
        // every dye in the game into a black one.
        let bare = walking();
        assert_eq!((bare.tex1, bare.tex2), (0, 0));

        // Alice Blue clothing dye, and a 9x9 textile as the accessory.
        let mut dyed = bare.clone();
        dyed.tex1 = 0x01F0_F8FF;
        dyed.tex2 = 0x0900_0007;

        let mask = dyed.changes_from(&bare);
        assert!(mask.has(FieldMask::DYES));

        let (read, _) = round_trip(&dyed, Some(&bare));
        assert_eq!(read.tex1, 0x01F0_F8FF, "the type byte survived");
        assert_eq!(read.tex2, 0x0900_0007);

        // Washing one off leaves the other, and costs the bit once.
        let mut washed = dyed.clone();
        washed.tex1 = 0;
        assert!(washed.changes_from(&dyed).has(FieldMask::DYES));
        let (read, _) = round_trip(&washed, Some(&dyed));
        assert_eq!((read.tex1, read.tex2), (0, 0x0900_0007));

        // And standing about in a dye is not a change.
        let mut walked = dyed.clone();
        walked.x += 1.0;
        assert!(!walked.changes_from(&dyed).has(FieldMask::DYES));
    }

    #[test]
    fn a_wall_says_which_neighbours_it_joins() {
        // `StatsType.ObjectConnection` (`ConnectedObject.cs:114-118`), which the client turns into
        // one of six connector shapes. The values are `ConnectionInfo`'s own, four bytes of 1 or 2,
        // and the top byte is set on every one of them -- so a field narrower than 32 bits, or one
        // written signed and read unsigned, loses a side of every wall.
        let loose = walking();
        assert_eq!(loose.connection, 0, "nothing joins onto anything by default");

        // Every `ConnectionInfo.Build` seed and its three rotations (`ConnectedObject.cs:20-25`).
        for seed in [
            0x0202_0202u32,
            0x0102_0202,
            0x0101_0202,
            0x0102_0102,
            0x0101_0201,
            0x0101_0101,
        ] {
            let mut bits = seed;
            for _ in 0..4 {
                let mut joined = loose.clone();
                joined.connection = bits;

                assert!(joined.changes_from(&loose).has(FieldMask::CONNECTION));
                let (read, _) = round_trip(&joined, Some(&loose));
                assert_eq!(read.connection, bits, "{bits:08x}");

                bits = (bits >> 8) | (bits << 24);
            }
        }
    }

    #[test]
    fn the_boost_clocks_travel_and_stop_costing_anything_once_they_run_out() {
        // `XPBoostTime`, `LDBoostTime`, `LTBoostTime` in seconds (`Player.cs:356-358`).
        let unboosted = walking();
        let mut boosted = unboosted.clone();
        boosted.experience_boost_seconds = 1800;
        boosted.loot_drop_boost_seconds = 600;
        boosted.loot_tier_boost_seconds = 0;

        assert!(boosted.changes_from(&unboosted).has(FieldMask::BOOST_TIME));
        let (read, _) = round_trip(&boosted, Some(&unboosted));
        assert_eq!(read.experience_boost_seconds, 1800);
        assert_eq!(read.loot_drop_boost_seconds, 600);
        assert_eq!(read.loot_tier_boost_seconds, 0);

        // A second passing moves one clock and raises the bit; a player with none raises nothing,
        // which is what keeps this off the wire for everybody who is not boosted.
        let mut ticked = boosted.clone();
        ticked.experience_boost_seconds -= 1;
        assert!(ticked.changes_from(&boosted).has(FieldMask::BOOST_TIME));
        assert!(!unboosted.changes_from(&unboosted).has(FieldMask::BOOST_TIME));

        // The original's XPBoost flag is derived rather than sent: `(XPBoostTime != 0) ? 1 : 0`.
        assert_eq!(read.experience_boost_seconds != 0, true);
    }

    #[test]
    fn the_fame_bars_ceiling_travels_beside_the_fame_itself() {
        // `StatsType.FameGoal` (`Player.cs:293`). It shares the levelling bit because
        // `HandleQuest` moves it from the same place the fame moves (`Player.Leveling.cs:242`), so
        // a fame gain that pushed past a threshold sends both or neither.
        let mut fresh = walking();
        fresh.fame = 46;
        fresh.fame_goal = 150;

        let (read, _) = round_trip(&fresh, None);
        assert_eq!((read.fame, read.fame_goal), (46, 150));

        // Earning past the threshold moves the ceiling, on the one bit.
        let mut earned = fresh.clone();
        earned.fame = 150;
        earned.fame_goal = 400;
        assert!(earned.changes_from(&fresh).has(FieldMask::PROGRESS));

        let (read, _) = round_trip(&earned, Some(&fresh));
        assert_eq!((read.fame, read.fame_goal), (150, 400));

        // And a ceiling that moves on its own -- which `/setfame` does -- still travels.
        let mut adjusted = earned.clone();
        adjusted.fame_goal = 2000;
        assert!(adjusted.changes_from(&earned).has(FieldMask::PROGRESS));
        assert_eq!(round_trip(&adjusted, Some(&earned)).0.fame_goal, 2000);
    }

    #[test]
    fn stats_travel_and_only_when_they_change() {
        let mut before = walking();
        before.stats = [100, 100, 12, 0, 12, 15, 10, 10, 55, 90, 0];

        let mut after = before.clone();
        assert_eq!(
            after.changes_from(&before),
            FieldMask::EMPTY,
            "nothing moved, so nothing is sent"
        );

        after.stats[2] = 18;
        let mask = after.changes_from(&before);
        assert!(mask.has(FieldMask::STATS));

        let mut buffer = Vec::new();
        after.encode(mask, Some(&before), &mut Writer::new(&mut buffer));
        let read = EntityState::decode(Some(&before), &mut Reader::new(&buffer)).unwrap();

        assert_eq!(read.stats, after.stats);
    }

    #[test]
    fn all_eleven_stats_cross_the_wire_including_the_three_no_class_declares() {
        // `Player.ExportStats` sends eleven, not eight: `DamageMin`, `DamageMax` and `Luck` are on
        // every update alongside the eight a class grows into (`Player.cs:331-341`). The first two
        // are the equipped weapon's own bounds, so a client that is not told them cannot say what a
        // shot will do; the last is the private-drop bonus.
        //
        // Swept one stat at a time so that a field dropped anywhere in the eleven shows up as
        // itself rather than as a decode failure, and with a negative value for each, since a stat
        // with enough taken off it now travels below zero.
        let mut combinations = 0usize;
        let mut mismatches = 0usize;

        for index in 0..STAT_COUNT {
            for value in [-4096, -197, -1, 0, 1, 7, 4096, i32::MAX, i32::MIN] {
                let before = walking();
                let mut after = before.clone();
                after.stats[index] = value;

                let mask = after.changes_from(&before);
                let mut buffer = Vec::new();
                after.encode(mask, Some(&before), &mut Writer::new(&mut buffer));
                let read = EntityState::decode(Some(&before), &mut Reader::new(&buffer)).unwrap();

                // A stat that moved has to raise the bit, and whatever crossed has to arrive
                // unchanged. A field dropped from the middle of the eleven fails the second of
                // those on some other stat, which is what makes this catch a miscount rather than
                // only a missing write.
                let announced = mask.has(FieldMask::STATS) || value == 0;

                combinations += 1;
                if !announced || read.stats != after.stats {
                    mismatches += 1;
                }
            }
        }

        assert_eq!(combinations, STAT_COUNT * 9);
        assert_eq!(mismatches, 0, "of {combinations} combinations");
    }

    #[test]
    fn a_texture_change_is_one_field_rather_than_a_new_entity() {
        // A boss changing phase keeps its type and its id; only what is drawn changes.
        let before = walking();
        let mut after = before.clone();
        after.texture = 3;

        let mask = after.changes_from(&before);
        assert!(mask.has(FieldMask::TEXTURE));
        assert!(!mask.has(FieldMask::OBJECT_TYPE));

        let mut buffer = Vec::new();
        after.encode(mask, Some(&before), &mut Writer::new(&mut buffer));
        assert_eq!(
            EntityState::decode(Some(&before), &mut Reader::new(&buffer))
                .unwrap()
                .texture,
            3
        );
    }

    use super::*;

    fn walking() -> EntityState {
        EntityState {
            object_type: 0x0a22,
            x: 103.5,
            y: 88.25,
            hp: 480,
            max_hp: 800,
            mp: 120,
            max_mp: 250,
            conditions: 0,
            size: 100,
            name: Some("Hobbit Mage".into()),
            texture: 0,
            stats: [0; STAT_COUNT],
            stars: 0,
            oxygen: 100,
            ..Default::default()
        }
    }

    #[test]
    fn stars_travel_and_only_when_they_change() {
        let mut wearing = walking();
        wearing.stars = 9;

        let (back, _) = round_trip(&wearing, None);
        assert_eq!(back.stars, 9);

        // Unchanged, so they cost nothing in a delta. A star moves when a character dies, which is
        // to say almost never, and paying a byte a tick for it would be paying for stillness.
        assert!(!wearing.changes_from(&wearing).has(FieldMask::STARS));

        let mut more = wearing.clone();
        more.stars = 10;
        assert!(more.changes_from(&wearing).has(FieldMask::STARS));

        let (back, _) = round_trip(&more, Some(&wearing));
        assert_eq!(back.stars, 10);
    }

    #[test]
    fn levelling_travels_and_only_when_it_changes() {
        let mut fresh = walking();
        fresh.level = 12;
        fresh.experience = 340;
        fresh.experience_goal = 1150;
        fresh.fame = 300;

        let (back, _) = round_trip(&fresh, None);
        assert_eq!((back.level, back.experience, back.fame), (12, 340, 300));
        assert_eq!(back.experience_goal, 1150);

        // Standing about earns nothing, so the bar costs nothing to leave where it is.
        assert!(!fresh.changes_from(&fresh).has(FieldMask::PROGRESS));

        let mut killed = fresh.clone();
        killed.experience += 95;
        assert!(killed.changes_from(&fresh).has(FieldMask::PROGRESS));

        let (back, _) = round_trip(&killed, Some(&fresh));
        assert_eq!(back.experience, 435);
        assert_eq!(
            back.level, 12,
            "the rest of the record came from the baseline"
        );
    }

    fn round_trip(current: &EntityState, baseline: Option<&EntityState>) -> (EntityState, usize) {
        let mask = match baseline {
            Some(from) => current.changes_from(from),
            None => FieldMask::ALL,
        };

        let mut buf = Vec::new();
        current.encode(mask, baseline, &mut Writer::new(&mut buf));

        let decoded = EntityState::decode(baseline, &mut Reader::new(&buf)).unwrap();
        (decoded, buf.len())
    }

    #[test]
    fn a_first_sighting_carries_every_field() {
        let entity = walking();
        let (decoded, _) = round_trip(&entity, None);
        assert_eq!(decoded, entity);
    }

    #[test]
    fn an_unchanged_entity_costs_a_single_byte() {
        let entity = walking();
        let (decoded, bytes) = round_trip(&entity, Some(&entity));
        assert_eq!(decoded, entity);
        assert_eq!(bytes, 1, "an empty mask is the whole record");
    }

    #[test]
    fn an_entity_that_only_moved_costs_three_bytes() {
        let before = walking();
        let mut after = before.clone();
        after.x += 0.125;
        after.y -= 0.125;

        let (decoded, bytes) = round_trip(&after, Some(&before));
        assert_eq!(quantize(decoded.x), quantize(after.x));
        assert_eq!(quantize(decoded.y), quantize(after.y));

        // One byte of mask plus one per axis.
        assert_eq!(bytes, 3);
    }

    #[test]
    fn sub_quantum_jitter_is_not_a_change() {
        let before = walking();
        let mut after = before.clone();

        // Far below one transmission step: the client already holds this value.
        after.x += 0.0001;
        after.y -= 0.0001;

        assert_eq!(after.changes_from(&before), FieldMask::EMPTY);
        let (_, bytes) = round_trip(&after, Some(&before));
        assert_eq!(bytes, 1);
    }

    #[test]
    fn unmentioned_fields_keep_their_baseline_values() {
        let before = walking();
        let mut after = before.clone();
        after.hp = 200;

        let (decoded, _) = round_trip(&after, Some(&before));
        assert_eq!(decoded.hp, 200);

        // Everything the mask did not name survived.
        assert_eq!(decoded.name, before.name);
        assert_eq!(decoded.max_hp, before.max_hp);
        assert_eq!(decoded.object_type, before.object_type);
        assert_eq!(quantize(decoded.x), quantize(before.x));
    }

    #[test]
    fn conditions_round_trip_at_full_width() {
        let before = walking();
        let mut after = before.clone();
        after.conditions = 1u128 << 120;

        let (decoded, _) = round_trip(&after, Some(&before));
        assert_eq!(decoded.conditions, 1u128 << 120);
    }

    #[test]
    fn a_name_can_be_added_and_cleared() {
        let mut anonymous = walking();
        anonymous.name = None;

        let named = walking();
        let (decoded, _) = round_trip(&named, Some(&anonymous));
        assert_eq!(decoded.name, named.name);

        let (decoded, _) = round_trip(&anonymous, Some(&named));
        assert_eq!(decoded.name, None);
    }

    #[test]
    fn a_busy_entity_still_beats_a_fixed_record() {
        let before = walking();
        let mut after = before.clone();
        after.x += 0.25;
        after.y += 0.25;
        after.hp -= 137;
        after.mp -= 40;
        after.conditions = 0b1010;

        let (_, bytes) = round_trip(&after, Some(&before));

        // Five changed fields. A fixed record would spend four bytes on each of the
        // twenty-odd fields an entity has, whether they changed or not.
        assert!(bytes < 16, "{bytes} bytes for five changed fields");
    }

    #[test]
    fn a_truncated_record_errors_rather_than_panicking() {
        let before = walking();
        let mut after = before.clone();
        after.hp = 1;
        after.name = Some("a considerably longer name".into());

        let mask = after.changes_from(&before);
        let mut buf = Vec::new();
        after.encode(mask, Some(&before), &mut Writer::new(&mut buf));

        for cut in 0..buf.len() {
            let mut reader = Reader::new(&buf[..cut]);
            let _ = EntityState::decode(Some(&before), &mut reader);
        }
    }
}

//! The eleven stats, and what they are worth.
//!
//! Every class declares a starting value, a ceiling and a per-level range for each of the first
//! eight. Together they decide how fast a character moves, how hard it hits and how often it can
//! fire, which is what separates a warrior from a wizard.
//!
//! # The other three
//!
//! `DamageMin`, `DamageMax` and `Luck` are stats in the same array (`StatsManager.NumStatTypes` is
//! 11) but no class declares them and no character saves them. `DamageMin` and `DamageMax` are the
//! equipped weapon's first projectile, written into the base layer on every recalculation by
//! `BaseStatManager.SetWeaponDamage`, and they are what a player's shot rolls between. `Luck` has
//! no base at all — the original writes one only through `ImportStats`, which nothing calls for a
//! player — and is read in exactly one place, the private half of the loot roll.
//!
//! # Three layers, kept apart
//!
//! A stat is a base, plus what equipment adds, plus what temporary boosts add. They are held
//! separately rather than summed into one number because taking a ring off has to subtract exactly
//! what it added, and a single total cannot say what that was. The alternative, recomputing from
//! the inventory on every change, is what the old server did and is why its stats drifted.
//!
//! # What a stat is worth
//!
//! Every formula and constant below is read from the original server's `StatsManager`. The
//! condition effects that change them are applied inside the same expression that reads the stat,
//! as they are there: Weak does not halve damage, it holds attack at its minimum.

use hendra_content::{PlayerDesc, STAT_COUNT, STATS, Stat};

use crate::effects::Rules;

/// One character's stats, in three layers.
///
/// `equipment` and `boosts` are together the original's single `Boost` layer: it sums worn items,
/// completed sets and timed boosts into one array (`BoostStatManager.ReCalculateValues`), and
/// splitting the durable half from the temporary half is what lets a boost lapse without having to
/// know what a ring was adding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// What levelling has produced, plus the weapon's damage in slots 8 and 9. Only the first
    /// eight persist.
    base: [i32; STAT_COUNT],

    /// What worn equipment adds. Rebuilt whenever equipment changes.
    equipment: [i32; STAT_COUNT],

    /// What temporary effects add.
    boosts: [i32; STAT_COUNT],
}

impl Stats {
    /// Something that cannot move under its own power, such as a wall or a sign.
    pub fn still() -> Stats {
        let mut stats = Stats::default();
        stats.base[Stat::Speed.index()] = NO_MOVEMENT;
        stats
    }

    /// A character at the start of its class.
    pub fn starting(class: &PlayerDesc) -> Stats {
        let mut base = [0i32; STAT_COUNT];
        for stat in STATS {
            base[stat.index()] = class.stat(stat).starting;
        }

        Stats {
            base,
            equipment: [0; STAT_COUNT],
            boosts: [0; STAT_COUNT],
        }
    }

    /// Restores stats from stored base values, which is how a character keeps what it levelled into.
    ///
    /// A slice of the wrong length is read as far as it goes and zero after, so a stored row that
    /// predates a stat cannot fail to load. That is `Utils.ResizeArray(Character.Stats,
    /// NumStatTypes)`, which the original calls for the same reason.
    pub fn from_base(stored: &[i32]) -> Stats {
        let mut base = [0i32; STAT_COUNT];
        for (slot, value) in base.iter_mut().zip(stored) {
            *slot = *value;
        }

        Stats {
            base,
            equipment: [0; STAT_COUNT],
            boosts: [0; STAT_COUNT],
        }
    }

    /// The eight base values, in stat order, for storing.
    ///
    /// Eight rather than eleven because the other three have no base worth keeping: the weapon's
    /// damage is recomputed from what is held every time anything changes, and nothing writes a
    /// base luck at all.
    pub fn to_base(&self) -> [i32; 8] {
        let mut out = [0i32; 8];
        out.copy_from_slice(&self.base[..8]);
        out
    }

    /// The same stats with the equipped weapon's damage written into the base layer.
    ///
    /// `BaseStatManager.SetWeaponDamage` reads `Inventory[0].Projectiles[0]` and writes its
    /// `MinDamage` and `MaxDamage` into base slots 8 and 9 on every recalculation, so the pair is
    /// always the weapon currently held rather than anything remembered. Taken at the moment a shot
    /// is made rather than cached on the body for the same reason: a cached copy is one missed
    /// equip away from being wrong, and the original never lets it get stale either.
    pub fn armed_with(&self, weapon_min: i32, weapon_max: i32) -> Stats {
        let mut armed = *self;
        armed.base[Stat::DamageMin.index()] = weapon_min;
        armed.base[Stat::DamageMax.index()] = weapon_max;
        armed
    }

    /// Writes the equipped weapon's damage into the base layer and keeps it there.
    ///
    /// The same two slots [`Self::armed_with`] fills, written when the equipment changes rather
    /// than when a shot is fired, because that is when `SetWeaponDamage` runs and the two stats are
    /// on every update a player receives: a client that is not told them has no damage to show on a
    /// character sheet.
    pub fn arm(&mut self, weapon_min: i32, weapon_max: i32) {
        self.base[Stat::DamageMin.index()] = weapon_min;
        self.base[Stat::DamageMax.index()] = weapon_max;
    }

    /// The base value, before equipment or boosts.
    pub fn base(&self, stat: Stat) -> i32 {
        self.base[stat.index()]
    }

    /// The total, which is what everything else asks for.
    ///
    /// `StatsManager.this[i]` is `Base[i] + Boost[i]` and nothing more (`StatsManager.cs:23`):
    /// uncapped above, and unfloored below. Equipment and boosts reach past a class's maximum,
    /// which is what late-game equipment is for, and a stat with enough taken off it lands below
    /// zero. The only floor the original has is the per-bonus one in [`Self::apply_equipment`].
    pub fn total(&self, stat: Stat) -> i32 {
        let index = stat.index();
        self.base[index] + self.equipment[index] + self.boosts[index]
    }

    /// Raises the base, refusing to go past the class's ceiling.
    pub fn raise(&mut self, class: &PlayerDesc, stat: Stat, by: i32) {
        let index = stat.index();
        let ceiling = class.stat(stat).maximum;
        self.base[index] = (self.base[index] + by).clamp(0, ceiling.max(0));
    }

    /// Writes the base outright, with no ceiling.
    ///
    /// `AEFixedStat` (`Player.UseItem.cs:645-649`) assigns `Stats.Base[idx] = eff.Amount` and stops
    /// there: no clamp against the class's maximum, and no overflow into a boost the way
    /// [`Self::raise`] has. An item that sets a stat past what the class allows sets it past it.
    pub fn set_base(&mut self, stat: Stat, amount: i32) {
        self.base[stat.index()] = amount;
    }

    /// Replaces what equipment contributes.
    ///
    /// Replaced wholesale rather than adjusted. Tracking which item added what and undoing it on
    /// removal is the bookkeeping that drifts.
    pub fn set_equipment(&mut self, boosts: [i32; STAT_COUNT]) {
        self.equipment = boosts;
    }

    /// Replaces what equipment contributes, from the bonuses one at a time.
    ///
    /// Each bonus is floored on its own way in rather than the sum being floored afterwards, which
    /// is what `IncrementBoost` does: a bonus that would take `Base[i] + amount` below one is
    /// shortened to leave the stat at exactly zero — or at one for maximum health, the only stat
    /// held above nothing (`BoostStatManager.cs:168-171`).
    ///
    /// Two consequences follow, and both are visible. A single item that would take a level-one
    /// wizard's 100 health to -10 leaves it at 1 rather than at 0, because the shortening is
    /// applied to the bonus and not to the result. And because each bonus is measured against the
    /// base alone rather than against what the bonuses before it already took off, two items that
    /// each empty the same stat take it off twice: an attack of 12 under two -60 rings is -12, not
    /// 0.
    ///
    /// The order the bonuses arrive in does not matter, since none of them can see another.
    pub fn apply_equipment(&mut self, bonuses: &[(Stat, i32)]) {
        let mut layer = [0i32; STAT_COUNT];

        for (stat, amount) in bonuses {
            let index = stat.index();
            let base = self.base[index];

            let amount = if base + amount < 1 {
                if index == Stat::MaxHitPoints.index() {
                    1 - base
                } else {
                    -base
                }
            } else {
                *amount
            };

            layer[index] += amount;
        }

        self.equipment = layer;
    }

    /// Adds a temporary boost.
    pub fn boost(&mut self, stat: Stat, amount: i32) {
        self.boosts[stat.index()] += amount;
    }

    /// What is added on top of the base, for the wire and for the character sheet.
    ///
    /// Equipment and timed boosts together, because that is the original's single `Boost` layer:
    /// `BoostStatManager.ReCalculateValues` sums worn items, completed sets and running boosts into
    /// one array, and `Player.ExportStats` sends that array (`Player.cs:342-352`). Splitting them is
    /// this server's own bookkeeping, so it is put back together on the way out.
    pub fn boost_totals(&self) -> [i32; STAT_COUNT] {
        let mut out = [0i32; STAT_COUNT];
        for index in 0..STAT_COUNT {
            out[index] = self.equipment[index] + self.boosts[index];
        }
        out
    }

    /// Sets what temporary boosts come to, having already been stacked.
    ///
    /// Set rather than added, because the answer is a function of what is held: recomputing it from
    /// the list every time a boost is given or lapses is what makes a lapse take the right amount
    /// away rather than whatever was added last.
    pub fn set_boosts(&mut self, boosts: [i32; STAT_COUNT]) {
        self.boosts = boosts;
    }

    /// What temporary boosts currently come to.
    pub fn boosts(&self) -> [i32; STAT_COUNT] {
        self.boosts
    }

    pub fn clear_boosts(&mut self) {
        self.boosts = [0; STAT_COUNT];
    }

    /// What each stat is at, for the wire and for a character sheet.
    ///
    /// All eleven, as `Player.ExportStats` sends all eleven: the eight a class declares plus
    /// `DamageMin`, `DamageMax` and `Luck` (`Player.cs:331-341`). The last three are constant in
    /// this content — nothing grants a bonus to any of them and no class declares one — but the
    /// first two carry the equipped weapon's damage, which changes every time the weapon does.
    pub fn totals(&self) -> [i32; STAT_COUNT] {
        let mut out = [0i32; STAT_COUNT];
        for stat in hendra_content::ALL_STATS {
            out[stat.index()] = self.total(stat);
        }
        out
    }

    // -- what the stats are worth -------------------------------------------------------------

    /// Multiplies weapon damage.
    ///
    /// `MinAttackMult + (attack / 75) * (MaxAttackMult - MinAttackMult)`, held at the minimum by
    /// Weak and multiplied by Damaging.
    pub fn damage_multiplier(&self, rules: &Rules) -> f32 {
        if rules.weak {
            return MIN_ATTACK_MULT;
        }

        let attack = self.total(Stat::Attack) as f32;
        let mult = MIN_ATTACK_MULT + (attack / STAT_SCALE) * (MAX_ATTACK_MULT - MIN_ATTACK_MULT);

        if rules.damaging { mult * 1.5 } else { mult }
    }

    /// What one shot does, given a roll in `0.0..1.0`.
    ///
    /// `GetAttackDamage(Stats[8], Stats[9], isAbility)`: a value drawn between the two damage stats
    /// and multiplied by the attack multiplier, truncated rather than rounded and with no floor of
    /// one. A weapon whose minimum is zero can therefore do nothing at all on a bad roll, which is
    /// what the original does.
    ///
    /// An ability is not multiplied: `GetAttackMult` returns 1 before it looks at anything, so
    /// neither attack nor Weak nor Damaging reaches a nova or a spell bomb.
    pub fn attack_damage(&self, rules: &Rules, roll: f32, is_ability: bool) -> i32 {
        let rolled = roll_between(
            self.total(Stat::DamageMin),
            self.total(Stat::DamageMax),
            roll,
        );

        if is_ability {
            return rolled;
        }

        (rolled as f32 * self.damage_multiplier(rules)) as i32
    }

    /// How much likelier this character is to receive a private drop, as a multiplier.
    ///
    /// `1 + Stats.Boost[10] / 100` (`Loots.cs:117`). The boost layer alone, not the total: the
    /// original names `Boost` there rather than the indexer, and since nothing ever writes a base
    /// luck the two agree in this content and would part company only for content that did.
    ///
    /// It multiplies the private roll and nothing else. Public loot, the required drops that are
    /// forced out afterwards, and which bag they land in are all untouched by it.
    pub fn loot_multiplier(&self) -> f64 {
        1.0 + self.luck_boost() as f64 / 100.0
    }

    /// What worn items, sets and timed boosts have added to luck.
    pub fn luck_boost(&self) -> i32 {
        let index = Stat::Luck.index();
        self.equipment[index] + self.boosts[index]
    }

    /// Shots per millisecond.
    ///
    /// A frequency rather than a cooldown, matching `GetAttackFrequency`. Dazed holds it at the
    /// minimum and Berserk multiplies it.
    pub fn attack_frequency(&self, rules: &Rules) -> f32 {
        if rules.dazed {
            return MIN_ATTACK_FREQ;
        }

        let dexterity = self.total(Stat::Dexterity) as f32;
        let frequency =
            MIN_ATTACK_FREQ + (dexterity / STAT_SCALE) * (MAX_ATTACK_FREQ - MIN_ATTACK_FREQ);

        if rules.berserk {
            frequency * 1.5
        } else {
            frequency
        }
    }

    /// Milliseconds between shots from a weapon of a given rate of fire.
    ///
    /// The weapon's own rate is a multiplier on the character's frequency, so a fast weapon in a
    /// dextrous hand compounds rather than replacing it.
    ///
    /// Divided in the order `ValidatePlayerShoot` divides it — `(int)(1 / frequency * 1 /
    /// RateOfFire)`, which is the reciprocal of the frequency divided by the rate rather than the
    /// reciprocal of their product. The two are the same number in arithmetic and not always the
    /// same float, and a millisecond either way is a shot either way at the edge of a cooldown.
    pub fn shot_cooldown_ms(&self, rules: &Rules, weapon_rate: f32) -> u32 {
        let frequency = self.attack_frequency(rules);
        if frequency <= 0.0 {
            return u32::MAX;
        }
        (1.0 / frequency / weapon_rate.max(0.01)).clamp(1.0, 60_000.0) as u32
    }

    /// Tiles per second.
    ///
    /// `4 + 5.6 * (speed / 75)`. Slowed holds it at the base rather than scaling it, and Paralyzed
    /// is handled by the caller refusing the move outright.
    pub fn movement_speed(&self, rules: &Rules) -> f32 {
        if rules.rooted {
            return 0.0;
        }
        if rules.slowed {
            return BASE_SPEED;
        }

        let speed = self.total(Stat::Speed) as f32;
        let ret = BASE_SPEED + SPEED_RANGE * (speed / STAT_SCALE);

        if rules.speedy {
            (ret * 1.5).max(0.0)
        } else {
            ret.max(0.0)
        }
    }

    /// Health regained per second.
    ///
    /// `6 + vitality * 0.12`. Sick zeroes the vitality but not the base, so a sick character still
    /// recovers slowly rather than not at all.
    pub fn health_regen(&self, rules: &Rules) -> f32 {
        let vitality = if rules.sick {
            0.0
        } else {
            self.total(Stat::HpRegen) as f32
        };
        BASE_HP_REGEN + vitality * HP_REGEN_PER_POINT
    }

    /// Magic regained per second. Quiet stops it entirely.
    pub fn magic_regen(&self, rules: &Rules) -> f32 {
        if rules.no_magic_regen {
            return 0.0;
        }
        BASE_MP_REGEN + (self.total(Stat::MpRegen) as f32) * MP_REGEN_PER_POINT
    }

    pub fn max_hp(&self) -> i32 {
        self.total(Stat::MaxHitPoints)
    }

    pub fn max_mp(&self) -> i32 {
        self.total(Stat::MaxMagicPoints)
    }

    pub fn defence(&self) -> i32 {
        self.total(Stat::Defense)
    }
}

/// The stat value every formula is scaled against.
const STAT_SCALE: f32 = 75.0;

/// The damage multiplier at zero attack, and at the scale value.
const MIN_ATTACK_MULT: f32 = 0.5;
const MAX_ATTACK_MULT: f32 = 2.0;

/// Shots per millisecond at zero dexterity, and at the scale value.
const MIN_ATTACK_FREQ: f32 = 0.0015;
const MAX_ATTACK_FREQ: f32 = 0.008;

/// Tiles per second at zero speed, and how much the scale value adds.
const BASE_SPEED: f32 = 4.0;
const SPEED_RANGE: f32 = 5.6;

/// Health per second at zero vitality, and what one point adds.
const BASE_HP_REGEN: f32 = 6.0;
const HP_REGEN_PER_POINT: f32 = 0.12;

/// Magic per second at zero wisdom, and what one point adds.
const BASE_MP_REGEN: f32 = 0.5;
const MP_REGEN_PER_POINT: f32 = 0.06;

/// The speed value that means "does not move".
///
/// Below the point where the formula returns zero, so a wall cannot drift.
const NO_MOVEMENT: i32 = -((BASE_SPEED / SPEED_RANGE * STAT_SCALE) as i32) - 1;

/// A value drawn between two bounds, given a roll in `0.0..1.0`.
///
/// `wRandom.NextIntRange` is `min == max ? min : min + Gen() % (max - min)`, so the maximum is
/// exclusive: a 55-90 weapon rolls 55 to 89. Equal bounds are returned rather than taken modulo
/// zero, which is the guard the original needs and keeps for the same reason.
fn roll_between(min: i32, max: i32, roll: f32) -> i32 {
    if max <= min {
        return min;
    }
    min + (roll * (max - min) as f32) as i32
}

/// What a set of worn items contributes, one bonus at a time.
///
/// Taken from what is worn rather than accumulated as items move, so the answer never depends on
/// having seen every change. A missed equip cannot leave a stat permanently wrong.
///
/// Kept apart rather than summed because that is the shape the original's floor needs:
/// `ApplyEquipBonus` walks the four worn slots and hands each `StatsBoost` entry to `IncrementBoost`
/// separately (`BoostStatManager.cs:46-56`), which floors each one on its own. Summing first and
/// flooring after is a different answer whenever two items take from the same stat.
pub fn equipment_bonuses<'a>(
    worn: impl Iterator<Item = &'a hendra_content::ItemDesc>,
) -> Vec<(Stat, i32)> {
    let mut out = Vec::new();
    for item in worn {
        for boost in &item.stat_boosts {
            // An unrecognised stat is dropped rather than applied to whichever slot its number
            // lands on, which is `IncrementBoost` returning on a -1 index (`BoostStatManager.cs:165`).
            if let Some(stat) = hendra_content::ALL_STATS.get(boost.stat as usize) {
                out.push((*stat, boost.amount));
            }
        }
    }
    out
}

/// What a list of bonuses comes to, summed and unfloored.
pub fn summed(bonuses: &[(Stat, i32)]) -> [i32; STAT_COUNT] {
    let mut out = [0i32; STAT_COUNT];
    for (stat, amount) in bonuses {
        out[stat.index()] += amount;
    }
    out
}

/// What a set of temporary boosts on one stat comes to.
///
/// Follows `ActivateBoost.GetBoost`. Boosts do not simply add: sorted, the largest counts in full,
/// the next at a half, the next at a quarter, and so on. Two rings of eight attack are worth twelve
/// rather than sixteen, which is why stacking the same buff is worth less each time and why a
/// second one is worth having at all.
///
/// Non-stacking boosts are separate and only the largest of them counts, which is what stops two
/// copies of a buff that says it does not stack from stacking. The original seeds that list with a
/// zero (`ActivateBoost` starts `_base` as `{ 0 }`) and reads its largest element, so a
/// non-stacking boost that lowers a stat is worth nothing rather than lowering it.
pub fn stacked(stacking: &[i32], separate: &[i32]) -> i32 {
    let mut sorted: Vec<i32> = stacking.to_vec();

    // Largest first, because the discount is applied by position and the largest is meant to be the
    // one that counts in full.
    sorted.sort_unstable_by(|a, b| b.cmp(a));

    let mut total = 0i32;
    for (index, amount) in sorted.iter().enumerate() {
        // Halving each time. Past a handful this is zero, which is the point: piling on more of the
        // same buff stops being worth anything.
        let share = 0.5f64.powi(index as i32);
        total += (*amount as f64 * share) as i32;
    }

    total + separate.iter().copied().chain([0]).max().unwrap_or(0)
}

#[cfg(test)]
mod stacking {
    use super::stacked;

    #[test]
    fn one_boost_counts_in_full() {
        assert_eq!(stacked(&[8], &[]), 8);
    }

    #[test]
    fn each_boost_after_the_largest_counts_for_half_of_the_last() {
        // Two rings of eight attack are worth twelve rather than sixteen, which is why stacking the
        // same buff is worth less each time and why a second one is worth having at all.
        assert_eq!(stacked(&[8, 8], &[]), 12);
        assert_eq!(stacked(&[8, 8, 8], &[]), 14);
        assert_eq!(stacked(&[8, 8, 8, 8], &[]), 15);
    }

    #[test]
    fn the_largest_is_the_one_that_counts_in_full_whatever_order_they_arrived_in() {
        // Sorted rather than taken as given, or a small boost arriving first would take the full
        // share and the large one behind it would be halved.
        assert_eq!(stacked(&[2, 20], &[]), stacked(&[20, 2], &[]));
        assert_eq!(stacked(&[2, 20], &[]), 21);
    }

    #[test]
    fn piling_on_more_of_the_same_stops_being_worth_anything() {
        let many = vec![10; 20];
        let few = vec![10; 6];

        // Past a handful the halving reaches zero, which is the point of the rule.
        assert_eq!(stacked(&many, &[]), stacked(&few, &[]));
    }

    #[test]
    fn the_kind_that_does_not_stack_takes_only_its_largest() {
        assert_eq!(stacked(&[], &[5, 5, 5]), 5);
        assert_eq!(stacked(&[], &[3, 9, 1]), 9);
    }

    #[test]
    fn a_non_stacking_boost_that_lowers_a_stat_is_worth_nothing() {
        // The list it is chosen from starts with a zero in it, so the largest is never below zero.
        assert_eq!(stacked(&[], &[-7]), 0);
        assert_eq!(stacked(&[], &[-7, 4]), 4);
    }

    #[test]
    fn the_two_kinds_are_added_to_each_other() {
        // They are separate rules rather than one pool: a stacking buff and an aura are both worth
        // having at once.
        assert_eq!(stacked(&[8, 8], &[10]), 22);
    }

    #[test]
    fn nothing_held_is_nothing() {
        assert_eq!(stacked(&[], &[]), 0);
    }

    #[test]
    fn a_negative_boost_still_counts_against_you() {
        // Some content lowers a stat, and sorting largest-first means the least bad one counts in
        // full. That is what the original does with them, and it is the merciful reading.
        assert_eq!(stacked(&[-10], &[]), -10);
        assert_eq!(stacked(&[-10, -10], &[]), -15);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hendra_content::Node;

    const WIZARD: &str = r#"
<Objects>
   <Object type="0x030e" id="Wizard">
      <Player/>
      <MaxHitPoints max="670">100</MaxHitPoints>
      <MaxMagicPoints max="385">100</MaxMagicPoints>
      <Attack max="75">12</Attack>
      <Defense max="25">0</Defense>
      <Speed max="50">12</Speed>
      <Dexterity max="75">15</Dexterity>
      <HpRegen max="40">10</HpRegen>
      <MpRegen max="60">10</MpRegen>
      <LevelIncrease min="20" max="30">MaxHitPoints</LevelIncrease>
   </Object>
</Objects>"#;

    fn wizard() -> PlayerDesc {
        let document = Node::parse(WIZARD).unwrap();
        let node = document.children_named("Object").next().unwrap();
        PlayerDesc::parse(node, hendra_content::ObjectType(0x030e)).unwrap()
    }

    #[test]
    fn a_fresh_character_starts_where_its_class_says() {
        let stats = Stats::starting(&wizard());

        assert_eq!(stats.total(Stat::MaxHitPoints), 100);
        assert_eq!(stats.total(Stat::Attack), 12);
        assert_eq!(stats.total(Stat::Defense), 0);
    }

    #[test]
    fn a_stat_cannot_be_levelled_past_its_class_ceiling() {
        let class = wizard();
        let mut stats = Stats::starting(&class);

        for _ in 0..500 {
            stats.raise(&class, Stat::Attack, 5);
        }

        assert_eq!(stats.base(Stat::Attack), 75, "the wizard's maximum");
    }

    #[test]
    fn equipment_reaches_past_the_ceiling_that_levelling_cannot() {
        // The whole point of late-game equipment. Capping the total would make a ring worthless to
        // exactly the characters who earned it.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        for _ in 0..500 {
            stats.raise(&class, Stat::Attack, 5);
        }

        let mut boosts = [0i32; STAT_COUNT];
        boosts[Stat::Attack.index()] = 6;
        stats.set_equipment(boosts);

        assert_eq!(stats.total(Stat::Attack), 81);
        assert_eq!(stats.base(Stat::Attack), 75, "the base is still capped");
    }

    #[test]
    fn taking_equipment_off_returns_every_stat_to_exactly_where_it_was() {
        // Drift is the risk: a stat that creeps up by a point each time something is equipped and
        // unequipped, which nobody notices until a character is wrong.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let before = stats.totals();

        let mut seed = 0x1234_5678u32;
        for round in 0..1_000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);

            let mut boosts = [0i32; STAT_COUNT];
            for slot in boosts.iter_mut() {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                *slot = (seed % 21) as i32 - 10;
            }
            stats.set_equipment(boosts);
            stats.set_equipment([0; STAT_COUNT]);

            assert_eq!(stats.totals(), before, "drifted on round {round}");
        }
    }

    #[test]
    fn two_items_boosting_the_same_stat_stack() {
        let ring = hendra_content::ItemDesc {
            stat_boosts: vec![hendra_content::StatBoost {
                stat: Stat::Dexterity as u8,
                amount: 6,
            }],
            ..Default::default()
        };

        let boosts = summed(&equipment_bonuses([&ring, &ring].into_iter()));
        assert_eq!(boosts[Stat::Dexterity.index()], 12);
    }

    #[test]
    fn a_stat_boost_naming_something_that_is_not_a_stat_is_ignored() {
        // The content has a few of these. Indexing an array with it would panic, and taking the
        // modulus would silently boost the wrong stat.
        let odd = hendra_content::ItemDesc {
            stat_boosts: vec![hendra_content::StatBoost {
                stat: 200,
                amount: 50,
            }],
            ..Default::default()
        };

        assert_eq!(equipment_bonuses([&odd].into_iter()), Vec::new());
    }

    /// `BoostStatManager.IncrementBoost` and `StatsManager.this[i]`, transliterated.
    ///
    /// The whole of what the original does with a set of worn bonuses: zero the layer, hand each
    /// bonus in on its own with the floor applied to it rather than to the running total, and read
    /// the answer back as base plus layer with nothing clamped
    /// (`BoostStatManager.cs:33-56,168-173`, `StatsManager.cs:23`).
    fn original(base: [i32; STAT_COUNT], bonuses: &[(Stat, i32)]) -> [i32; STAT_COUNT] {
        let mut boost = [0i32; STAT_COUNT];

        for (stat, amount) in bonuses {
            let i = stat.index();
            let mut amount = *amount;
            if base[i] + amount < 1 {
                amount = if i == 0 { -base[i] + 1 } else { -base[i] };
            }
            boost[i] += amount;
        }

        let mut out = [0i32; STAT_COUNT];
        for i in 0..STAT_COUNT {
            out[i] = base[i] + boost[i];
        }
        out
    }

    #[test]
    fn every_pair_of_bonuses_lands_where_the_original_lands_it() {
        // The floor goes under each bonus on its way in rather than under the sum afterwards, and
        // the two only agree while a stat carries at most one bonus deep enough to reach it. Swept
        // across every stat, a spread of bases and every ordered pair of amounts, because the
        // interesting cases are exactly the ones the shipped content does not contain.
        let bases = [0, 1, 2, 12, 20, 50, 100, 140, 200];
        let amounts = [
            -300, -201, -200, -199, -140, -110, -100, -99, -60, -50, -20, -13, -2, -1, 0, 1, 5, 20,
            50, 120,
        ];

        let mut combinations = 0usize;
        let mut mismatches = 0usize;

        for stat in hendra_content::ALL_STATS {
            for start in bases {
                let mut base = [0i32; STAT_COUNT];
                base[stat.index()] = start;

                for first in amounts {
                    for second in amounts {
                        for pair in [vec![(stat, first)], vec![(stat, first), (stat, second)]] {
                            let mut stats = Stats::from_base(&base);
                            stats.apply_equipment(&pair);

                            let mine = stats.totals();
                            let theirs = original(base, &pair);

                            combinations += 1;
                            if mine != theirs {
                                mismatches += 1;
                            }
                        }
                    }
                }
            }
        }

        assert_eq!(combinations, 11 * 9 * 20 * 20 * 2);
        assert_eq!(mismatches, 0, "of {combinations} combinations");
    }

    #[test]
    fn one_item_that_would_empty_the_health_bar_leaves_a_single_point() {
        // `IncrementBoost` shortens the bonus rather than clamping the result, and maximum health
        // is the one stat it leaves above nothing (`BoostStatManager.cs:168-171`). A level-one
        // wizard in an item worth -110 health has one, not none.
        let class = wizard();
        let mut stats = Stats::starting(&class);
        let base = stats.base(Stat::MaxHitPoints);

        stats.apply_equipment(&[(Stat::MaxHitPoints, -(base + 10))]);
        assert_eq!(stats.max_hp(), 1);

        // Every other stat is left at exactly nothing by the same arithmetic.
        let mut stats = Stats::starting(&class);
        let attack = stats.base(Stat::Attack);
        stats.apply_equipment(&[(Stat::Attack, -(attack + 10))]);
        assert_eq!(stats.total(Stat::Attack), 0);
    }

    #[test]
    fn two_items_emptying_one_stat_take_it_below_nothing() {
        // Each bonus is measured against the base alone, never against what the one before it
        // already took off, so the shortening happens twice and the total goes negative. The
        // original has no floor under the sum at all: `this[i]` is `Base[i] + Boost[i]`
        // (`StatsManager.cs:23`).
        let mut stats = Stats::from_base(&[100, 100, 12, 0, 12, 15, 10, 10]);

        stats.apply_equipment(&[(Stat::Attack, -60), (Stat::Attack, -60)]);
        assert_eq!(stats.total(Stat::Attack), -12);

        // And the same for health, where each bonus is shortened to leave one rather than none.
        stats.apply_equipment(&[(Stat::MaxHitPoints, -200), (Stat::MaxHitPoints, -200)]);
        assert_eq!(stats.max_hp(), 100 + 2 * (1 - 100));
    }

    #[test]
    fn a_bonus_that_does_not_reach_the_floor_is_applied_whole() {
        let mut stats = Stats::from_base(&[100, 100, 12, 0, 12, 15, 10, 10]);
        stats.apply_equipment(&[(Stat::Defense, 25), (Stat::MaxHitPoints, -99)]);

        assert_eq!(stats.total(Stat::Defense), 25);
        assert_eq!(stats.max_hp(), 1, "exactly reaching one is not shortened");
    }

    #[test]
    fn more_attack_means_more_damage_and_more_dexterity_means_faster() {
        let none = Rules::NONE;
        let class = wizard();
        let plain = Stats::starting(&class);

        let mut strong = plain;
        strong.boost(Stat::Attack, 50);
        assert!(strong.damage_multiplier(&none) > plain.damage_multiplier(&none));

        let mut quick = plain;
        quick.boost(Stat::Dexterity, 50);
        assert!(
            quick.shot_cooldown_ms(&none, 1.0) < plain.shot_cooldown_ms(&none, 1.0),
            "a shorter wait between shots is faster"
        );
    }

    #[test]
    fn the_damage_multiplier_spans_the_range_the_original_uses() {
        let none = Rules::NONE;
        let mut stats = Stats::default();

        assert!((stats.damage_multiplier(&none) - MIN_ATTACK_MULT).abs() < 0.001);

        stats.boost(Stat::Attack, STAT_SCALE as i32);
        assert!((stats.damage_multiplier(&none) - MAX_ATTACK_MULT).abs() < 0.001);
    }

    #[test]
    fn being_weak_holds_attack_at_the_minimum_rather_than_halving_it() {
        // `return MinAttackMult`, not a multiplier. On a character with high attack the difference
        // is large, and halving would leave it far above where the original puts it.
        let mut stats = Stats::default();
        stats.boost(Stat::Attack, 75);

        let weakened = Rules {
            weak: true,
            ..Rules::NONE
        };
        assert!((stats.damage_multiplier(&weakened) - MIN_ATTACK_MULT).abs() < 0.001);
        assert!(stats.damage_multiplier(&Rules::NONE) > MIN_ATTACK_MULT * 2.0);
    }

    #[test]
    fn being_dazed_holds_rate_of_fire_at_the_minimum() {
        let mut stats = Stats::default();
        stats.boost(Stat::Dexterity, 75);

        let dazed = Rules {
            dazed: true,
            ..Rules::NONE
        };
        assert!(stats.shot_cooldown_ms(&dazed, 1.0) > stats.shot_cooldown_ms(&Rules::NONE, 1.0));
        assert_eq!(
            stats.shot_cooldown_ms(&dazed, 1.0),
            (1.0 / MIN_ATTACK_FREQ) as u32
        );
    }

    #[test]
    fn being_slowed_holds_speed_at_the_base_rather_than_scaling_it() {
        // `ret = 4`, not `ret *= 0.5`. A fast character slowed drops to the same speed as a slow
        // one slowed, which is what the original does and what the content is balanced against.
        let mut fast = Stats::default();
        fast.boost(Stat::Speed, 75);
        let slow = Stats::default();

        let slowed = Rules {
            slowed: true,
            ..Rules::NONE
        };
        assert_eq!(fast.movement_speed(&slowed), BASE_SPEED);
        assert_eq!(slow.movement_speed(&slowed), BASE_SPEED);
        assert!(fast.movement_speed(&Rules::NONE) > BASE_SPEED);
    }

    #[test]
    fn a_rooted_character_has_no_speed_at_all() {
        let mut stats = Stats::default();
        stats.boost(Stat::Speed, 75);

        let rooted = Rules {
            rooted: true,
            ..Rules::NONE
        };
        assert_eq!(stats.movement_speed(&rooted), 0.0);
    }

    #[test]
    fn being_sick_zeroes_vitality_without_stopping_regeneration_entirely() {
        // `vit = 0` leaves the base of six. A sick character recovers slowly rather than not at all.
        let mut stats = Stats::default();
        stats.boost(Stat::HpRegen, 60);

        let sick = Rules {
            sick: true,
            ..Rules::NONE
        };
        assert_eq!(stats.health_regen(&sick), BASE_HP_REGEN);
        assert!(stats.health_regen(&Rules::NONE) > BASE_HP_REGEN);
    }

    #[test]
    fn quiet_stops_magic_returning_entirely() {
        let mut stats = Stats::default();
        stats.boost(Stat::MpRegen, 60);

        let quiet = Rules {
            no_magic_regen: true,
            ..Rules::NONE
        };
        assert_eq!(stats.magic_regen(&quiet), 0.0);
        assert!(stats.magic_regen(&Rules::NONE) > 0.0);
    }

    #[test]
    fn a_character_with_nothing_in_a_stat_is_not_useless() {
        // Zero attack must not mean zero damage, or a fresh character could not kill anything.
        let none = Rules::NONE;
        let empty = Stats::default();

        assert!(empty.damage_multiplier(&none) > 0.0);
        assert!(empty.shot_cooldown_ms(&none, 1.0) > 0);
        assert!(empty.movement_speed(&none) > 0.0);
    }

    #[test]
    fn something_that_cannot_move_stays_where_it_is() {
        assert_eq!(Stats::still().movement_speed(&Rules::NONE), 0.0);
    }

    #[test]
    fn boosts_can_be_cleared_without_touching_the_other_layers() {
        let class = wizard();
        let mut stats = Stats::starting(&class);

        let mut worn = [0i32; STAT_COUNT];
        worn[Stat::Speed.index()] = 5;
        stats.set_equipment(worn);
        stats.boost(Stat::Speed, 20);

        assert_eq!(stats.total(Stat::Speed), 12 + 5 + 20);

        stats.clear_boosts();
        assert_eq!(stats.total(Stat::Speed), 17, "equipment and base remain");
    }

    #[test]
    fn a_timed_boost_has_no_floor_under_it_at_all() {
        // The floor in the original lives in `IncrementBoost`, which only worn items and completed
        // sets go through. `ApplyActivateBonus` adds the timed boosts straight into the same array
        // with no test of any kind (`BoostStatManager.cs:122-128`), and the total is read back as
        // `Base[i] + Boost[i]` (`StatsManager.cs:23`). A large enough negative boost takes the stat
        // below nothing, which is what a debuff aura in content that had one would do.
        let mut stats = Stats::starting(&wizard());
        let base = stats.base(Stat::Attack);
        stats.boost(Stat::Attack, -1_000);

        assert_eq!(stats.total(Stat::Attack), base - 1_000);
    }

    #[test]
    fn the_weapon_damage_stats_are_the_weapon_and_whatever_is_worn_on_top_of_it() {
        // `SetWeaponDamage` writes the first projectile of what is held into base slots 8 and 9, and
        // `DamageMinBonus` / `DamageMaxBonus` land in the boost layer over it. Nothing in this
        // server's content grants either bonus, so in practice the pair is the weapon alone -- but
        // the stat is what the shot reads, not the descriptor, and content that granted one would
        // be felt.
        let mut stats = Stats::default();
        let armed = stats.armed_with(55, 90);
        assert_eq!(armed.total(Stat::DamageMin), 55);
        assert_eq!(armed.total(Stat::DamageMax), 90);

        stats.boost(Stat::DamageMin, 10);
        stats.boost(Stat::DamageMax, 20);
        let armed = stats.armed_with(55, 90);
        assert_eq!(armed.total(Stat::DamageMin), 65);
        assert_eq!(armed.total(Stat::DamageMax), 110);
    }

    #[test]
    fn a_shot_rolls_between_the_bounds_and_never_reaches_the_top_one() {
        // Half-open, as `min + Gen() % (max - min)` is. A 55-90 weapon rolls 55 to 89.
        let stats = Stats::default().armed_with(55, 90);
        let mut seen = [false; 35];

        for step in 0..3_500 {
            let roll = step as f32 / 3_500.0;
            let damage = stats.attack_damage(&Rules::NONE, roll, true);
            assert!((55..90).contains(&damage), "{damage} is outside 55..90");
            seen[(damage - 55) as usize] = true;
        }

        assert!(
            seen.iter().all(|hit| *hit),
            "every value in the span occurs"
        );
    }

    #[test]
    fn a_weapon_whose_bounds_are_equal_needs_no_roll_at_all() {
        // The original guards `min == max` before taking a modulus, because the modulus would be
        // zero. Ours has to answer the same thing rather than divide by nothing.
        let stats = Stats::default().armed_with(80, 80);
        for step in 0..100 {
            assert_eq!(
                stats.attack_damage(&Rules::NONE, step as f32 / 100.0, true),
                80
            );
        }
    }

    #[test]
    fn an_ability_is_not_multiplied_by_attack_at_all() {
        // `GetAttackMult` returns 1 for an ability before it looks at anything, so a wizard's spell
        // does the same damage as a fresh character's.
        let mut strong = Stats::default();
        strong.boost(Stat::Attack, 75);
        let strong = strong.armed_with(100, 200);
        let weak = Stats::default().armed_with(100, 200);

        for step in 0..200 {
            let roll = step as f32 / 200.0;
            assert_eq!(
                strong.attack_damage(&Rules::NONE, roll, true),
                weak.attack_damage(&Rules::NONE, roll, true)
            );
        }
    }

    // -- swept against a transliteration of the original -----------------------------------------

    /// `StatsManager.GetAttackMult`, transliterated.
    fn cs_attack_mult(attack: i32, weak: bool, damaging: bool, is_ability: bool) -> f32 {
        if is_ability {
            return 1.0;
        }
        if weak {
            return 0.5f32;
        }
        let mut mult = 0.5f32 + (attack as f32 / 75f32) * (2f32 - 0.5f32);
        if damaging {
            mult *= 1.5f32;
        }
        mult
    }

    /// `wRandom.NextIntRange`, transliterated. The third argument stands in for what `Gen()` returned.
    fn cs_next_int_range(min: u32, max: u32, generated: u32) -> u32 {
        if min == max {
            min
        } else {
            min + generated % (max - min)
        }
    }

    /// `StatsManager.GetAttackDamage`, transliterated, ending in C#'s truncating `(int)` cast.
    #[allow(clippy::too_many_arguments)]
    fn cs_attack_damage(
        min: i32,
        max: i32,
        generated: u32,
        attack: i32,
        weak: bool,
        damaging: bool,
        is_ability: bool,
    ) -> i32 {
        let rolled = cs_next_int_range(min as u32, max as u32, generated);
        (rolled as f32 * cs_attack_mult(attack, weak, damaging, is_ability)) as i32
    }

    #[test]
    fn shot_damage_matches_the_original_across_the_whole_space() {
        // The technique that settled the attack multiplier, applied to the two damage stats: every
        // combination of bounds, offset within the span, attack and the two conditions that reach
        // the multiplier, against a transliteration of the two C# methods that produce the number.
        //
        // The offset is swept rather than the raw generator output because the two servers draw
        // from different streams: `Gen() % span` and `(roll * span) as i32` both produce every
        // offset in `0..span`, and it is the arithmetic downstream of the draw that is under test.
        let bounds = [
            (0, 0),
            (0, 1),
            (0, 30),
            (1, 2),
            (5, 6),
            (55, 90),
            (100, 101),
            (101, 202),
            (220, 275),
            (350, 350),
            (1, 1_000),
        ];
        let attacks = [0, 1, 3, 7, 12, 25, 37, 50, 63, 75, 90, 120, 250];
        let flags = [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, false),
            (false, false, true),
            (true, false, true),
        ];

        let mut compared = 0usize;
        let mut mismatched = 0usize;

        for (min, max) in bounds {
            let span = (max - min).max(1);
            for offset in 0..span.min(64) {
                // Sits in the middle of the offset's slice of `0.0..1.0`, so the truncation on our
                // side lands on exactly the offset the C# side is handed.
                let roll = (offset as f32 + 0.5) / span as f32;

                for attack in attacks {
                    for (weak, damaging, is_ability) in flags {
                        let mut stats = Stats::default();
                        stats.boost(Stat::Attack, attack);
                        let stats = stats.armed_with(min, max);

                        let rules = Rules {
                            weak,
                            damaging,
                            ..Rules::NONE
                        };

                        let ours = stats.attack_damage(&rules, roll, is_ability);
                        let theirs = cs_attack_damage(
                            min,
                            max,
                            offset as u32,
                            attack,
                            weak,
                            damaging,
                            is_ability,
                        );

                        compared += 1;
                        if ours != theirs {
                            mismatched += 1;
                            if mismatched < 10 {
                                eprintln!(
                                    "{min}..{max} offset {offset} attack {attack} \
                                     weak {weak} damaging {damaging} ability {is_ability}: \
                                     ours {ours}, theirs {theirs}"
                                );
                            }
                        }
                    }
                }
            }
        }

        assert!(compared > 5_000, "swept {compared} combinations");
        assert_eq!(mismatched, 0, "{mismatched} of {compared} disagreed");
    }

    #[test]
    fn the_wait_between_shots_matches_the_original_across_the_whole_space() {
        // `ValidatePlayerShoot` computes `(int)(1 / Stats.GetAttackFrequency() * 1 /
        // item.RateOfFire)`. Every rate of fire the content declares, against every dexterity a
        // character can reach and the two conditions that reach the frequency.
        let rates = [
            0.25f32, 0.3, 0.33, 0.4, 0.5, 0.6, 0.7, 0.75, 0.8, 0.85, 0.9, 1.0, 1.09, 1.1, 1.15,
            1.2, 1.25, 1.3, 1.5, 1.6, 2.0,
        ];

        let mut compared = 0usize;
        let mut mismatched = 0usize;

        for dexterity in 0..=100i32 {
            for (dazed, berserk) in [(false, false), (true, false), (false, true), (true, true)] {
                let mut stats = Stats::default();
                stats.boost(Stat::Dexterity, dexterity);

                let rules = Rules {
                    dazed,
                    berserk,
                    ..Rules::NONE
                };

                // `GetAttackFrequency`, transliterated.
                let frequency = if dazed {
                    0.0015f32
                } else {
                    let rof = 0.0015f32 + (dexterity as f32 / 75f32) * (0.008f32 - 0.0015f32);
                    if berserk { rof * 1.5f32 } else { rof }
                };

                for rate in rates {
                    let theirs = (1f32 / frequency * 1f32 / rate) as i32;
                    let ours = stats.shot_cooldown_ms(&rules, rate) as i32;

                    compared += 1;
                    if ours != theirs {
                        mismatched += 1;
                        if mismatched < 10 {
                            eprintln!(
                                "dex {dexterity} dazed {dazed} berserk {berserk} rate {rate}: \
                                 ours {ours}, theirs {theirs}"
                            );
                        }
                    }
                }
            }
        }

        assert!(compared > 5_000, "swept {compared} combinations");
        assert_eq!(mismatched, 0, "{mismatched} of {compared} disagreed");
    }

    #[test]
    fn luck_multiplies_the_private_loot_roll_by_a_hundredth_of_itself() {
        // `1 + Stats.Boost[10] / 100.0`, and nothing else in the server reads the stat.
        let mut stats = Stats::default();
        assert_eq!(stats.loot_multiplier(), 1.0);

        stats.boost(Stat::Luck, 25);
        assert_eq!(stats.luck_boost(), 25);
        assert!((stats.loot_multiplier() - 1.25).abs() < 1e-12);
    }

    #[test]
    fn luck_is_read_from_the_boost_layer_and_not_from_the_base() {
        // The original names `Stats.Boost[10]` rather than `Stats[10]`. Nothing writes a base luck,
        // so the two agree in this content -- but the reader is the boost layer, and a server that
        // read the total would pay out differently the moment anything wrote one.
        let mut stats = Stats::from_base(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 40]);
        assert_eq!(stats.luck_boost(), 0, "a base luck is not a boost");

        let mut worn = [0i32; STAT_COUNT];
        worn[Stat::Luck.index()] = 40;
        stats.set_equipment(worn);
        assert_eq!(stats.luck_boost(), 40, "a worn one is");
    }
}

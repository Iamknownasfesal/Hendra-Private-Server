//! Which enemy a player is pointed at.
//!
//! Follows `Player.HandleQuest`, whose table names a hundred and twenty enemies and gives each a
//! priority and a level range. What it points at is not the nearest thing worth killing but the most
//! worthwhile thing near enough to be worth walking to, which is a different answer and the reason
//! the arrow is useful at all.
//!
//! # How one is chosen
//!
//! `(20 - |enemy level - your level|) * priority - distance / 100`, from the original. Three things
//! pull against each other: something suited to your level beats something that is not, something
//! important beats something ordinary, and something close beats something far. Distance is divided
//! by a hundred, so a whole realm's width is worth about twenty points and a priority of one is
//! worth twenty: near enough that neither always wins.
//!
//! # Why a range as well as a score
//!
//! Because a level-one character should not be pointed at a god. The range is a hard filter and the
//! score only orders what is left.

/// One enemy worth pointing at: how important it is, and who it suits.
pub struct Quest {
    pub name: &'static str,

    /// How much it matters. Multiplied by how well it suits, so a high priority is worth more the
    /// closer the level match.
    pub priority: i32,

    /// The levels it is meant for, inclusive at both ends.
    pub from_level: i16,
    pub to_level: i16,
}

/// The table, from `Player.Leveling.QuestDat`.
const QUESTS: &[(&str, i32, i16, i16)] = &[
    ("Scorpion Queen", 1, 1, 6),
    ("Bandit Leader", 1, 1, 6),
    ("Hobbit Mage", 3, 3, 8),
    ("Undead Hobbit Mage", 3, 3, 8),
    ("Giant Crab", 3, 3, 8),
    ("Desert Werewolf", 3, 3, 8),
    ("Sandsman King", 4, 4, 9),
    ("Goblin Mage", 4, 4, 9),
    ("Elf Wizard", 4, 4, 9),
    ("Dwarf King", 5, 5, 10),
    ("Swarm", 6, 6, 11),
    ("Shambling Sludge", 6, 6, 11),
    ("Great Lizard", 7, 7, 12),
    ("Wasp Queen", 8, 7, 20),
    ("Horned Drake", 8, 7, 20),
    ("Deathmage", 5, 6, 11),
    ("Great Coil Snake", 6, 6, 12),
    ("Lich", 8, 6, 20),
    ("Actual Lich", 8, 7, 20),
    ("Ent Ancient", 9, 7, 20),
    ("Actual Ent Ancient", 9, 7, 20),
    ("Oasis Giant", 10, 8, 20),
    ("Phoenix Lord", 10, 9, 20),
    ("Ghost King", 11, 10, 20),
    ("Actual Ghost King", 11, 10, 20),
    ("Cyclops God", 12, 10, 20),
    ("Kage Kami", 12, 10, 20),
    ("Red Demon", 13, 15, 20),
    ("The Magicial lord of sky", 15, 1, 20),
    ("LH Sentry", 15, 1, 20),
    ("shtrs Defense System", 14, 15, 20),
    ("Fanatic of Chaos", 14, 15, 20),
    ("Skull Shrine", 14, 15, 20),
    ("Pentaract", 14, 15, 20),
    ("Cube God", 14, 15, 20),
    ("Grand Sphinx", 14, 15, 20),
    ("Lord of the Lost Lands", 14, 15, 20),
    ("Hermit God", 14, 15, 20),
    ("Ghost Ship", 14, 15, 20),
    ("Dragon Head", 14, 15, 20),
    ("Lucky Ent God", 14, 15, 20),
    ("Lucky Djinn", 14, 15, 20),
    ("Zombie Horde", 14, 15, 20),
    ("Evil Chicken God", 15, 1, 20),
    ("Bonegrind the Butcher", 15, 1, 20),
    ("Dreadstump the Pirate King", 15, 1, 20),
    ("Mama Megamoth", 15, 1, 20),
    ("Arachna the Spider Queen", 15, 1, 20),
    ("Stheno the Snake Queen", 15, 1, 20),
    ("Mixcoatl the Masked God", 15, 1, 20),
    ("Limon the Sprite God", 15, 1, 20),
    ("Septavius the Ghost God", 15, 1, 20),
    ("Davy Jones", 15, 1, 20),
    ("Lord Ruthven", 15, 1, 20),
    ("Archdemon Malphas", 15, 1, 20),
    ("Thessal the Mermaid Goddess", 15, 1, 20),
    ("Dr Terrible", 15, 1, 20),
    ("Horrific Creation", 15, 1, 20),
    ("Masked Party God", 15, 1, 20),
    ("Oryx Stone Guardian Left", 15, 1, 20),
    ("Oryx Stone Guardian Right", 15, 1, 20),
    ("Oryx the Mad God 1", 15, 1, 20),
    ("Oryx the Mad God 2", 15, 1, 20),
    ("Oryx the Mad God 3", 15, 1, 20),
    ("Oryx the Mad God 4", 15, 1, 20),
    ("Gigacorn", 15, 1, 20),
    ("Desire Troll", 15, 1, 20),
    ("Spoiled Creampuff", 15, 1, 20),
    ("MegaRototo", 15, 1, 20),
    ("Swoll Fairy", 15, 1, 20),
    ("BedlamGod", 15, 1, 20),
    ("Troll 3", 15, 1, 20),
    ("Arena Ghost Bride", 15, 1, 20),
    ("Arena Statue Left", 15, 1, 20),
    ("Arena Statue Right", 15, 1, 20),
    ("Arena Grave Caretaker", 15, 1, 20),
    ("Ghost of Skuld", 15, 1, 20),
    ("Tomb Defender", 15, 1, 20),
    ("Tomb Support", 15, 1, 20),
    ("Tomb Attacker", 15, 1, 20),
    ("Active Sarcophagus", 15, 1, 20),
    ("shtrs Bridge Sentinel", 15, 1, 20),
    ("shtrs The Forgotten King", 15, 1, 20),
    ("shtrs Twilight Archmage", 15, 1, 20),
    ("NM Black Dragon God", 15, 1, 20),
    ("NM Black Dragon God Hardmode", 15, 1, 20),
    ("NM Red Dragon God", 15, 1, 20),
    ("NM Red Dragon God Hardmode", 15, 1, 20),
    ("NM Blue Dragon God", 15, 1, 20),
    ("NM Blue Dragon God Hardmode", 15, 1, 20),
    ("NM Green Dragon God", 15, 1, 20),
    ("NM Green Dragon God Hardmode", 15, 1, 20),
    ("lod Ivory Wyvern", 15, 1, 20),
    ("The Puppet Master", 15, 1, 20),
    ("Jon Bilgewater the Pirate King", 15, 1, 20),
    ("Epic Larva", 15, 1, 20),
    ("Epic Mama Megamoth", 15, 1, 20),
    ("Murderous Megamoth", 15, 1, 20),
    ("Son of Arachna", 15, 1, 20),
    ("Golden Oryx Effigy", 15, 1, 20),
    ("Murderous Megamoth Deux", 15, 1, 20),
    ("Lord Ruthven Deux", 15, 1, 20),
    ("NM Green Dragon God Deux", 15, 1, 20),
    ("Archdemon Malphas Deux", 15, 1, 20),
    ("Stheno the Snake Queen Deux", 15, 1, 20),
    ("Golden Oryx Effigy Deux", 15, 1, 20),
    ("Oryx the Mad God Deux", 15, 1, 20),
    ("vlntns Botany Bella", 15, 1, 20),
    ("md1 Head of Shaitan", 15, 1, 20),
    ("Queen of Hearts", 15, 1, 20),
    ("Fabian the King of the Ossis", 15, 1, 20),
    ("TestChicken 2", 15, 1, 20),
    ("Golem King", 15, 1, 20),
    ("LH Marble Colossus", 15, 1, 20),
    ("Barriel Guy", 15, 1, 20),
    ("Bandit Robber Rat", 15, 1, 20),
    ("Megaman", 50, 20, 20),
    ("Boshy", 50, 20, 20),
    ("The Kid", 50, 20, 20),
    ("Sanic", 50, 20, 20),
];

/// What one candidate scores for a player at this level and distance.
///
/// Higher is better. Negative is possible and fine: something far enough away scores below
/// something near, which is the point.
pub fn score(priority: i32, enemy_level: i16, player_level: i16, distance: f32) -> i32 {
    let suits = 20 - (enemy_level - player_level).abs() as i32;
    suits * priority - (distance / 100.0) as i32
}

/// What the table says about an enemy, if anything.
pub fn quest_for(name: &str) -> Option<Quest> {
    QUESTS.iter().find(|(held, _, _, _)| *held == name).map(
        |(name, priority, from_level, to_level)| Quest {
            name,
            priority: *priority,
            from_level: *from_level,
            to_level: *to_level,
        },
    )
}

/// Whether a quest suits a player at this level.
pub fn suits(quest: &Quest, level: i16) -> bool {
    level >= quest.from_level && level <= quest.to_level
}

/// How many enemies the table names.
pub fn known() -> usize {
    QUESTS.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_table_is_here() {
        // A hundred and twenty in the original. A table half-copied is an arrow that points at the
        // wrong thing for whichever half was dropped.
        assert_eq!(known(), 120);
    }

    #[test]
    fn a_level_one_character_is_not_pointed_at_a_god() {
        // The range is a hard filter rather than part of the score, or a high enough priority would
        // send a beginner to something that kills them.
        let lich = quest_for("Lich").expect("the lich is in the table");
        assert!(!suits(&lich, 1));
        assert!(suits(&lich, 10));

        let scorpion = quest_for("Scorpion Queen").expect("the scorpion is in the table");
        assert!(suits(&scorpion, 1));
        assert!(!suits(&scorpion, 15), "a beginner's quest at level fifteen");
    }

    #[test]
    fn something_suited_to_your_level_beats_something_that_is_not() {
        // Same priority, same distance: the one nearer your level wins.
        let near = score(5, 10, 10, 0.0);
        let far = score(5, 20, 10, 0.0);

        assert!(near > far, "{near} vs {far}");
    }

    #[test]
    fn something_important_beats_something_ordinary() {
        let important = score(9, 10, 10, 0.0);
        let ordinary = score(1, 10, 10, 0.0);

        assert!(important > ordinary);
    }

    #[test]
    fn something_close_beats_the_same_thing_far_away() {
        let close = score(5, 10, 10, 0.0);
        let away = score(5, 10, 10, 2000.0);

        assert!(close > away, "{close} vs {away}");
        assert_eq!(close - away, 20, "a realm's width is worth about twenty");
    }

    #[test]
    fn distance_does_not_drown_out_importance() {
        // A hundred tiles is worth one point, so a priority of nine still beats a priority of one
        // standing next to you. Both pulls matter, which is what makes the arrow worth following.
        let important_far = score(9, 10, 10, 1000.0);
        let ordinary_here = score(1, 10, 10, 0.0);

        assert!(
            important_far > ordinary_here,
            "{important_far} vs {ordinary_here}"
        );
    }

    /// The names the original's table has that this content does not ship.
    ///
    /// Kept rather than deleted, so the table stays the original's and the difference is a fact
    /// somebody can read. Each was checked by hand: none is a near-miss on capitals, which is what
    /// two of the shop names turned out to be.
    const NOT_IN_THIS_CONTENT: &[&str] = &[
        "Fanatic of Chaos",
        "Oryx the Mad God 3",
        "Oryx the Mad God 4",
        "BedlamGod",
        "Queen of Hearts",
        "Fabian the King of the Ossis",
        "Megaman",
        "Boshy",
        "The Kid",
        "Sanic",
    ];

    #[test]
    fn every_name_in_the_table_is_in_the_content_or_known_not_to_be() {
        // A name the content does not have is an enemy the arrow can never point at. Ten of the
        // original's are missing here and are listed above; an eleventh is a typo, and this is what
        // tells the two apart.
        let Ok((catalog, _)) = hendra_content::Catalog::load_dir(std::path::Path::new(
            "../../../godot-client/assets/xml",
        )) else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let missing: Vec<&str> = QUESTS
            .iter()
            .map(|(name, _, _, _)| *name)
            .filter(|name| catalog.type_of(name).is_none())
            .filter(|name| !NOT_IN_THIS_CONTENT.contains(name))
            .collect();

        assert!(missing.is_empty(), "not in the content: {missing:?}");
    }

    #[test]
    fn the_names_known_to_be_absent_really_are_absent() {
        // The other direction. If a content drop adds one, this fails and says the exception can go.
        let Ok((catalog, _)) = hendra_content::Catalog::load_dir(std::path::Path::new(
            "../../../godot-client/assets/xml",
        )) else {
            return;
        };

        let arrived: Vec<&str> = NOT_IN_THIS_CONTENT
            .iter()
            .copied()
            .filter(|name| catalog.type_of(name).is_some())
            .collect();

        assert!(arrived.is_empty(), "the content now has {arrived:?}");
    }

    #[test]
    fn every_range_is_the_right_way_round() {
        for (name, priority, from, to) in QUESTS {
            assert!(from <= to, "{name} runs from {from} to {to}");
            assert!(*priority > 0, "{name} has no priority");
        }
    }
}

//! What the shops sell, and where they stand.
//!
//! Follows `wServer/realm/entities/vendors/MerchantLists.cs`, which keeps the same table in code.
//! Each shop is a region the map marks, and one merchant is placed on every square of that region:
//! eight `Store_1` squares in the nexus is eight weapon merchants, each holding one thing.
//!
//! # Why the stock is a list rather than a slot
//!
//! A shop holds more items than it has squares, so the list is dealt out around them and wraps. That
//! is what the original does, and it is why walking along a row of merchants shows different things.

use hendra_content::Region;

/// What a shop takes payment in.
///
/// Named here rather than taken from the store, because the simulation knows about worlds and not
/// about databases. The server maps this onto the durable one when it takes the payment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    Gold,
    Fame,
}

/// One shop: where it stands, what it takes, and what it holds.
pub struct Shop {
    pub region: Region,
    pub currency: Currency,

    /// How many stars somebody needs to buy here at all.
    ///
    /// Stars rather than any other standing, as `SellableObject.ValidateCustomer` has it: a shop
    /// that asks for a rank is asking what you have done with a character, not what you have been
    /// given.
    pub rank: i16,

    /// What it sells, as `(name, price)`, in the order it is dealt out.
    pub stock: &'static [(&'static str, i32)],
}

/// Where a player's own listings stand in the marketplace, by what they are.
///
/// From `Market.GetItemType`, which maps a region to a kind of item so a marketplace has a row for
/// weapons, a row for abilities and so on rather than one heap.
pub const MARKET_ROWS: &[(Region, &str)] = &[
    (Region::Store9, "Weapon"),
    (Region::Store10, "Ability"),
    (Region::Store11, "Armor"),
    (Region::Store12, "Ring"),
    (Region::Store13, "Potion"),
    (Region::Store14, "Other"),
];

/// Every shop, from `MerchantLists.Shops`.
/// Whether somebody may buy from a shop that asks for a rank.
///
/// `SellableObject.ValidateCustomer` compares the requirement against the player's stars, which is
/// what makes the gate mean something: five stars is a character taken to two thousand fame, and no
/// amount of money or time buys it.
///
/// An administrator is admitted regardless, since being unable to look at a shop is not a useful
/// thing to be unable to do.
pub fn admits(shop_rank: i16, stars: u8, admin_rank: i16) -> bool {
    shop_rank <= 0 || stars as i16 >= shop_rank || admin_rank >= shop_rank
}

pub const SHOPS: &[Shop] = &[
    Shop {
        region: Region::Store1,
        currency: Currency::Fame,
        rank: 0,
        stock: &[
            ("Dagger of Foul Malevolence", 500),
            ("Bow of Covert Havens", 500),
            ("Staff of the Cosmic Whole", 500),
            ("Wand of Recompense", 500),
            ("Sword of Acclaim", 500),
            ("Masamune", 500),
        ],
    },
    Shop {
        region: Region::Store2,
        currency: Currency::Fame,
        rank: 0,
        stock: &[
            ("Cloak of Ghostly Concealment", 500),
            ("Quiver of Elvish Mastery", 500),
            ("Elemental Detonation Spell", 500),
            ("Tome of Holy Guidance", 500),
            ("Helm of the Great General", 500),
            ("Colossus Shield", 500),
            ("Seal of the Blessed Champion", 500),
            ("Baneserpent Poison", 500),
            ("Bloodsucker Skull", 500),
            ("Giantcatcher Trap", 500),
            ("Planefetter Orb", 500),
            ("Prism of Apparitions", 500),
            ("Scepter of Storms", 500),
            ("Doom Circle", 500),
        ],
    },
    Shop {
        region: Region::Store3,
        currency: Currency::Fame,
        rank: 0,
        stock: &[
            ("Robe of the Grand Sorcerer", 600),
            ("Hydra Skin Armor", 600),
            ("Acropolis Armor", 600),
        ],
    },
    Shop {
        region: Region::Store4,
        currency: Currency::Fame,
        rank: 0,
        stock: &[
            ("Ring of Paramount Attack", 1000),
            ("Ring of Paramount Defense", 1000),
            ("Ring of Paramount Speed", 1000),
            ("Ring of Paramount Dexterity", 1000),
            ("Ring of Paramount Vitality", 1000),
            ("Ring of Paramount Wisdom", 1000),
            ("Ring of Paramount Health", 1000),
            ("Ring of Paramount Magic", 1000),
            ("Ring of Unbound Attack", 750),
            ("Ring of Unbound Defense", 900),
            ("Ring of Unbound Speed", 750),
            ("Ring of Unbound Dexterity", 900),
            ("Ring of Unbound Vitality", 750),
            ("Ring of Unbound Wisdom", 750),
            ("Ring of Unbound Health", 1000),
            ("Ring of Unbound Magic", 1000),
        ],
    },
    Shop {
        region: Region::Store5,
        currency: Currency::Fame,
        rank: 0,
        stock: &[
            ("Undead Lair Key", 200),
            ("Sprite World Key", 200),
            ("Abyss of Demons Key", 200),
            ("Ocean Trench Key", 200),
            ("Snake Pit Key", 200),
            ("Lost Halls Key", 800),
            ("Tomb of the Ancients Key", 500),
        ],
    },
    Shop {
        region: Region::Store6,
        currency: Currency::Fame,
        rank: 5,
        stock: &[
            ("50 Fame", 50),
            ("100 Fame", 100),
            ("500 Fame", 500),
            ("1000 Fame", 1000),
            ("5000 Fame", 5000),
        ],
    },
    Shop {
        region: Region::Store7,
        currency: Currency::Fame,
        rank: 0,
        stock: &[
            ("XP Booster", 0),
            ("Loot Drop Potion", 1500),
            ("Backpack", 1000),
        ],
    },
    Shop {
        region: Region::Store8,
        currency: Currency::Fame,
        rank: 0,
        stock: &[("Amulet of Resurrection", 50000)],
    },
    Shop {
        region: Region::Store21,
        currency: Currency::Gold,
        rank: 0,
        stock: &[
            ("Claymore of Eternal Light", 500),
            ("Helm of the Heavenly Guard", 500),
            ("Vault of the Skies", 500),
            ("Ring of Deep Radiance", 500),
            ("Maxy", 250),
            ("Health Maxy", 50),
            ("Wisdom Maxy", 50),
            ("Vitality Maxy", 50),
            ("Dexterity Maxy", 50),
            ("Defense Maxy", 50),
            ("Mana Maxy", 50),
            ("Speed Maxy", 50),
            ("Attack Maxy", 50),
            ("Staff of the Phoenix Lord", 500),
            ("Elven Tablet of the Blood Moon", 500),
            ("Robe of the Elven Highlord", 500),
            ("Ring of the Golden Sun", 500),
            ("Wand of Dark Philosophies", 500),
            ("Scepter of the Dark Descent", 500),
            ("Robe of Foreboding Signs", 500),
            ("Crown of the Insane Alchemist", 500),
            ("Gladiator Sword", 500),
            ("Minotaur's Waraxe", 500),
            ("Champions breastplate", 500),
            ("Gladiator Trophy", 500),
            ("Titus's Shield", 500),
            ("Gladiator Trophy", 500),
            ("Titus's Shield", 500),
            ("Horn of Magical", 500),
            ("Sky Dagger", 500),
            ("Falling Star", 500),
            ("Golem's Axe", 900),
            ("Golem's Robe", 600),
            ("Sky Katana", 800),
            ("Veil of the ancient oceans", 500),
            ("Pink Robe", 500),
            ("Pink Tome", 500),
            ("Septavius Ghost Robe", 500),
            ("Pink Wand", 500),
            ("Pink Ring", 500),
            ("Royality Ring of Depth Oceans", 500),
            ("Thessal's Hide", 500),
            ("Sharped corals of the oceans", 500),
            ("Ghost Cannon", 500),
            ("Ghostly Armor", 500),
            ("Ghostly trap", 500),
            ("Revenge Ring", 500),
            ("Barriel's Enchanted Spear", 800),
            ("Leaf Amulet", 500),
            ("Wooden Helm", 500),
            ("Wooden Hide Armor", 500),
            ("Staff of blood", 700),
            ("Staff of Green Posion", 600),
            ("Robber's Gun", 1000),
            ("Puppet Rainbow Dagger", 500),
            ("Stheno Quiver", 600),
            ("Sword of the Spirit Walker", 500),
            ("Claymore of Eternal Light", 10000),
            ("Helm of the Heavenly Guard", 10000),
            ("Vault of the Skies", 10000),
            ("Ring of Deep Radiance", 10000),
            ("Maxy", 10000),
            ("Health Maxy", 5000),
            ("Wisdom Maxy", 5000),
            ("Vitality Maxy", 5000),
            ("Dexterity Maxy", 5000),
            ("Defense Maxy", 5000),
            ("Mana Maxy", 5000),
            ("Speed Maxy", 5000),
            ("Attack Maxy", 5000),
            ("Staff of the Phoenix Lord", 5000),
            ("Elven Tablet of the Blood Moon", 5000),
            ("Robe of the Elven Highlord", 5000),
            ("Ring of the Golden Sun", 5000),
            ("Wand of Dark Philosophies", 5000),
            ("Scepter of the Dark Descent", 5000),
            ("Robe of Foreboding Signs", 5000),
            ("Crown of the Insane Alchemist", 5000),
            ("Gladiator Sword", 5000),
            ("Minotaur's Waraxe", 5000),
            ("Champions breastplate", 5000),
            ("Gladiator Trophy", 5000),
            ("Titus's Shield", 5000),
            ("Gladiator Trophy", 5000),
            ("Titus's Shield", 5000),
            ("Horn of Magical", 5000),
            ("Sky Dagger", 5000),
            ("Falling Star", 5000),
            ("Golem's Axe", 12000),
            ("Golem's Robe", 5000),
            ("Sky Katana", 5000),
            ("Veil of the ancient oceans", 5000),
            ("Pink Robe", 5000),
            ("Pink Tome", 5000),
            ("Septavius Ghost Robe", 5000),
            ("Pink Wand", 5000),
            ("Pink Ring", 5000),
            ("Royality Ring of Depth Oceans", 5000),
            ("Thessal's Hide", 5000),
            ("Sharped corals of the oceans", 5000),
            ("Sword of the Spirit Walker", 5000),
            ("5000 Fame", 300),
        ],
    },
    Shop {
        region: Region::Store19,
        currency: Currency::Fame,
        rank: 0,
        stock: &[],
    },
];

/// What one merchant is selling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stall {
    pub item: hendra_content::ObjectType,
    pub price: i32,
    pub currency: Currency,
    pub rank: i16,

    /// The market listing this is, where it is one.
    ///
    /// A shop's stock is endless and a player's listing is one item that somebody else owns until
    /// it is bought, so the two are bought through different paths and this is what tells them
    /// apart.
    pub listing: Option<i64>,
}

/// Deals a shop's stock out over the squares its region marks.
///
/// `squares` is how many merchants stand there. A shop with more stock than squares shows only what
/// fits, and one with fewer wraps, which is what the original's rotation comes to.
///
/// Names the catalog does not have are skipped and reported, because a shop selling something that
/// does not exist is a merchant nobody can buy from and nothing to say why.
pub fn deal(
    shop: &Shop,
    squares: usize,
    catalog: &hendra_content::Catalog,
) -> (Vec<Stall>, Vec<&'static str>) {
    let mut stalls = Vec::new();
    let mut missing = Vec::new();

    if shop.stock.is_empty() || squares == 0 {
        return (stalls, missing);
    }

    for square in 0..squares {
        let (name, price) = shop.stock[square % shop.stock.len()];

        let Some(item) = named(catalog, name) else {
            if !missing.contains(&name) {
                missing.push(name);
            }
            continue;
        };

        stalls.push(Stall {
            item,
            price,
            currency: shop.currency,
            rank: shop.rank,
            listing: None,
        });
    }

    (stalls, missing)
}

/// Finds an item by name, ignoring case if an exact match fails.
///
/// The original's own table gets two names wrong by capitals alone: it asks for
/// "Veil of the ancient oceans" and "Ghostly trap", and the content spells both with capitals. A
/// case-sensitive lookup drops both, which is two things a shop is meant to sell and nothing to say
/// why. The intent is not in doubt, so the capitals are not insisted on.
fn named(catalog: &hendra_content::Catalog, name: &str) -> Option<hendra_content::ObjectType> {
    if let Some(exact) = catalog.type_of(name) {
        return Some(exact);
    }

    catalog
        .objects()
        .find(|desc| desc.id.eq_ignore_ascii_case(name))
        .map(|desc| desc.object_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shop_sells_something_the_content_has() {
        // A shop selling something that does not exist is a merchant nobody can buy from, and
        // without this the only symptom is a nexus stall that does nothing.
        let Ok((catalog, _)) = hendra_content::Catalog::load_dir(std::path::Path::new(
            "../../../godot-client/assets/xml",
        )) else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let mut absent: Vec<String> = Vec::new();
        for shop in SHOPS {
            let (stalls, missing) = deal(shop, shop.stock.len(), &catalog);

            for name in missing {
                absent.push(format!("{:?}: {name}", shop.region));
            }
            if !shop.stock.is_empty() {
                assert!(!stalls.is_empty(), "{:?} sells nothing at all", shop.region);
            }
        }

        assert!(absent.is_empty(), "not in the content: {absent:#?}");
    }

    #[test]
    fn a_name_that_differs_only_in_capitals_still_finds_its_item() {
        // The original's table gets two wrong this way, and a case-sensitive lookup drops both.
        let Ok((catalog, _)) = hendra_content::Catalog::load_dir(std::path::Path::new(
            "../../../godot-client/assets/xml",
        )) else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        assert!(catalog.type_of("Ghostly trap").is_none(), "the exact name");
        assert!(
            named(&catalog, "Ghostly trap").is_some(),
            "but the item is there"
        );
        assert_eq!(
            named(&catalog, "Ghostly trap"),
            catalog.type_of("Ghostly Trap")
        );
    }

    #[test]
    fn stock_wraps_around_the_squares_it_is_dealt_over() {
        // A shop with three things and eight squares shows all three, twice over and then some.
        let Ok((catalog, _)) = hendra_content::Catalog::load_dir(std::path::Path::new(
            "../../../godot-client/assets/xml",
        )) else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let armor = SHOPS
            .iter()
            .find(|shop| shop.region == Region::Store3)
            .expect("the armour shop");

        let (stalls, _) = deal(armor, 8, &catalog);
        assert_eq!(stalls.len(), 8);
        assert_eq!(stalls[0], stalls[3], "the fourth should be the first again");
    }

    #[test]
    fn a_shop_with_nowhere_to_stand_deals_nothing() {
        let Ok((catalog, _)) = hendra_content::Catalog::load_dir(std::path::Path::new(
            "../../../godot-client/assets/xml",
        )) else {
            return;
        };

        let (stalls, _) = deal(&SHOPS[0], 0, &catalog);
        assert!(stalls.is_empty());
    }

    #[test]
    fn the_fame_shop_is_the_one_that_asks_for_rank() {
        // Buying fame with fame is the one thing the original gates, and it gates it at five.
        let fame = SHOPS
            .iter()
            .find(|shop| shop.region == Region::Store6)
            .expect("the fame shop");

        assert_eq!(fame.rank, 5);
        assert!(SHOPS.iter().filter(|shop| shop.rank > 0).count() == 1);
    }
}

/// What prestige buys, from `PrestigeBuyHandler`.
///
/// A short list rather than a shop with merchants: the original sells these from a menu rather than
/// from anything standing in the world.
pub const PRESTIGE_OFFERS: &[(&str, i32)] = &[
    ("Claymore of Eternal Light", 50),
    ("Vault of the Skies", 150),
    ("Helm of the Heavenly Guard", 150),
    ("Ring of Deep Radiance", 150),
];

#[cfg(test)]
mod prestige_tests {
    use super::*;

    #[test]
    fn everything_prestige_buys_is_in_the_content() {
        let Ok((catalog, _)) = hendra_content::Catalog::load_dir(std::path::Path::new(
            "../../../godot-client/assets/xml",
        )) else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        for (name, price) in PRESTIGE_OFFERS {
            assert!(
                named(&catalog, name).is_some(),
                "{name} is not in the content"
            );
            assert!(*price > 0, "{name} costs nothing");
        }
    }
}

#[cfg(test)]
mod rank {
    use super::*;

    #[test]
    fn a_shop_that_asks_for_nothing_admits_everybody() {
        assert!(admits(0, 0, 0));
    }

    #[test]
    fn a_shop_that_asks_for_stars_wants_stars() {
        // Five is a character taken to two thousand fame, which is the point of the gate: no amount
        // of money or time buys it.
        assert!(!admits(5, 4, 0));
        assert!(admits(5, 5, 0));
        assert!(admits(5, 9, 0));
    }

    #[test]
    fn an_administrator_is_admitted_regardless() {
        assert!(admits(5, 0, 5));
    }

    #[test]
    fn exactly_one_shop_in_the_game_asks_for_a_rank() {
        // The fame shop. If a second one appears, the gate is worth reading again rather than
        // assuming it means what it meant for this one.
        let gated: Vec<i16> = SHOPS
            .iter()
            .filter(|shop| shop.rank > 0)
            .map(|shop| shop.rank)
            .collect();

        assert_eq!(gated, vec![5]);
    }
}

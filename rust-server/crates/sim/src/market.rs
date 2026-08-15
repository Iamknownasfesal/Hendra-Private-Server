//! The board of merchants a marketplace stands, and what each of them is holding.
//!
//! Follows `wServer/realm/Market.cs`. The marketplace does not stand one merchant per listing: it
//! stands one merchant per *item*, showing the cheapest listing of it and how many there are, and
//! every merchant moves on to a different item on a clock. A row of twenty squares with three
//! distinct swords for sale is three merchants, not twenty, and each of the three shows a different
//! sword a few seconds later.
//!
//! # Why a queue rather than a list
//!
//! `Market` keeps one queue of item types per row. A merchant takes the front of it when it is
//! stood and, whenever it rotates, puts what it was holding back on the end and takes the front
//! again (`Market.cs:210-223`). With more items than squares that deals every item a turn in front
//! of somebody; with fewer, the queue empties and the leftover squares stay bare.
//!
//! # What is deliberately not reproduced
//!
//! `Market.Reload` reads `market[nextItem][0]` without checking that the list still has anything in
//! it (`Market.cs:228-232`). An item whose last listing was bought while no merchant was holding it
//! stays in the queue, and the merchant that eventually dequeues it throws out of a task
//! continuation and stops updating. Here such an item is dropped from the queue instead.
//!
//! A merchant's `TimeLeft` starts at -1 and nothing sets it until its first turn comes round
//! (`Merchant.cs:42`), so a sale in the first twenty seconds of a marketplace's life rotates the
//! merchant that made it even though what it was holding is still for sale. Every later sale
//! refreshes it instead, because by then `TimeLeft` is the thirty thousand `Reload` writes. That
//! first-cycle case is not reproduced: it is an artefact of the default rather than a rule, and the
//! rule is the one every other sale follows.

use hendra_content::{Catalog, ObjectType, Region};
use std::collections::{HashMap, HashSet, VecDeque};

/// Which row of the marketplace an item stands in.
///
/// `Market.GetItemType` (`realm/Market.cs:403-427`), which sorts by what a thing is rather than by
/// what it is worth, so a marketplace reads as rows of a kind rather than one heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Weapon,
    Ability,
    Armor,
    Ring,

    /// A potion that raises a stat for good, which is the one thing the original singles out ahead
    /// of the slot it goes in.
    StatPot,

    Other,
}

/// How many rows there are, which is how many queues and stock tables the board holds.
pub const KINDS: usize = 6;

impl Kind {
    fn index(self) -> usize {
        match self {
            Kind::Weapon => 0,
            Kind::Ability => 1,
            Kind::Armor => 2,
            Kind::Ring => 3,
            Kind::StatPot => 4,
            Kind::Other => 5,
        }
    }

    /// The row a marked square belongs to, or `None` for a square that marks something else.
    ///
    /// `Market.GetItemType(TileRegion)` (`realm/Market.cs:429-448`).
    pub fn of_region(region: Region) -> Option<Kind> {
        match region {
            Region::Store9 => Some(Kind::Weapon),
            Region::Store10 => Some(Kind::Ability),
            Region::Store11 => Some(Kind::Armor),
            Region::Store12 => Some(Kind::Ring),
            Region::Store13 => Some(Kind::StatPot),
            Region::Store14 => Some(Kind::Other),
            _ => None,
        }
    }

    /// The row an item belongs to.
    ///
    /// `Market.GetItemType(PlayerShopItem)` (`realm/Market.cs:403-427`), in its order: a potion
    /// that raises a stat is a stat potion whatever slot it goes in, and anything whose slot no
    /// class declares — a consumable, a key, a healing potion — falls to `Other`.
    pub fn of_item(catalog: &Catalog, item: ObjectType) -> Kind {
        let Some(desc) = catalog.object(item) else {
            return Kind::Other;
        };
        let Some(item) = desc.item.as_ref() else {
            return Kind::Other;
        };

        if item.potion
            && item
                .activate
                .iter()
                .any(|effect| effect.name == "IncrementStat")
        {
            return Kind::StatPot;
        }

        // `SlotType2ItemType` is built from the eight slots every class declares, in the order
        // weapon, ability, armour, ring and then four the market has no row for
        // (`common/resources/XmlData.cs:271-278`). A slot no class declares is not in the table at
        // all, which is what sends a healing potion to `Other`.
        //
        // Every class writes over what the last one wrote, so a slot two classes disagree about
        // means whatever the last of them says. Walked in the same order and kept the same way.
        let mut mapped = None;
        for class in catalog.classes() {
            for (slot, declared) in class.slot_types.iter().enumerate() {
                if *declared != item.slot_type {
                    continue;
                }
                mapped = Some(match slot {
                    0 => Kind::Weapon,
                    1 => Kind::Ability,
                    2 => Kind::Armor,
                    3 => Kind::Ring,

                    // Amulet, belt, wings and the one marked as coming soon: real entries in the
                    // table, and all four fall through `GetItemType`'s switch to `Other`.
                    _ => Kind::Other,
                });
            }
        }

        mapped.unwrap_or(Kind::Other)
    }
}

/// One thing somebody has listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listing {
    pub id: i64,
    pub item: ObjectType,
    pub price: i32,
}

/// A square the map marks for a row, and when the merchant standing on it moves on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pitch {
    pub x: u32,
    pub y: u32,
    pub kind: Kind,

    /// How far into the twenty-second cycle this one rotates, in milliseconds.
    ///
    /// `InitShopLocations` gives the first square of a row 500, the second 1000 and so on
    /// (`Market.cs:262-266`), so a row moves on in a ripple rather than all at once. A row with
    /// more than thirty-nine squares gives the fortieth an offset of twenty thousand or more, and
    /// `Merchant.Tick` compares against `TotalElapsedMs % 20000` — so those never come round at
    /// all. An oddity of the original, kept: the marketplace map has fifty-eight squares in some
    /// rows, and the merchants past the thirty-ninth stand still.
    pub offset: i64,
}

/// What one merchant is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held {
    /// The item type it is showing, which is a whole item rather than one listing: it displays the
    /// cheapest of them and how many there are.
    pub item: ObjectType,

    /// The listing somebody walking up would buy, which is the cheapest of that item.
    pub listing: i64,

    pub price: i32,

    /// How many of this item are for sale, `Market.Reload`'s `merchant.Count = shop.Count`.
    pub count: i32,
}

/// The board: what is for sale, whose turn it is, and who is standing where.
pub struct Market {
    pitches: Vec<Pitch>,

    /// Per row, the listings of each item, cheapest first and oldest first within a price.
    stock: Vec<HashMap<ObjectType, Vec<Listing>>>,

    /// Per row, whose turn it is next.
    queue: Vec<VecDeque<ObjectType>>,

    /// Per row, the items that are either waiting in the queue or held by a merchant.
    ///
    /// `Market._items`, which is what stops one item being dealt to two merchants at once.
    taken: Vec<HashSet<ObjectType>>,

    /// What stands on each pitch, parallel to `pitches`.
    standing: Vec<Option<Held>>,

    /// Per pitch, whether a rotation is owed because somebody was standing next to it.
    awaiting: Vec<bool>,
}

/// How long one turn of the rotation takes.
///
/// `Merchant.Tick` measures against `time.TotalElapsedMs % 20000` (`Merchant.cs:99`). The thirty
/// seconds `Market` writes into `TimeLeft` never decides anything, because `Tick` sets `TimeLeft`
/// to -1 immediately before every reload it drives (`Merchant.cs:119`).
pub const ROTATION_MS: i64 = 20_000;

/// How far apart consecutive squares of a row rotate.
pub const OFFSET_STEP: i64 = 500;

/// How close somebody has to stand to hold a merchant still.
///
/// `this.AnyPlayerNearby(2)` (`Merchant.cs:106`): a merchant does not change what it is selling
/// under somebody who has walked up to buy it.
pub const HOLD_RADIUS: f32 = 2.0;

impl Market {
    /// Reads the squares a marketplace marks and gives each one its place in the rotation.
    ///
    /// `InitShopLocations` (`Market.cs:250-268`), walking the map in the order `Wmap` records
    /// regions — row by row, left to right (`realm/terrain/Wmap.cs:347-356`).
    pub fn of_map(map: &hendra_content::Map) -> Market {
        let mut squares: Vec<(u32, u32, Kind)> = map
            .regions()
            .filter_map(|(x, y, region)| Kind::of_region(region).map(|kind| (x, y, kind)))
            .collect();
        squares.sort_by_key(|(x, y, _)| (*y, *x));

        let mut step = [0i64; KINDS];
        let pitches = squares
            .into_iter()
            .map(|(x, y, kind)| {
                step[kind.index()] += OFFSET_STEP;
                Pitch {
                    x,
                    y,
                    kind,
                    offset: step[kind.index()],
                }
            })
            .collect();

        Market::new(pitches)
    }

    pub fn new(pitches: Vec<Pitch>) -> Market {
        let count = pitches.len();
        Market {
            pitches,
            stock: (0..KINDS).map(|_| HashMap::new()).collect(),
            queue: (0..KINDS).map(|_| VecDeque::new()).collect(),
            taken: (0..KINDS).map(|_| HashSet::new()).collect(),
            standing: vec![None; count],
            awaiting: vec![false; count],
        }
    }

    pub fn pitches(&self) -> &[Pitch] {
        &self.pitches
    }

    /// What stands on a pitch, or `None` for a bare square.
    pub fn standing(&self, pitch: usize) -> Option<Held> {
        self.standing.get(pitch).copied().flatten()
    }

    /// Replaces everything for sale, then brings the board back into agreement with it.
    ///
    /// The original is told about one listing at a time — `Market.Add` and `Market.Remove` each
    /// reload the one merchant they affect (`Market.cs:111-133`, `:165-166`) — where this is handed
    /// the whole board afresh. The outcome is the same: an item nobody is holding and nobody has
    /// queued joins the queue, a merchant whose item is still for sale is refreshed to the cheapest
    /// of it, and a merchant whose item has sold out moves on.
    ///
    /// `listings` must be in the order they were listed, oldest first: the sort below is stable, so
    /// that order is what breaks a tie on price, exactly as `PlaceShopItem` does
    /// (`Market.cs:357-382`).
    pub fn restock(&mut self, catalog: &Catalog, listings: &[Listing]) {
        for row in self.stock.iter_mut() {
            row.clear();
        }

        for listed in listings {
            let kind = Kind::of_item(catalog, listed.item).index();
            self.stock[kind]
                .entry(listed.item)
                .or_default()
                .push(*listed);
        }

        for row in self.stock.iter_mut() {
            for offers in row.values_mut() {
                offers.sort_by_key(|listed| listed.price);
            }
        }

        // A merchant holding something that has sold out moves on; one holding something still for
        // sale is refreshed to the cheapest of it, which is what `Market.Remove` does after a
        // purchase takes the cheapest away.
        for pitch in 0..self.pitches.len() {
            let Some(held) = self.standing[pitch] else {
                continue;
            };
            let kind = self.pitches[pitch].kind.index();

            match self.stock[kind].get(&held.item) {
                Some(offers) if !offers.is_empty() => {
                    self.standing[pitch] = Some(Held {
                        item: held.item,
                        listing: offers[0].id,
                        price: offers[0].price,
                        count: offers.len() as i32,
                    });
                }
                _ => self.reload(pitch),
            }
        }

        // Anything for sale that nobody is holding and nothing has queued gets a turn.
        for kind in 0..KINDS {
            let mut fresh: Vec<ObjectType> = self.stock[kind]
                .iter()
                .filter(|(item, offers)| !offers.is_empty() && !self.taken[kind].contains(item))
                .map(|(item, _)| *item)
                .collect();

            // The map is unordered, so a deterministic order here is what stops two servers reading
            // the same board and standing different merchants.
            fresh.sort_by_key(|item| item.0);

            for item in fresh {
                self.taken[kind].insert(item);
                self.queue[kind].push_back(item);
            }
        }

        self.fill_pitches();
    }

    /// Moves on every merchant whose turn has come round.
    ///
    /// `Merchant.Tick` (`Merchant.cs:95-124`): each one rotates once per cycle at its own offset,
    /// and one with somebody standing within two squares waits until they leave rather than
    /// changing what it sells under them.
    pub fn tick(&mut self, elapsed_ms: i64, delta_ms: i64, mut busy: impl FnMut(usize) -> bool) {
        let within = elapsed_ms.rem_euclid(ROTATION_MS);

        for pitch in 0..self.pitches.len() {
            if self.standing[pitch].is_none() {
                continue;
            }

            let offset = self.pitches[pitch].offset;
            let due = self.awaiting[pitch] || (within - delta_ms <= offset && within > offset);
            if !due {
                continue;
            }

            if busy(pitch) {
                self.awaiting[pitch] = true;
                continue;
            }

            self.reload(pitch);
            self.awaiting[pitch] = false;
        }
    }

    /// Puts what a merchant was holding back on the end of the queue and gives it the front.
    ///
    /// `Market.Reload` with `TimeLeft` at -1, which is what every tick-driven reload has
    /// (`Merchant.cs:119`, `Market.cs:210-237`). An item still for sale goes back in the queue; one
    /// that has sold out leaves the board entirely.
    fn reload(&mut self, pitch: usize) {
        let Some(held) = self.standing[pitch] else {
            return;
        };
        let kind = self.pitches[pitch].kind.index();

        let left = self.stock[kind]
            .get(&held.item)
            .map(|offers| offers.len())
            .unwrap_or(0);

        if left > 0 {
            self.queue[kind].push_back(held.item);
        } else {
            self.taken[kind].remove(&held.item);
        }

        self.standing[pitch] = self.deal(kind);
    }

    /// Stands a merchant on every bare square the queue can still fill.
    ///
    /// `AddMerchants` (`Market.cs:270-305`), which walks every marked square and gives each one
    /// whatever the queue hands over. A square the queue cannot fill stays bare, which is why a
    /// marketplace with three swords for sale has three merchants in a row of fifty-eight squares.
    fn fill_pitches(&mut self) {
        for pitch in 0..self.pitches.len() {
            if self.standing[pitch].is_some() {
                continue;
            }
            let kind = self.pitches[pitch].kind.index();
            self.standing[pitch] = self.deal(kind);
        }
    }

    /// Takes the next item off a row's queue, skipping any that has sold out since it was queued.
    fn deal(&mut self, kind: usize) -> Option<Held> {
        while let Some(item) = self.queue[kind].pop_front() {
            let Some(offers) = self.stock[kind].get(&item) else {
                self.taken[kind].remove(&item);
                continue;
            };
            let Some(cheapest) = offers.first() else {
                self.taken[kind].remove(&item);
                continue;
            };

            return Some(Held {
                item,
                listing: cheapest.id,
                price: cheapest.price,
                count: offers.len() as i32,
            });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pitches(kind: Kind, count: usize) -> Vec<Pitch> {
        (0..count)
            .map(|index| Pitch {
                x: index as u32,
                y: 0,
                kind,
                offset: OFFSET_STEP * (index as i64 + 1),
            })
            .collect()
    }

    fn listing(id: i64, item: u16, price: i32) -> Listing {
        Listing {
            id,
            item: ObjectType(item),
            price,
        }
    }

    fn catalog() -> Option<Catalog> {
        hendra_content::Catalog::load_dir(std::path::Path::new("../../../godot-client/assets/xml"))
            .ok()
            .map(|(catalog, _)| catalog)
    }

    /// A board of one row whose items are classified without asking the content, so the queue and
    /// rotation can be tested on their own.
    fn board(count: usize) -> Market {
        Market::new(pitches(Kind::Other, count))
    }

    /// Puts listings straight into a row, bypassing the classifier.
    fn stock(market: &mut Market, listings: &[Listing]) {
        for row in market.stock.iter_mut() {
            row.clear();
        }
        for listed in listings {
            market.stock[Kind::Other.index()]
                .entry(listed.item)
                .or_default()
                .push(*listed);
        }
        for offers in market.stock[Kind::Other.index()].values_mut() {
            offers.sort_by_key(|listed| listed.price);
        }

        let mut fresh: Vec<ObjectType> = market.stock[Kind::Other.index()]
            .keys()
            .copied()
            .filter(|item| !market.taken[Kind::Other.index()].contains(item))
            .collect();
        fresh.sort_by_key(|item| item.0);
        for item in fresh {
            market.taken[Kind::Other.index()].insert(item);
            market.queue[Kind::Other.index()].push_back(item);
        }

        for pitch in 0..market.pitches.len() {
            let Some(held) = market.standing[pitch] else {
                continue;
            };
            match market.stock[Kind::Other.index()].get(&held.item) {
                Some(offers) if !offers.is_empty() => {
                    market.standing[pitch] = Some(Held {
                        item: held.item,
                        listing: offers[0].id,
                        price: offers[0].price,
                        count: offers.len() as i32,
                    })
                }
                _ => market.reload(pitch),
            }
        }
        market.fill_pitches();
    }

    #[test]
    fn a_merchant_shows_the_cheapest_of_what_it_holds_and_how_many_there_are() {
        // `Market.Reload` takes `shop[0]` for the price and `shop.Count` for the number
        // (`Market.cs:231-236`), and `PlaceShopItem` keeps the list cheapest first
        // (`Market.cs:357-382`).
        let mut market = board(1);
        stock(
            &mut market,
            &[
                listing(1, 0x0a22, 500),
                listing(2, 0x0a22, 120),
                listing(3, 0x0a22, 900),
            ],
        );

        let held = market.standing(0).expect("a merchant for the one item");
        assert_eq!(held.item, ObjectType(0x0a22));
        assert_eq!(held.price, 120, "the cheapest is what is on offer");
        assert_eq!(held.listing, 2);
        assert_eq!(held.count, 3, "and all three are counted");
    }

    #[test]
    fn two_listings_at_one_price_are_offered_oldest_first() {
        // `PlaceShopItem` inserts before an equal price only when the standing one was listed
        // later, so the older of two identical offers is the one that sells (`Market.cs:372-376`).
        let mut market = board(1);
        stock(
            &mut market,
            &[listing(7, 0x0a22, 300), listing(9, 0x0a22, 300)],
        );

        assert_eq!(market.standing(0).unwrap().listing, 7);
    }

    #[test]
    fn one_merchant_per_item_rather_than_one_per_listing() {
        // Twenty squares and two distinct items is two merchants. Standing one per listing is the
        // difference between a marketplace and a wall of stalls.
        let mut market = board(20);
        stock(
            &mut market,
            &[
                listing(1, 0x0a22, 100),
                listing(2, 0x0a22, 200),
                listing(3, 0x0a22, 300),
                listing(4, 0x0904, 50),
            ],
        );

        let standing = (0..20)
            .filter(|pitch| market.standing(*pitch).is_some())
            .count();
        assert_eq!(standing, 2);
    }

    #[test]
    fn a_row_deals_every_item_a_turn_in_front_of_somebody() {
        // Three items and one square: the merchant works through all three, one per cycle, and
        // comes back round to the first.
        let mut market = board(1);
        stock(
            &mut market,
            &[
                listing(1, 0x0001, 10),
                listing(2, 0x0002, 20),
                listing(3, 0x0003, 30),
            ],
        );

        let mut seen = Vec::new();
        for turn in 0..4 {
            seen.push(market.standing(0).unwrap().item);
            // One tick past this pitch's offset, a cycle at a time.
            market.tick(ROTATION_MS * turn + OFFSET_STEP + 100, 200, |_| false);
        }

        assert_eq!(
            seen,
            vec![
                ObjectType(0x0001),
                ObjectType(0x0002),
                ObjectType(0x0003),
                ObjectType(0x0001),
            ]
        );
    }

    #[test]
    fn a_merchant_does_not_change_under_somebody_standing_at_it() {
        // `AnyPlayerNearby(2)` defers the rotation rather than skipping it, so it happens the
        // moment they walk away (`Merchant.cs:106-110`).
        let mut market = board(1);
        stock(
            &mut market,
            &[listing(1, 0x0001, 10), listing(2, 0x0002, 20)],
        );

        let before = market.standing(0).unwrap().item;
        market.tick(OFFSET_STEP + 100, 200, |_| true);
        assert_eq!(market.standing(0).unwrap().item, before, "held still");

        // And the moment nobody is there it catches up, without waiting for the next cycle.
        market.tick(OFFSET_STEP + 300, 200, |_| false);
        assert_ne!(market.standing(0).unwrap().item, before);
    }

    #[test]
    fn a_merchant_whose_item_sells_out_moves_on() {
        // `Market.Reload` with nothing left takes the item out of the board rather than putting it
        // back in the queue (`Market.cs:213-215`).
        let mut market = board(1);
        stock(
            &mut market,
            &[listing(1, 0x0001, 10), listing(2, 0x0002, 20)],
        );
        let first = market.standing(0).unwrap().item;

        // Everything of the held item is bought.
        let left: Vec<Listing> = [listing(1, 0x0001, 10), listing(2, 0x0002, 20)]
            .into_iter()
            .filter(|listed| listed.item != first)
            .collect();
        stock(&mut market, &left);

        assert_ne!(market.standing(0).unwrap().item, first);

        // And it does not come back round, because it is no longer for sale.
        for turn in 0..3 {
            market.tick(ROTATION_MS * turn + OFFSET_STEP + 100, 200, |_| false);
            assert_ne!(market.standing(0).unwrap().item, first);
        }
    }

    #[test]
    fn the_last_thing_for_sale_leaves_a_bare_square_behind() {
        let mut market = board(3);
        stock(&mut market, &[listing(1, 0x0001, 10)]);
        assert!(market.standing(0).is_some());

        stock(&mut market, &[]);
        assert!(
            (0..3).all(|pitch| market.standing(pitch).is_none()),
            "a merchant with nothing to sell is a stall that refuses everybody"
        );
    }

    #[test]
    fn a_row_rotates_in_a_ripple_rather_than_all_at_once() {
        // Consecutive squares are five hundred milliseconds apart (`Market.cs:262-266`), so a tick
        // that crosses the first one's turn does not cross the second's.
        let mut market = board(2);
        stock(
            &mut market,
            &[
                listing(1, 0x0001, 10),
                listing(2, 0x0002, 20),
                listing(3, 0x0003, 30),
            ],
        );

        let before = [market.standing(0).unwrap(), market.standing(1).unwrap()];
        market.tick(OFFSET_STEP + 100, 200, |_| false);

        assert_ne!(market.standing(0).unwrap().item, before[0].item);
        assert_eq!(market.standing(1).unwrap().item, before[1].item);
    }

    #[test]
    fn a_square_past_the_thirty_ninth_of_a_row_never_comes_round() {
        // `Merchant.Tick` compares against `TotalElapsedMs % 20000`, and `InitShopLocations` keeps
        // adding five hundred, so the fortieth square of a row is given an offset the cycle can
        // never reach. An oddity of the original, and the marketplace map has rows of fifty-eight.
        let mut market = Market::new(pitches(Kind::Other, 40));
        stock(
            &mut market,
            &(1..=60)
                .map(|n| listing(n as i64, n as u16, 10))
                .collect::<Vec<_>>(),
        );

        let stuck = market.standing(39).unwrap().item;
        for turn in 0..3 {
            for step in 0..(ROTATION_MS / 200) {
                market.tick(ROTATION_MS * turn + step * 200, 200, |_| false);
            }
        }

        assert_eq!(market.pitches()[39].offset, OFFSET_STEP * 40);
        assert_eq!(
            market.standing(39).unwrap().item,
            stuck,
            "an offset of twenty thousand is never crossed"
        );
        assert_ne!(
            market.standing(0).unwrap().item,
            ObjectType(1),
            "while the first square has moved on several times"
        );
    }

    #[test]
    fn the_marketplace_map_marks_squares_for_every_row() {
        // Without these the world stands nothing at all, and the market is reachable only by
        // typing. Read from our own content rather than from the specification, because our copy
        // is the one a running server builds the marketplace from.
        let (Some(catalog), Ok(json)) = (
            catalog(),
            std::fs::read_to_string("../../content/worlds/marketplace.jm"),
        ) else {
            eprintln!("skipping: the marketplace map is not where the test looks for it");
            return;
        };
        let (map, _) =
            hendra_content::legacy::from_jm(&json, &catalog).expect("our marketplace map");
        let market = Market::of_map(&map);

        for kind in [
            Kind::Weapon,
            Kind::Ability,
            Kind::Armor,
            Kind::Ring,
            Kind::StatPot,
            Kind::Other,
        ] {
            assert!(
                market.pitches().iter().any(|pitch| pitch.kind == kind),
                "the map marks no square for {kind:?}"
            );
        }

        // The first square of each row rotates half a second in, as `InitShopLocations` numbers
        // them.
        for kind in [Kind::Weapon, Kind::Other] {
            let first = market
                .pitches()
                .iter()
                .find(|pitch| pitch.kind == kind)
                .unwrap();
            assert_eq!(first.offset, OFFSET_STEP);
        }
    }

    #[test]
    fn a_healing_potion_stands_in_the_other_row_and_a_life_potion_in_its_own() {
        // `GetItemType` singles out a potion that raises a stat and lets every other potion fall
        // through to `Other`, because no class declares the slot a consumable goes in
        // (`Market.cs:408-412`, `common/resources/XmlData.cs:271-278`).
        let Some(catalog) = catalog() else {
            eprintln!("skipping: the content files are not where the test looks for them");
            return;
        };

        let life = catalog.type_of("Potion of Life").expect("a life potion");
        assert_eq!(Kind::of_item(&catalog, life), Kind::StatPot);

        let healing = catalog.type_of("Health Potion").expect("a healing potion");
        assert_eq!(Kind::of_item(&catalog, healing), Kind::Other);

        let sword = catalog.type_of("Sword of Acclaim").expect("a sword");
        assert_eq!(Kind::of_item(&catalog, sword), Kind::Weapon);

        let robe = catalog
            .type_of("Robe of the Grand Sorcerer")
            .expect("a robe");
        assert_eq!(Kind::of_item(&catalog, robe), Kind::Armor);

        let ring = catalog.type_of("Ring of Paramount Attack").expect("a ring");
        assert_eq!(Kind::of_item(&catalog, ring), Kind::Ring);

        let cloak = catalog
            .type_of("Cloak of Ghostly Concealment")
            .expect("a cloak");
        assert_eq!(Kind::of_item(&catalog, cloak), Kind::Ability);
    }

    #[test]
    fn a_restock_that_changes_nothing_leaves_every_merchant_where_it_was() {
        // The board is read afresh every few seconds; a read that says the same thing must not
        // shuffle the marketplace under everybody standing in it.
        let mut market = board(4);
        let listings = [
            listing(1, 0x0001, 10),
            listing(2, 0x0002, 20),
            listing(3, 0x0003, 30),
        ];
        stock(&mut market, &listings);

        let before: Vec<Option<Held>> = (0..4).map(|pitch| market.standing(pitch)).collect();
        stock(&mut market, &listings);
        let after: Vec<Option<Held>> = (0..4).map(|pitch| market.standing(pitch)).collect();

        assert_eq!(before, after);
    }
}

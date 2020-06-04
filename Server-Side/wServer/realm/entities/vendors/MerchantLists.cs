using System;
using System.Collections.Generic;
using System.Linq;
using common;
using common.resources;
using log4net;
using wServer.realm.terrain;

namespace wServer.realm.entities.vendors
{
    public class ShopItem : ISellableItem
    {
        public ushort ItemId { get; private set; }
        public int Price { get; }
        public int Count { get; }
        public string Name { get; }

        public ShopItem(string name, ushort price, int count = -1)
        {
            ItemId = ushort.MaxValue;
            Price = price;
            Count = count;
            Name = name;
        }

        public void SetItem(ushort item)
        {
            if (ItemId != ushort.MaxValue)
                throw new AccessViolationException("Can't change item after it has been set.");

            ItemId = item;
        }
    }
    
    internal static class MerchantLists
    {
        private static readonly ILog Log = LogManager.GetLogger(typeof(MerchantLists));

        private static readonly List<ISellableItem> Weapons = new List<ISellableItem>
        {
            new ShopItem("Dagger of Foul Malevolence", 500),
            new ShopItem("Bow of Covert Havens", 500),
            new ShopItem("Staff of the Cosmic Whole", 500),
            new ShopItem("Wand of Recompense", 500), 
            new ShopItem("Sword of Acclaim", 500),
            new ShopItem("Masamune", 500) 
        };

        private static readonly List<ISellableItem> Abilities = new List<ISellableItem>
        {
            new ShopItem("Cloak of Ghostly Concealment", 500),
            new ShopItem("Quiver of Elvish Mastery", 500),  
            new ShopItem("Elemental Detonation Spell", 500),
            new ShopItem("Tome of Holy Guidance", 500),
            new ShopItem("Helm of the Great General", 500),
            new ShopItem("Colossus Shield", 500), 
            new ShopItem("Seal of the Blessed Champion", 500),
            new ShopItem("Baneserpent Poison", 500),
            new ShopItem("Bloodsucker Skull", 500),
            new ShopItem("Giantcatcher Trap", 500),
            new ShopItem("Planefetter Orb", 500),
            new ShopItem("Prism of Apparitions", 500),
            new ShopItem("Scepter of Storms", 500),
            new ShopItem("Doom Circle", 500)
        };

        private static readonly List<ISellableItem> Armor = new List<ISellableItem>
        {
            new ShopItem("Robe of the Grand Sorcerer", 600),
            new ShopItem("Hydra Skin Armor", 600),
            new ShopItem("Acropolis Armor", 600)
        };

        private static readonly List<ISellableItem> Rings = new List<ISellableItem>
        {
            new ShopItem("Ring of Paramount Attack", 1000),
            new ShopItem("Ring of Paramount Defense", 1000),
            new ShopItem("Ring of Paramount Speed", 1000),
            new ShopItem("Ring of Paramount Dexterity", 1000),
            new ShopItem("Ring of Paramount Vitality", 1000),
            new ShopItem("Ring of Paramount Wisdom", 1000),
            new ShopItem("Ring of Paramount Health", 1000),
            new ShopItem("Ring of Paramount Magic", 1000),
            new ShopItem("Ring of Unbound Attack", 750),
            new ShopItem("Ring of Unbound Defense", 900),
            new ShopItem("Ring of Unbound Speed", 750),
            new ShopItem("Ring of Unbound Dexterity", 900),
            new ShopItem("Ring of Unbound Vitality", 750),
            new ShopItem("Ring of Unbound Wisdom", 750),
            new ShopItem("Ring of Unbound Health", 1000),
            new ShopItem("Ring of Unbound Magic", 1000)
        };

        private static readonly List<ISellableItem> Keys = new List<ISellableItem>
        {
            new ShopItem("Undead Lair Key", 200),
            new ShopItem("Sprite World Key", 200),
            new ShopItem("Abyss of Demons Key", 200),
            new ShopItem("Ocean Trench Key", 200),
            new ShopItem("Snake Pit Key", 200),
            new ShopItem("Lost Halls Key", 800),
            new ShopItem("Tomb of the Ancients Key", 500),
        };

        private static readonly List<ISellableItem> PurchasableFame = new List<ISellableItem>
        {
            new ShopItem("50 Fame", 50),
            new ShopItem("100 Fame", 100),
            new ShopItem("500 Fame", 500),
            new ShopItem("1000 Fame", 1000),
            new ShopItem("5000 Fame", 5000)
        };

        private static readonly List<ISellableItem> Consumables = new List<ISellableItem>
        {
            new ShopItem("XP Booster", 0),
            new ShopItem("Loot Drop Potion", 1500),
            new ShopItem("Backpack", 1000),
        };

        private static readonly List<ISellableItem> Special = new List<ISellableItem>
        {
            new ShopItem("Amulet of Resurrection", 50000) 
        };

        private static readonly List<ISellableItem> DonorShop = new List<ISellableItem>
        {
            new ShopItem("Claymore of Eternal Light", 500),
            new ShopItem("Helm of the Heavenly Guard", 500),
            new ShopItem("Vault of the Skies", 500),
            new ShopItem("Ring of Deep Radiance", 500),
            new ShopItem("Maxy", 250),
            new ShopItem("Health Maxy", 50),
            new ShopItem("Wisdom Maxy", 50),
            new ShopItem("Vitality Maxy", 50),
            new ShopItem("Dexterity Maxy", 50),
            new ShopItem("Defense Maxy", 50),
            new ShopItem("Mana Maxy", 50),
            new ShopItem("Speed Maxy", 50),
            new ShopItem("Attack Maxy", 50),
            new ShopItem("Staff of the Phoenix Lord", 500),
            new ShopItem("Elven Tablet of the Blood Moon", 500),
            new ShopItem("Robe of the Elven Highlord", 500),
            new ShopItem("Ring of the Golden Sun", 500),
            new ShopItem("Wand of Dark Philosophies", 500),
            new ShopItem("Scepter of the Dark Descent", 500),
            new ShopItem("Robe of Foreboding Signs", 500),
            new ShopItem("Crown of the Insane Alchemist", 500),
            new ShopItem("Gladiator Sword", 500),
            new ShopItem("Minotaur's Waraxe", 500),
            new ShopItem("Champions breastplate", 500),
            new ShopItem("Gladiator Trophy", 500),
            new ShopItem("Titus's Shield", 500),
            new ShopItem("Gladiator Trophy", 500),
            new ShopItem("Titus's Shield", 500),
            new ShopItem("Horn of Magical", 500),
            new ShopItem("Sky Dagger", 500),
            new ShopItem("Falling Star", 500),
            new ShopItem("Golem's Axe", 900),
            new ShopItem("Golem's Robe", 600),
            new ShopItem("Sky Katana", 800),
            new ShopItem("Veil of the ancient oceans", 500),
            new ShopItem("Pink Robe", 500),
            new ShopItem("Pink Tome", 500),
            new ShopItem("Septavius Ghost Robe", 500),
            new ShopItem("Pink Wand", 500),
            new ShopItem("Pink Ring", 500),
            new ShopItem("Royality Ring of Depth Oceans", 500),
            new ShopItem("Thessal's Hide", 500),
            new ShopItem("Sharped corals of the oceans", 500),
            new ShopItem("Ghost Cannon", 500),
            new ShopItem("Ghostly Armor", 500),
            new ShopItem("Ghostly trap", 500),
            new ShopItem("Revenge Ring", 500),
            new ShopItem("Barriel's Enchanted Spear", 800),
            new ShopItem("Leaf Amulet", 500),
            new ShopItem("Wooden Helm", 500),
            new ShopItem("Wooden Hide Armor", 500),
            new ShopItem("Staff of blood", 700),
            new ShopItem("Staff of Green Posion", 600),
            new ShopItem("Robber's Gun", 1000),
            new ShopItem("Puppet Rainbow Dagger", 500),
            new ShopItem("Stheno Quiver", 600),
            new ShopItem("Sword of the Spirit Walker", 500)
                    };
        private static readonly List<ISellableItem> NexusShop = new List<ISellableItem>
        {
            new ShopItem("Claymore of Eternal Light", 10000),
            new ShopItem("Helm of the Heavenly Guard", 10000),
            new ShopItem("Vault of the Skies", 10000),
            new ShopItem("Ring of Deep Radiance", 10000),
            new ShopItem("Maxy", 10000),
            new ShopItem("Health Maxy", 5000),
            new ShopItem("Wisdom Maxy", 5000),
            new ShopItem("Vitality Maxy", 5000),
            new ShopItem("Dexterity Maxy", 5000),
            new ShopItem("Defense Maxy", 5000),
            new ShopItem("Mana Maxy", 5000),
            new ShopItem("Speed Maxy", 5000),
            new ShopItem("Attack Maxy", 5000),
            new ShopItem("Staff of the Phoenix Lord", 5000),
            new ShopItem("Elven Tablet of the Blood Moon", 5000),
            new ShopItem("Robe of the Elven Highlord", 5000),
            new ShopItem("Ring of the Golden Sun", 5000),
            new ShopItem("Wand of Dark Philosophies", 5000),
            new ShopItem("Scepter of the Dark Descent", 5000),
            new ShopItem("Robe of Foreboding Signs", 5000),
            new ShopItem("Crown of the Insane Alchemist", 5000),
            new ShopItem("Gladiator Sword", 5000),
            new ShopItem("Minotaur's Waraxe", 5000),
            new ShopItem("Champions breastplate", 5000),
            new ShopItem("Gladiator Trophy", 5000),
            new ShopItem("Titus's Shield", 5000),
            new ShopItem("Gladiator Trophy", 5000),
            new ShopItem("Titus's Shield", 5000),
            new ShopItem("Horn of Magical", 5000),
            new ShopItem("Sky Dagger", 5000),
            new ShopItem("Falling Star", 5000),
            new ShopItem("Golem's Axe", 12000),
            new ShopItem("Golem's Robe", 5000),
            new ShopItem("Sky Katana", 5000),
            new ShopItem("Veil of the ancient oceans", 5000),
            new ShopItem("Pink Robe", 5000),
            new ShopItem("Pink Tome", 5000),
            new ShopItem("Septavius Ghost Robe", 5000),
            new ShopItem("Pink Wand", 5000),
            new ShopItem("Pink Ring", 5000),
            new ShopItem("Royality Ring of Depth Oceans", 5000),
            new ShopItem("Thessal's Hide", 5000),
            new ShopItem("Sharped corals of the oceans", 5000),
            new ShopItem("Sword of the Spirit Walker", 5000),
            new ShopItem("5000 Fame", 300)

        };

        public static readonly Dictionary<TileRegion, Tuple<List<ISellableItem>, CurrencyType, /*Rank Req*/int>> Shops = 
            new Dictionary<TileRegion, Tuple<List<ISellableItem>, CurrencyType, int>>()
        {
            { TileRegion.Store_1, new Tuple<List<ISellableItem>, CurrencyType, int>(Weapons, CurrencyType.Fame, 0) },
            { TileRegion.Store_2, new Tuple<List<ISellableItem>, CurrencyType, int>(Abilities, CurrencyType.Fame, 0) },
            { TileRegion.Store_3, new Tuple<List<ISellableItem>, CurrencyType, int>(Armor, CurrencyType.Fame, 0) },
            { TileRegion.Store_4, new Tuple<List<ISellableItem>, CurrencyType, int>(Rings, CurrencyType.Fame, 0) },
            { TileRegion.Store_5, new Tuple<List<ISellableItem>, CurrencyType, int>(Keys, CurrencyType.Fame, 0) },
            { TileRegion.Store_6, new Tuple<List<ISellableItem>, CurrencyType, int>(PurchasableFame, CurrencyType.Fame, 5) },
            { TileRegion.Store_7, new Tuple<List<ISellableItem>, CurrencyType, int>(Consumables, CurrencyType.Fame, 0) },
            { TileRegion.Store_8, new Tuple<List<ISellableItem>, CurrencyType, int>(Special, CurrencyType.Fame, 0) },
            { TileRegion.Store_21, new Tuple<List<ISellableItem>, CurrencyType, int>(DonorShop, CurrencyType.Gold, 0) },
            { TileRegion.Store_19, new Tuple<List<ISellableItem>, CurrencyType, int>(NexusShop, CurrencyType.Fame, 0) },
        };
        
        public static void Init(RealmManager manager)
        {
            foreach (var shop in Shops)
                foreach (var shopItem in shop.Value.Item1.OfType<ShopItem>())
                {
                    ushort id;
                    if (!manager.Resources.GameData.IdToObjectType.TryGetValue(shopItem.Name, out id))
                        Log.WarnFormat("Item name: {0}, not found.", shopItem.Name);
                    else
                        shopItem.SetItem(id);
                }
        }
    }
}
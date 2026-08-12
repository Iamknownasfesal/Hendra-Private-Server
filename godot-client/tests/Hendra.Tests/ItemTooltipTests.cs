using System.Linq;
using Hendra.Data;
using Hendra.Resources;
using Hendra.UI;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// What an item's tooltip says.
///
/// The wording is the original's, from the same language keys, and what appears is decided by what
/// the item is: a weapon by its shot, an ability by its cost, armour by what it adds. Listing every
/// field for every item would bury the one line that matters under a dozen reading "0".
/// </summary>
public sealed class ItemTooltipTests
{
    private static string[] Effects(ObjectDesc desc) =>
        ItemTooltip.Effects(desc).Select(line => line.Text).ToArray();

    [Fact]
    public void AWeaponIsDescribedByItsShot()
    {
        var bow = new ObjectDesc
        {
            Id = "Shortbow",
            SlotType = 3,
            Tier = 0,
            Projectiles = new System.Collections.Generic.Dictionary<int, ProjectileDesc>
            {
                // Speed times lifetime over ten thousand: 160 * 440 / 10000 = 7.04 tiles.
                [0] = new() { MinDamage = 15, MaxDamage = 45, Speed = 160f, LifetimeMs = 440, MultiHit = true },
            },
        };

        var lines = Effects(bow);

        Assert.Contains("Damage: 15 - 45", lines);
        Assert.Contains("Range: 7", lines);
        Assert.Contains("Shots hit multiple targets", lines);
        Assert.Equal("T0", ItemTooltip.TierTag(bow));
    }

    [Fact]
    public void ArmourIsDescribedByWhatItAdds()
    {
        var armour = new ObjectDesc
        {
            Id = "Wolfskin Armor",
            Tier = 1,
            EquipBonuses = new[] { ((int)StatsType.Defense, 5) },
        };

        var lines = Effects(armour);

        Assert.Contains("On Equip:", lines);
        Assert.Contains("  +5 Defense", lines);
        Assert.True(ItemTooltip.Effects(armour).First(l => l.Text == "On Equip:").IsHeading);
    }

    /// <summary>A penalty keeps its own sign rather than gaining a second one.</summary>
    [Fact]
    public void APenaltyReadsAsOne()
    {
        var heavy = new ObjectDesc { Id = "Heavy Thing", EquipBonuses = new[] { ((int)StatsType.Speed, -3) } };

        Assert.Contains("  -3 Speed", Effects(heavy));
    }

    /// <summary>
    /// An item with nothing to say says nothing, rather than a list of zeroes.
    /// </summary>
    [Fact]
    public void APlainItemHasNoEffects()
    {
        Assert.Empty(Effects(new ObjectDesc { Id = "Rock" }));
        Assert.Empty(Effects(null));
    }

    /// <summary>An item with no tier is untiered, not tier zero — the two read differently.</summary>
    [Fact]
    public void AnUntieredItemSaysSo()
    {
        Assert.Equal("UT", ItemTooltip.TierTag(new ObjectDesc { Id = "Something Special", Tier = -1 }));
    }

    /// <summary>Consumables have no notion of quality, so they carry no tag at all.</summary>
    [Fact]
    public void AConsumableHasNoTag()
    {
        Assert.Null(ItemTooltip.TierTag(new ObjectDesc { Id = "Health Potion", Consumable = true }));
    }

    [Fact]
    public void BindingAndBeingSpentAreRestrictions()
    {
        var potion = new ObjectDesc { Id = "Potion", Consumable = true, Soulbound = true };

        var lines = ItemTooltip.Restrictions(potion);

        Assert.Contains("Consumed with use", lines);
        Assert.Contains("Soulbound", lines);
    }

    /// <summary>The language table wins over the built-in English when it has an entry.</summary>
    [Fact]
    public void TheLanguageTableIsPreferred()
    {
        var strings = new Hendra.Text.StringMap();
        strings.Set("EquipmentToolTip.mpCost", "Coût en PM : {cost}");

        var wand = new ObjectDesc { Id = "Wand", MpCost = 20 };

        Assert.Contains("Coût en PM : 20", ItemTooltip.Effects(wand, strings).Select(l => l.Text));
    }
}

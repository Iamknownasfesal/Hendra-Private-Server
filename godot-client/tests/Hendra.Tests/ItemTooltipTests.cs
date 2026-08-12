using Hendra.Data;
using Hendra.Resources;
using Hendra.UI;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// What an item's tooltip says.
///
/// The point is that it says what the item *is*, not every field on it: a weapon by its damage,
/// armour by what it adds, an ability by what it costs. Listing all of them would bury the one line
/// that matters under a dozen reading "0".
/// </summary>
public sealed class ItemTooltipTests
{
    [Fact]
    public void AWeaponIsDescribedByItsShot()
    {
        var bow = new ObjectDesc
        {
            Id = "Shortbow",
            SlotType = 3,
            Tier = 0,
            Description = "A well-made shortbow.",
            Projectiles = new System.Collections.Generic.Dictionary<int, ProjectileDesc>
            {
                [0] = new() { MinDamage = 15, MaxDamage = 45, Speed = 160f, LifetimeMs = 440 },
            },
        };

        string text = ItemTooltip.Describe(bow);

        Assert.Contains("Shortbow   T0", text);
        Assert.Contains("Bow", text);
        Assert.Contains("Damage: 15–45", text);
        Assert.Contains("A well-made shortbow.", text);
    }

    [Fact]
    public void ArmourIsDescribedByWhatItAdds()
    {
        var armour = new ObjectDesc
        {
            Id = "Wolfskin Armor",
            SlotType = 6,
            Tier = 1,
            EquipBonuses = new[] { ((int)StatsType.Defense, 5) },
        };

        string text = ItemTooltip.Describe(armour);

        Assert.Contains("+5 Defense", text);
        Assert.Contains("Leather armour", text);
    }

    /// <summary>A negative bonus keeps its own sign rather than gaining a second one.</summary>
    [Fact]
    public void APenaltyReadsAsOne()
    {
        var cursed = new ObjectDesc
        {
            Id = "Heavy Thing",
            EquipBonuses = new[] { ((int)StatsType.Speed, -3) },
        };

        Assert.Contains("-3 Speed", ItemTooltip.Describe(cursed));
    }

    [Fact]
    public void NothingToDescribeIsNotAnError()
    {
        Assert.Null(ItemTooltip.Describe(null));
    }

    /// <summary>An item with no tier is untiered, not tier zero — the two read differently.</summary>
    [Fact]
    public void AnUntieredItemSaysNoTier()
    {
        var untiered = new ObjectDesc { Id = "Something Special", Tier = -1 };

        string text = ItemTooltip.Describe(untiered);

        Assert.Equal("Something Special", text);
    }
}

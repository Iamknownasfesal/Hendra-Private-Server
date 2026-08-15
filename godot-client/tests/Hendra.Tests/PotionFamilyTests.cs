using Hendra.Resources;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// Which consumables a potion slot counts as its own.
///
/// The rule is read off the item and not off a list of names: a drinkable in slot type 10 whose
/// Activate is Heal belongs to the health family, and the amount written on that Activate is what
/// drinking it restores. The content has twenty-nine of the first kind and twenty of the second,
/// which is why the alternative -- naming the two everyone knows -- is wrong.
/// </summary>
public sealed class PotionFamilyTests
{
    private static ObjectDesc Drink(string id, params ActivateDesc[] activates) => new()
    {
        Id = id,
        Consumable = true,
        SlotType = 10,
        Activates = activates,
    };

    [Fact]
    public void TheBasicPotionsAreTheirFamilies()
    {
        var health = Drink("Health Potion", new ActivateDesc("Heal", 100));
        var magic = Drink("Magic Potion", new ActivateDesc("Magic", 100));

        Assert.True(Potions.IsOf(health, PotionFamily.Health));
        Assert.False(Potions.IsOf(health, PotionFamily.Magic));
        Assert.True(Potions.IsOf(magic, PotionFamily.Magic));
        Assert.False(Potions.IsOf(magic, PotionFamily.Health));
    }

    [Fact]
    public void ADrinkThatIsNotCalledAPotionIsStillOne()
    {
        var fireWater = Drink("Fire Water", new ActivateDesc("Heal", 230));

        Assert.True(Potions.IsOf(fireWater, PotionFamily.Health));
        Assert.Equal(230, Potions.Amount(fireWater, PotionFamily.Health));
    }

    [Fact]
    public void ADrinkCanBelongToBothFamilies()
    {
        var coral = Drink("Coral Juice",
            new ActivateDesc("Heal", 100), new ActivateDesc("Magic", 100));

        Assert.Equal(100, Potions.Amount(coral, PotionFamily.Health));
        Assert.Equal(100, Potions.Amount(coral, PotionFamily.Magic));
    }

    [Fact]
    public void AStatPotionIsInNeitherFamily()
    {
        // Consumable, slot type ten, and still not something either counter counts: what it does
        // is raise a stat, and no amount of it refills a bar.
        var life = Drink("Greater Potion of Life", new ActivateDesc("IncrementStat", 5));

        Assert.False(Potions.IsOf(life, PotionFamily.Health));
        Assert.False(Potions.IsOf(life, PotionFamily.Magic));
    }

    [Fact]
    public void SomethingThatHealsButIsNotDrunkIsNotAPotion()
    {
        // An ability that heals is not a potion however plainly it heals: a slot type of its own
        // is what tells a tome from a bottle.
        var tome = new ObjectDesc
        {
            Id = "Tome of Purification",
            SlotType = 4,
            Activates = new[] { new ActivateDesc("Heal", 120) },
        };

        Assert.False(Potions.IsOf(tome, PotionFamily.Health));
    }

    [Fact]
    public void TheLargestAmountOfARepeatedVerbWins()
    {
        var brew = Drink("Saint Patty's Brew",
            new ActivateDesc("Heal", 100), new ActivateDesc("Heal", 140));

        Assert.Equal(140, Potions.Amount(brew, PotionFamily.Health));
    }
}

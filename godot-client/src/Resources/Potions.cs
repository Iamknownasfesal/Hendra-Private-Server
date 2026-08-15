namespace Hendra.Resources;

/// <summary>Which vital a potion refills.</summary>
public enum PotionFamily
{
    Health,
    Magic,
}

/// <summary>
/// Which consumables refill a vital, and by how much.
/// </summary>
/// <remarks>
/// <para>
/// Nothing in the item data says "this is a health potion". What it says is that an object is
/// <c>Consumable</c>, sits in slot type 10, and declares an <c>Activate</c> of <c>Heal</c> with an
/// amount. That triple is the whole rule, and reading it rather than matching ids is what makes
/// Fire Water and Chardonnay members of the same family as the Health Potion: the content has
/// twenty-nine items that heal and twenty that restore magic, and a list of names would be wrong
/// the first time either grew.
/// </para>
/// <para>
/// A few items are in both families -- Coral Juice and Mad God Ale heal and restore magic in the
/// same mouthful -- which the data expresses as two <c>Activate</c> elements and this treats as
/// membership of two families rather than of some third one.
/// </para>
/// </remarks>
public static class Potions
{
    /// <summary>The slot type every drinkable shares. Ten in the original's numbering.</summary>
    public const int SlotType = 10;

    /// <summary>
    /// The object types the server pins to its two stacks.
    /// </summary>
    /// <remarks>
    /// The stacked potions are not inventory: the server builds an <c>ItemStacker</c> per slot in
    /// <c>Player.Init</c> and hands it one object type for the character's whole life, so a stack
    /// holds that item and refuses every other. These are those two types, which is why the cells
    /// can name the item they hold without the server ever sending it.
    /// </remarks>
    public const ushort HealthStackType = 0x0A22;

    public const ushort MagicStackType = 0x0A23;

    /// <summary>The <c>Activate</c> verb that refills a family's vital.</summary>
    public static string Verb(PotionFamily family) =>
        family == PotionFamily.Health ? "Heal" : "Magic";

    /// <summary>The object type the server's stack for this family holds.</summary>
    public static ushort StackType(PotionFamily family) =>
        family == PotionFamily.Health ? HealthStackType : MagicStackType;

    /// <summary>
    /// Whether an item belongs to a family: a drinkable that refills that family's vital.
    /// </summary>
    public static bool IsOf(ObjectDesc desc, PotionFamily family) => Amount(desc, family) > 0;

    /// <summary>
    /// How much of the family's vital one of these restores, straight off the item's own
    /// <c>Activate</c>. Zero for anything that is not a member of the family.
    /// </summary>
    public static int Amount(ObjectDesc desc, PotionFamily family)
    {
        if (desc == null || !desc.Consumable || desc.SlotType != SlotType)
            return 0;

        return desc.ActivateAmount(Verb(family));
    }

    /// <summary>The descriptor of the item a family's stack holds, or null if the data lacks it.</summary>
    public static ObjectDesc StackItem(GameData data, PotionFamily family) =>
        data?.GetObject(StackType(family));
}

using System.Collections.Generic;

namespace Hendra.Resources;

/// <summary>
/// What kind of thing an item is, for the vault's filter rail.
/// </summary>
/// <remarks>
/// <para>
/// The item data has no category field -- it has a slot type, which says which square on the
/// character an item goes in, and there are two dozen of those. Twenty-four buttons down the side
/// of the panel would be a worse question than the one they answer, so slot types are grouped.
/// </para>
/// <para>
/// The grouping lives here, in the data layer, and not in the panel. The panel asks what categories
/// exist and draws that, in that order; adding a category or moving an item between them is a
/// change to this file and to nothing that draws. That is as close to the brief's "categories come
/// from item data" as data with no categories in it can be brought.
/// </para>
/// </remarks>
public static class ItemCategories
{
    public const string All = "ALL";

    /// <summary>Every category, in the order the rail stacks them.</summary>
    public static readonly string[] Order =
    {
        "Weapon", "Armor", "Heavy", "Ring", "Ability", "Consumable", "Pet", "Special",
    };

    /// <summary>
    /// Slot type to category.
    /// </summary>
    /// <remarks>
    /// The numbering is the original's and is fixed by the data files: 1 is a sword, 9 a ring, 10 a
    /// potion. Anything not named here is Special, which is where the one-off and the unrecognised
    /// both belong -- an unknown slot type should appear under some button rather than nowhere.
    /// </remarks>
    private static readonly Dictionary<int, string> BySlotType = new()
    {
        // Weapons.
        [1] = "Weapon", [2] = "Weapon", [3] = "Weapon", [8] = "Weapon",
        [17] = "Weapon", [24] = "Weapon",

        // Worn.
        [6] = "Armor", [14] = "Armor",
        [7] = "Heavy",
        [9] = "Ring",

        // Abilities: the second square, whatever class fills it.
        [4] = "Ability", [5] = "Ability", [11] = "Ability", [12] = "Ability",
        [13] = "Ability", [15] = "Ability", [16] = "Ability", [18] = "Ability",
        [19] = "Ability", [20] = "Ability", [21] = "Ability", [22] = "Ability",
        [23] = "Ability", [25] = "Ability",

        [10] = "Consumable",
        [26] = "Pet",
    };

    /// <summary>Which category an item belongs to. Never null for a real item.</summary>
    public static string Of(ObjectDesc desc)
    {
        if (desc == null)
            return null;

        if (BySlotType.TryGetValue(desc.SlotType, out string category))
            return category;

        // Consumables are a behaviour rather than a slot -- a health potion is slot type 10, but a
        // stat potion is not equipment at all and would otherwise fall through to Special.
        return desc.Consumable ? "Consumable" : "Special";
    }
}

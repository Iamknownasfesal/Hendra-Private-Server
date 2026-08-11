using Hendra.Data;

namespace Hendra.World;

/// <summary>
/// Applies the stat deltas carried by Update and NewTick onto entities.
/// </summary>
/// <remarks>
/// <para>
/// The equipment layout is one array with three regions, which is not obvious from the wire format:
/// <c>Inventory0..15</c> map to indices 0-15 (0-7 worn, 8-15 carried) and <c>Backpack0..7</c>
/// continue at 16-23. A slot holding nothing is -1.
/// </para>
/// <para>
/// Condition effects arrive split across two int stats, and reassembling them is the one piece of
/// arithmetic here that can go quietly wrong — see <see cref="ConditionEffectsCodec"/> for why the
/// halves overlap by a bit.
/// </para>
/// </remarks>
public static class StatApplier
{
    /// <summary>Worn equipment, carried inventory and backpack together.</summary>
    public const int TotalSlots = 24;

    private const int BackpackFirstIndex = 16;

    public static void Apply(Entity entity, in ObjectStats stats, bool isSelf)
    {
        if (stats.Stats == null)
            return;

        foreach (var stat in stats.Stats)
            Apply(entity, stat, isSelf);
    }

    public static void Apply(Entity entity, in StatData stat, bool isSelf)
    {
        var player = entity as LocalPlayer;

        if (stat.Type.IsInventorySlot())
        {
            SetSlot(entity, stat.Type - StatsType.Inventory0, stat.IntValue);
            return;
        }

        if (stat.Type.IsBackpackSlot())
        {
            SetSlot(entity, BackpackFirstIndex + (stat.Type - StatsType.Backpack0), stat.IntValue);
            return;
        }

        switch (stat.Type)
        {
            case StatsType.MaxHp: entity.MaxHp = stat.IntValue; break;
            case StatsType.Hp: entity.Hp = stat.IntValue; break;
            case StatsType.Size: entity.Size = stat.IntValue; break;
            case StatsType.Level: entity.Level = stat.IntValue; break;
            case StatsType.Defense: entity.Defense = stat.IntValue; break;
            case StatsType.Name: entity.Name = stat.StringValue; break;
            case StatsType.Tex1: entity.Texture1 = stat.IntValue; break;
            case StatsType.Tex2: entity.Texture2 = stat.IntValue; break;
            case StatsType.AltTextureIndex: entity.AltTextureIndex = stat.IntValue; break;

            case StatsType.Condition:
                entity.Conditions = ConditionEffectsCodec.WithEffects(entity.Conditions, stat.IntValue);
                break;

            case StatsType.Condition2:
                entity.Conditions = ConditionEffectsCodec.WithEffects2(entity.Conditions, stat.IntValue);
                break;

            case StatsType.SinkLevel:
                // The client owns how deep its own player has sunk, because it is derived from the
                // movement it performed locally. Accepting the server's value would fight with that.
                if (!isSelf)
                    entity.SinkLevel = stat.IntValue;
                break;

            case StatsType.MerchandiseType: entity.MerchandiseType = stat.IntValue; break;
            case StatsType.MerchandisePrice: entity.MerchandisePrice = stat.IntValue; break;
            case StatsType.MerchandiseCurrency: entity.MerchandiseCurrency = stat.IntValue; break;
            case StatsType.MerchandiseCount: entity.MerchandiseCount = stat.IntValue; break;
            case StatsType.MerchandiseRankReq: entity.MerchandiseRankRequired = stat.IntValue; break;

            case StatsType.MaxMp when player != null: player.MaxMp = stat.IntValue; break;
            case StatsType.Mp when player != null: player.Mp = stat.IntValue; break;
            case StatsType.Attack when player != null: player.Attack = stat.IntValue; break;
            case StatsType.Speed when player != null: player.Speed = stat.IntValue; break;
            case StatsType.Vitality when player != null: player.Vitality = stat.IntValue; break;
            case StatsType.Wisdom when player != null: player.Wisdom = stat.IntValue; break;
            case StatsType.Dexterity when player != null: player.Dexterity = stat.IntValue; break;
            case StatsType.Breath when player != null: player.Breath = stat.IntValue; break;
            case StatsType.Credits when player != null: player.Credits = stat.IntValue; break;
            case StatsType.CurrentFame when player != null: player.Fame = stat.IntValue; break;

            // Sent as a number rather than a flag, and it is what makes the last eight inventory
            // slots real -- the server refuses a swap into them for a character without one.
            case StatsType.HasBackpack when player != null: player.HasBackpack = stat.IntValue != 0; break;
        }
    }

    private static void SetSlot(Entity entity, int index, int objectType)
    {
        if (index < 0 || index >= TotalSlots)
            return;

        entity.Equipment ??= NewEmptyInventory();
        entity.Equipment[index] = objectType;
    }

    /// <summary>An inventory with every slot empty. Empty is -1, not zero — zero is a valid type.</summary>
    public static int[] NewEmptyInventory()
    {
        var slots = new int[TotalSlots];
        for (int i = 0; i < slots.Length; i++)
            slots[i] = -1;
        return slots;
    }
}

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
            // The two dye layers. Which one a dye paints is the content's to say -- the clothing
            // dye and the accessory dye of one colour carry the same number and differ only in
            // which element declared it (`Player.UseItem.cs:583-589`), so the two arrive apart and
            // are never worked out from the value.
            case StatsType.Tex1: entity.Texture1 = stat.IntValue; break;
            case StatsType.Tex2: entity.Texture2 = stat.IntValue; break;

            // Which neighbours a wall or a fence joins onto, which picks its connector shape
            // (`ConnectedObject.as:98`).
            case StatsType.ObjectConnection: entity.ConnectType = stat.IntValue; break;

            // Whether a portal will let anybody through. `PortalPanel` hides its Enter button
            // while this is off (`PortalPanel.as:92-97`).
            case StatsType.PortalActive: entity.PortalActive = stat.IntValue != 0; break;
            case StatsType.AltTextureIndex: entity.AltTextureIndex = stat.IntValue; break;
            case StatsType.Skin: entity.Skin = stat.IntValue; break;

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
            case StatsType.Breath when player != null: player.Breath = stat.IntValue; break;
            case StatsType.Attack when player != null: player.Attack = stat.IntValue; break;
            case StatsType.Speed when player != null: player.Speed = stat.IntValue; break;
            case StatsType.Vitality when player != null: player.Vitality = stat.IntValue; break;
            case StatsType.Wisdom when player != null: player.Wisdom = stat.IntValue; break;
            case StatsType.Dexterity when player != null: player.Dexterity = stat.IntValue; break;

            // The equipped weapon's damage bounds and the private-drop bonus: the three stats the
            // eight-stat character sheet has no row for, sent because a client cannot work the
            // first two out from an item tooltip once a bonus has moved them.
            case StatsType.DamageMin when player != null: player.DamageMin = stat.IntValue; break;
            case StatsType.DamageMax when player != null: player.DamageMax = stat.IntValue; break;
            case StatsType.Luck when player != null: player.Luck = stat.IntValue; break;
            case StatsType.Credits when player != null: player.Credits = stat.IntValue; break;
            // Two different things, and the original sends both: CurrentFame is the account's
            // money and Fame is what this character has earned (Player.cs:291-292). One field for
            // both would leave every vendor charging against a purse nothing is ever spent from.
            case StatsType.CurrentFame when player != null: player.CurrentFame = stat.IntValue; break;
            case StatsType.Prestige when player != null: player.Prestige = stat.IntValue; break;
            case StatsType.Fame when player != null: player.Fame = stat.IntValue; break;
            case StatsType.GuildName: entity.Guild = stat.StringValue; break;
            case StatsType.GuildRank: entity.GuildRank = stat.IntValue; break;

            // The rating beside a name and the mark that recolours it. Both belong to the account
            // rather than to the body, and both are drawn into the name plate rather than written
            // out (`FameUtil.numStarsToIcon(numStars_, admin_)`, `Player.as:750-756`).
            case StatsType.NumStars: entity.Stars = stat.IntValue; break;
            case StatsType.Admin: entity.Admin = stat.IntValue != 0; break;

            // The halo `/glow` sets, which the original paints around the whole sprite
            // (`GameObject.setGlow`, `GlowRedrawer.as:19-46`). Zero puts it out.
            case StatsType.GlowColor: entity.GlowColor = stat.IntValue; break;
            case StatsType.Exp when player != null: player.Experience = stat.IntValue; break;

            case StatsType.MaxHpBoost when player != null: player.Boosts[0] = stat.IntValue; break;
            case StatsType.MaxMpBoost when player != null: player.Boosts[1] = stat.IntValue; break;
            case StatsType.AttackBoost when player != null: player.Boosts[2] = stat.IntValue; break;
            case StatsType.DefenseBoost when player != null: player.Boosts[3] = stat.IntValue; break;
            case StatsType.SpeedBoost when player != null: player.Boosts[4] = stat.IntValue; break;
            case StatsType.DexterityBoost when player != null: player.Boosts[5] = stat.IntValue; break;
            case StatsType.VitalityBoost when player != null: player.Boosts[6] = stat.IntValue; break;
            case StatsType.WisdomBoost when player != null: player.Boosts[7] = stat.IntValue; break;

            // The last three of the eleven, which the eight-row sheet has no line for: what is worn
            // moves the weapon's own damage bounds and the private-drop bonus too
            // (`Player.cs:350-352`).
            case StatsType.DamageMinBonus when player != null: player.Boosts[8] = stat.IntValue; break;
            case StatsType.DamageMaxBonus when player != null: player.Boosts[9] = stat.IntValue; break;
            case StatsType.LuckBonus when player != null: player.Boosts[10] = stat.IntValue; break;
            case StatsType.NextLevelExp when player != null: player.NextLevelExperience = stat.IntValue; break;
            case StatsType.NextClassQuestFame when player != null: player.NextClassQuestFame = stat.IntValue; break;

            // Whether this account picked its own name, which colours it over the head
            // (`Player.getNameColor`, `Player.as:757-764`).
            case StatsType.NameChosen: entity.NameChosen = stat.IntValue != 0; break;

            // What is left of the three timed boosts, in whole seconds. The flag beside the first
            // is derived from its own clock in the original (`Player.cs:355`), so only the clocks
            // are kept and the flag is asked of them.
            case StatsType.XpTimer when player != null: player.ExperienceBoostSeconds = stat.IntValue; break;
            case StatsType.LdTimer when player != null: player.LootDropBoostSeconds = stat.IntValue; break;
            case StatsType.LtTimer when player != null: player.LootTierBoostSeconds = stat.IntValue; break;

            // Sent as a number rather than a flag, and it is what makes the last eight inventory
            // slots real -- the server refuses a swap into them for a character without one.
            case StatsType.HasBackpack when player != null: player.HasBackpack = stat.IntValue != 0; break;

            // The two counted potion stacks, which the vitals row shows beside the bar each one
            // refills. They are not slots in the inventory array; see LocalPlayer.
            case StatsType.HealthPotionStack when player != null: player.HealthPotions = stat.IntValue; break;
            case StatsType.MagicPotionStack when player != null: player.MagicPotions = stat.IntValue; break;
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

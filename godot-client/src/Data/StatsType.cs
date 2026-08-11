namespace Hendra.Data;

/// <summary>
/// Per-entity stat identifiers, mirroring wServer/realm/Stats.cs.
///
/// These arrive inside <see cref="ObjectStats"/> as an id byte followed by a value. Most values are
/// int32; a small set is string-typed — see <see cref="StatsTypeExtensions.IsStringStat"/>.
/// </summary>
public enum StatsType : byte
{
    MaxHp = 0,
    Hp = 1,
    Size = 2,
    MaxMp = 3,
    Mp = 4,
    NextLevelExp = 5,
    Exp = 6,
    Level = 7,

    Inventory0 = 8,
    Inventory1 = 9,
    Inventory2 = 10,
    Inventory3 = 11,
    Inventory4 = 12,
    Inventory5 = 13,
    Inventory6 = 14,
    Inventory7 = 15,
    Inventory8 = 16,
    Inventory9 = 17,
    Inventory10 = 18,
    Inventory11 = 19,
    Inventory12 = 20,
    Inventory13 = 21,
    Inventory14 = 22,
    Inventory15 = 23,

    Attack = 24,
    Defense = 25,
    Speed = 26,
    Vitality = 27,
    Wisdom = 28,
    Dexterity = 29,

    /// <summary>Condition bits 1..31, as <c>1 &lt;&lt; (id - 1)</c>.</summary>
    Condition = 30,

    NumStars = 31,
    Name = 32,
    Tex1 = 33,
    Tex2 = 34,
    MerchandiseType = 35,
    Credits = 36,
    MerchandisePrice = 37,
    PortalActive = 38,
    AccountId = 39,
    Fame = 40,
    MerchandiseCurrency = 41,
    ObjectConnection = 42,
    MerchandiseCount = 43,
    MerchandiseMinsLeft = 44,
    MerchandiseDiscount = 45,
    MerchandiseRankReq = 46,

    MaxHpBoost = 47,
    MaxMpBoost = 48,
    AttackBoost = 49,
    DefenseBoost = 50,
    SpeedBoost = 51,
    VitalityBoost = 52,
    WisdomBoost = 53,
    DexterityBoost = 54,

    OwnerAccountId = 55,
    RankRequired = 56,
    NameChosen = 57,
    CurrentFame = 58,
    NextClassQuestFame = 59,
    GlowColor = 60,
    SinkLevel = 61,
    AltTextureIndex = 62,
    GuildName = 63,
    GuildRank = 64,
    Breath = 65,
    XpBoosted = 66,
    XpTimer = 67,
    LdTimer = 68,
    LtTimer = 69,
    HealthPotionStack = 70,
    MagicPotionStack = 71,

    Backpack0 = 72,
    Backpack1 = 73,
    Backpack2 = 74,
    Backpack3 = 75,
    Backpack4 = 76,
    Backpack5 = 77,
    Backpack6 = 78,
    Backpack7 = 79,

    HasBackpack = 80,
    Skin = 81,

    /// <summary>Condition bits 32..63, as <c>1 &lt;&lt; (id - 32)</c>.</summary>
    Condition2 = 82,

    DamageMin = 83,
    DamageMax = 84,
    DamageMinBonus = 85,
    DamageMaxBonus = 86,
    LuckBonus = 87,
    Rank = 88,
    Admin = 89,
    Luck = 90,
    Prestige = 91,

    None = 255,
}

public static class StatsTypeExtensions
{
    /// <summary>
    /// Whether this stat's value is a length-prefixed string rather than an int32.
    /// </summary>
    /// <remarks>
    /// Verified against what the server actually puts in the dictionary, not against its reader.
    /// <c>ObjectStats.Write</c> dispatches on the boxed runtime type, and <c>Player.ExportStats</c> /
    /// <c>Container.ExportStats</c> assign <c>AccountId</c> and <c>OwnerAccountId</c> via
    /// <c>.ToString()</c> — so four stats are string-typed on the wire. The server's own
    /// <c>ObjectStats.Read</c> only special-cases Name and Guild, but that path is never exercised
    /// (ObjectStats only ever flows server-to-client), so its narrower list is a latent bug rather
    /// than the contract. Getting this wrong does not fail loudly: it desynchronises the reader and
    /// silently corrupts every remaining stat in the packet.
    /// </remarks>
    public static bool IsStringStat(this StatsType type) => type
        is StatsType.Name
        or StatsType.GuildName
        or StatsType.AccountId
        or StatsType.OwnerAccountId;

    /// <summary>Inventory0..15 map to equipment indices 0..15 (0-7 equipped, 8-15 inventory).</summary>
    public static bool IsInventorySlot(this StatsType type) =>
        type >= StatsType.Inventory0 && type <= StatsType.Inventory15;

    /// <summary>Backpack0..7 map to equipment indices 16..23.</summary>
    public static bool IsBackpackSlot(this StatsType type) =>
        type >= StatsType.Backpack0 && type <= StatsType.Backpack7;
}

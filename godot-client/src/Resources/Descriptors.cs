using System;
using System.Collections.Generic;
using Hendra.Data;

namespace Hendra.Resources;

/// <summary>How an object's artwork is specified in XML.</summary>
public enum TextureKind
{
    None,

    /// <summary>A single cell of a sheet.</summary>
    Sprite,

    /// <summary>A character animation set.</summary>
    AnimatedChar,

    /// <summary>One of several variants, chosen deterministically from the entity id.</summary>
    Random,

    /// <summary>Fetched from the app server rather than shipped with the client.</summary>
    Remote,
}

/// <summary>
/// An object's artwork, as declared in XML and before it is resolved against the asset library.
/// </summary>
/// <remarks>
/// Kept separate from the loaded sprite so the XML can be parsed without the engine present, and so
/// remote textures — which arrive later over HTTP — can slot into the same structure.
/// </remarks>
public sealed class TextureSpec
{
    public TextureKind Kind = TextureKind.None;

    /// <summary>Sheet name for <see cref="TextureKind.Sprite"/> and <see cref="TextureKind.AnimatedChar"/>.</summary>
    public string File;

    public int Index;

    /// <summary>Variants for <see cref="TextureKind.Random"/>, picked as <c>entityId % Count</c>.</summary>
    public List<TextureSpec> Variants;

    /// <summary>The recolour mask, when declared separately from the texture.</summary>
    public TextureSpec Mask;

    /// <summary>Alternate artwork keyed by the AltTextureIndex stat.</summary>
    public Dictionary<int, TextureSpec> Alternates;

    /// <summary>Identifier for <see cref="TextureKind.Remote"/>, fetched from /app/getTextures.</summary>
    public string RemoteId;

    /// <summary>Whether a remote character sheet's first row faces right rather than down.</summary>
    public bool RemoteFacesRight;

    /// <summary>Particle effect attached to the object, if any.</summary>
    public string EffectId;
}

/// <summary>One of an object's projectile definitions.</summary>
public sealed class ProjectileDesc
{
    /// <summary>Index within the owning object, matching the BulletType on the wire.</summary>
    public int BulletType;

    /// <summary>The object id whose artwork and properties the projectile borrows.</summary>
    public string ObjectId;

    public int LifetimeMs;

    /// <summary>Tiles per 10,000 ms — the unit the trajectory maths expects.</summary>
    public float Speed;

    /// <summary>-1 means "inherit the owner's Size".</summary>
    public int Size = -1;

    public int MinDamage;
    public int MaxDamage;

    public bool MultiHit;
    public bool PassesCover;
    public bool ArmorPiercing;
    public bool Wavy;
    public bool Parametric;
    public bool Boomerang;
    public bool FaceDir;

    public float Amplitude;
    public float Frequency = 1f;
    public float Magnitude = 3f;

    public bool ParticleTrail;
    public int ParticleTrailColor = 0xFF00FF;
    public int ParticleTrailLifetimeMs = 600;

    public List<ConditionEffectIndex> Effects;

    /// <summary>Effects flagged <c>target="1"</c>, which a pet applies rather than the shooter.</summary>
    public HashSet<ConditionEffectIndex> PetEffects;
}

/// <summary>Movement and rendering overrides that apply only while an object is moving.</summary>
public sealed class WhileMovingDesc
{
    public float Z;
    public bool Flying;
}

/// <summary>
/// One <c>Activate</c> verb an item declares, and the amount written on it.
/// </summary>
/// <remarks>
/// The amount means whatever the verb means -- hit points for <c>Heal</c>, magic points for
/// <c>Magic</c>, a stat's increment for <c>IncrementStat</c> -- and is zero for the verbs that
/// carry no number at all, such as <c>Backpack</c>.
/// </remarks>
public readonly struct ActivateDesc
{
    public ActivateDesc(string effect, int amount)
    {
        Effect = effect ?? string.Empty;
        Amount = amount;
    }

    public readonly string Effect;
    public readonly int Amount;

    /// <summary>Whether this is the named verb. The XML's casing is not guaranteed.</summary>
    public bool Is(string effect) =>
        string.Equals(Effect, effect, System.StringComparison.OrdinalIgnoreCase);
}

/// <summary>
/// Everything the client needs to know about an object type: how it renders, how it blocks
/// movement, and what it shoots.
/// </summary>
/// <remarks>
/// This is the client-side <c>ObjectProperties</c>, not the server's <c>ObjectDesc</c>. The two
/// read the same XML but care about different halves of it — the server ignores artwork, and the
/// client ignores loot tables and spawn rules.
/// </remarks>
public sealed class ObjectDesc
{
    public ushort Type;
    public string Id;
    public string DisplayId;
    public string Class;
    public string Group;
    public string DungeonName;

    // Presence flags.
    public bool IsPlayer;

    /// <summary>
    /// The highest each of the eight stats can be raised to for this class, indexed the way the
    /// stat manager indexes them: MaxHP, MaxMP, Attack, Defense, Speed, Dexterity, Vitality, Wisdom.
    /// </summary>
    /// <remarks>
    /// The XML names two of them after what they do rather than what they are called on screen --
    /// HpRegen is Vitality and MpRegen is Wisdom -- which is why the parse maps them by name.
    /// Null for anything that is not a player class.
    /// </remarks>
    public int[] StatMaxima;
    public bool IsEnemy;

    /// <summary>A Hero of Oryx, or a boss of one of the Realm's roaming encounters.</summary>
    /// <remarks>
    /// Two separate flags in the data and one thing to a player looking at a map: something worth
    /// crossing the Realm for. Marked at any distance rather than only when it is already close
    /// enough to see, which is the whole reason a player looks at the map.
    /// </remarks>
    public bool IsHero;

    public bool IsEncounter;

    /// <summary>A god. Common enough to be worth a colour of its own rather than a mark.</summary>
    public bool IsGod;

    public bool DrawOnGround;
    public bool DrawUnder;
    public bool OccupySquare;
    public bool FullOccupy;
    public bool EnemyOccupySquare;
    public bool Static;
    public bool NoMiniMap;
    public bool ProtectFromGroundDamage;
    public bool ProtectFromSink;
    public bool Flying;
    public bool ShowName;
    public bool DontFaceAttacks;
    public bool BlocksSight;
    public bool Connects;
    public bool IsIntergamePortal;

    public bool StunImmune;
    public bool ParalyzeImmune;
    public bool DazedImmune;

    public int ShadowSize = 100;
    public int ShadowColor;
    public float Z;
    public int Color = 0xFFFFFF;

    public int MinSize = 100;
    public int MaxSize = 100;
    public int SizeStep = 5;

    /// <summary>Radians, stored as the XML value multiplied by a quarter turn.</summary>
    public float AngleCorrection;

    /// <summary>
    /// The XML's Rotation, raw.
    /// </summary>
    /// <remarks>
    /// Two meanings share the field. On a model object it is a yaw in degrees. On a projectile it
    /// is a period: elapsed milliseconds divided by it give the spin in radians, so a larger number
    /// is a slower spin and zero means none.
    /// </remarks>

    /// <summary>Milliseconds for a full turn. Zero means the sprite does not spin.</summary>
    /// <summary>How far a model is turned about the vertical, in radians. Declared in degrees.</summary>
    public float Rotation;

    public float BloodProb;
    public int BloodColor = 0xFF0000;

    public int MaxHitPoints = 200;
    public int Defense;
    public int Tex1;
    public int Tex2;

    public string Model;
    public string HitSound;
    public string DeathSound;

    public int[] SlotTypes;

    // ---- Item fields. Only meaningful for equipment; harmless defaults otherwise. ----

    /// <summary>Which equipment slot this fits. -1 means it is not equipment.</summary>
    public int SlotType = -1;

    /// <summary>Quality, zero upward. Negative for anything untiered, which is how UT items read.</summary>
    public int Tier = -1;

    /// <summary>The flavour line the tooltip ends with.</summary>
    public string Description;

    /// <summary>Fame earned for feeding this to a pet, and a rough proxy for how good it is.</summary>
    public int FeedPower;

    /// <summary>
    /// What equipping this adds to each stat, as (stat, amount) pairs.
    /// </summary>
    /// <remarks>
    /// The stat is one of the wire enum's, translated from the XML's own numbering on the way in --
    /// the two disagree, and 21 means Defense in the file and Inventory13 in the enum.
    /// </remarks>
    public (int Stat, int Amount)[] EquipBonuses = System.Array.Empty<(int, int)>();

    /// <summary>
    /// Multiplier on the wielder's attack rate. The server's shot-cooldown check divides by this,
    /// so a mismatch here means shots are silently rejected.
    /// </summary>
    public float RateOfFire = 1f;

    /// <summary>Shots per volley. Every one is sent as its own packet sharing a single timestamp.</summary>
    public int NumProjectiles = 1;

    /// <summary>Angle between shots in a volley, in radians. Defaults to 11.25 degrees.</summary>
    public float ArcGap = 11.25f * MathF.PI / 180f;

    public int MpCost;
    public int MpEndCost;
    public bool Consumable;
    public bool Usable;
    public bool Soulbound;

    /// <summary>Ability cooldown in milliseconds. Zero means the default half second.</summary>
    public int CooldownMs;

    /// <summary>
    /// Whether using this is a press-and-release rather than a single press.
    /// </summary>
    /// <remarks>
    /// A multi-phase ability charges while the key is held and resolves on release, and it costs
    /// magic at both ends -- <see cref="MpCost"/> to begin and <see cref="MpEndCost"/> to finish.
    /// </remarks>
    public bool MultiPhase;

    /// <summary>
    /// What this item does when used: every <c>Activate</c> verb it declares, with its amount.
    /// </summary>
    /// <remarks>
    /// Resolving an activation is the server's business, but the verb and its amount are the only
    /// statement the data makes about what an item is for -- a potion is not marked as a health
    /// potion anywhere, it is an item whose activation is <c>Heal</c>. Anything that wants to sort
    /// consumables by what they do reads this rather than matching names.
    /// </remarks>
    public ActivateDesc[] Activates = System.Array.Empty<ActivateDesc>();

    /// <summary>Whether using this fires projectiles the client has to author itself.</summary>
    public bool ActivatesShoot;

    /// <summary>
    /// The amount named by an activation verb, or zero if this item does not declare it.
    /// </summary>
    /// <remarks>
    /// Verbs repeat -- Coral Juice both heals and restores magic -- so the largest amount wins,
    /// which is the same one the server applies when it walks the list.
    /// </remarks>
    public int ActivateAmount(string effect)
    {
        int amount = 0;
        foreach (var activate in Activates)
            if (activate.Is(effect) && activate.Amount > amount)
                amount = activate.Amount;

        return amount;
    }

    public WhileMovingDesc WhileMoving;
    public TextureSpec Texture;
    public TextureSpec TopTexture;
    public TextureSpec Portrait;

    /// <summary>Projectiles keyed by their XML id, which is the BulletType on the wire.</summary>
    public Dictionary<int, ProjectileDesc> Projectiles;

    /// <summary>Sounds keyed by the SoundId a PlaySound packet carries.</summary>
    public Dictionary<int, string> Sounds;

    /// <summary>
    /// A concrete size for a new instance. Objects declare a range and a step, and the original
    /// picks a random multiple of the step within it.
    /// </summary>
    public int RollSize(System.Random random) =>
        MaxSize <= MinSize || SizeStep <= 0
            ? MinSize
            : MinSize + random.Next((MaxSize - MinSize) / SizeStep + 1) * SizeStep;
}

/// <summary>How an animated tile scrolls or ripples.</summary>
public enum GroundAnimation
{
    None = 0,
    Wave = 1,
    Flow = 2,
}

/// <summary>
/// A terrain type: how it looks, whether it can be walked on, and what it does to whoever stands
/// on it.
/// </summary>
public sealed class GroundDesc
{
    public ushort Type;
    public string Id;

    public bool NoWalk;
    public int MinDamage;
    public int MaxDamage;

    /// <summary>A conveyor tile; the push direction comes from the animation vector.</summary>
    public bool Push;

    public GroundAnimation Animation = GroundAnimation.None;
    public float AnimationDx;
    public float AnimationDy;

    /// <summary>Higher priority bleeds over lower when two terrains meet. -1 means never blend.</summary>
    public int BlendPriority = -1;

    public int CompositePriority;

    /// <summary>Movement multiplier for anything standing on it.</summary>
    public float Speed = 1f;

    /// <summary>How much momentum carries over on ice. 0 means normal footing.</summary>
    public float SlideAmount;

    public float XOffset;
    public float YOffset;

    /// <summary>Sprites standing here are drawn partially submerged.</summary>
    public bool Sink;

    /// <summary>Sprites standing here sink progressively the longer they stay.</summary>
    public bool Sinking;

    public bool RandomOffset;
    public bool SameTypeEdgeMode;

    public int Color = -1;

    public TextureSpec Texture;
    public TextureSpec EdgeTexture;
    public TextureSpec CornerTexture;
    public TextureSpec InnerCornerTexture;
    public TextureSpec TopTexture;

    public GroundAnimation TopAnimation = GroundAnimation.None;
    public float TopAnimationDx;
    public float TopAnimationDy;

    public bool HasEdge => EdgeTexture != null;
}

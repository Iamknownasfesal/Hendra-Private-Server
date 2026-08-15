using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Xml.Linq;
using Hendra.Data;

namespace Hendra.Resources;

/// <summary>
/// The parsed game data: every object and terrain type the client knows about.
/// </summary>
/// <remarks>
/// Deliberately free of any engine dependency, so the XML can be parsed and tested without Godot.
/// Artwork is kept as an unresolved <see cref="TextureSpec"/> for the same reason; binding it to
/// actual textures happens separately.
///
/// The original parsed this XML with E4X and a <c>hasOwnProperty</c> call per field per object,
/// re-walking the document on every lookup. Here it is parsed once into plain objects.
/// </remarks>
public sealed class GameData
{
    private readonly Dictionary<ushort, ObjectDesc> _objectsByType = new();
    private readonly Dictionary<string, ObjectDesc> _objectsById = new(StringComparer.Ordinal);
    private readonly Dictionary<ushort, GroundDesc> _groundByType = new();
    private readonly Dictionary<string, GroundDesc> _groundById = new(StringComparer.Ordinal);

    public IReadOnlyDictionary<ushort, ObjectDesc> Objects => _objectsByType;
    public IReadOnlyDictionary<ushort, GroundDesc> Ground => _groundByType;

    /// <summary>Player classes, in the order the XML declares them.</summary>
    public List<ObjectDesc> PlayerClasses { get; } = new();

    public ObjectDesc GetObject(ushort type) =>
        _objectsByType.TryGetValue(type, out var desc) ? desc : null;

    public ObjectDesc GetObject(string id) =>
        id != null && _objectsById.TryGetValue(id, out var desc) ? desc : null;

    public GroundDesc GetGround(ushort type) =>
        _groundByType.TryGetValue(type, out var desc) ? desc : null;

    public GroundDesc GetGround(string id) =>
        id != null && _groundById.TryGetValue(id, out var desc) ? desc : null;

    /// <summary>
    /// Merges one object XML document. Later documents override earlier ones for the same type,
    /// which is how per-dungeon overlays replace base definitions.
    /// </summary>
    public void AddObjects(string xml)
    {
        foreach (var element in Root(xml).Elements("Object"))
        {
            var desc = ParseObject(element);
            if (desc == null)
                continue;

            _objectsByType[desc.Type] = desc;
            if (desc.Id != null)
                _objectsById[desc.Id] = desc;

            if (desc.IsPlayer)
            {
                PlayerClasses.RemoveAll(existing => existing.Type == desc.Type);
                PlayerClasses.Add(desc);
            }
        }
    }

    /// <summary>Merges one ground XML document.</summary>
    public void AddGround(string xml)
    {
        foreach (var element in Root(xml).Elements("Ground"))
        {
            var desc = ParseGround(element);
            if (desc == null)
                continue;

            _groundByType[desc.Type] = desc;
            if (desc.Id != null)
                _groundById[desc.Id] = desc;
        }
    }

    private static XElement Root(string xml)
    {
        // Several of these files declare ISO-8859-1 and then contain bytes that are not valid in it,
        // so they are handed to us as text and parsed leniently.
        return XDocument.Parse(xml, LoadOptions.None).Root
               ?? throw new InvalidOperationException("XML document has no root element.");
    }

    // ------------------------------------------------------------------------------------------
    // Objects
    // ------------------------------------------------------------------------------------------

    private static ObjectDesc ParseObject(XElement e)
    {
        if (!TryParseInt(e.Attribute("type")?.Value, out int type))
            return null;

        var desc = new ObjectDesc
        {
            Type = (ushort)type,
            Id = e.Attribute("id")?.Value,
            DisplayId = Text(e, "DisplayId"),
            Class = Text(e, "Class"),
            Group = Text(e, "Group"),
            DungeonName = Text(e, "DungeonName"),

            IsPlayer = Has(e, "Player"),
            StatMaxima = Has(e, "Player") ? ParseStatMaxima(e) : null,
            IsEnemy = Has(e, "Enemy"),
            IsHero = Has(e, "Hero"),
            IsEncounter = Has(e, "Encounter"),
            IsGod = Has(e, "God"),
            DrawOnGround = Has(e, "DrawOnGround"),
            DrawUnder = Has(e, "DrawUnder"),
            OccupySquare = Has(e, "OccupySquare"),
            FullOccupy = Has(e, "FullOccupy"),
            EnemyOccupySquare = Has(e, "EnemyOccupySquare"),
            Static = Has(e, "Static"),
            NoMiniMap = Has(e, "NoMiniMap"),
            ProtectFromGroundDamage = Has(e, "ProtectFromGroundDamage"),
            ProtectFromSink = Has(e, "ProtectFromSink"),
            Flying = Has(e, "Flying"),
            ShowName = Has(e, "ShowName"),
            DontFaceAttacks = Has(e, "DontFaceAttacks"),
            BlocksSight = Has(e, "BlocksSight"),
            Connects = Has(e, "Connects"),
            IsIntergamePortal = Has(e, "IntergamePortal"),
            StunImmune = Has(e, "StunImmune"),
            ParalyzeImmune = Has(e, "ParalyzeImmune"),
            DazedImmune = Has(e, "DazedImmune"),

            ShadowSize = Int(e, "ShadowSize", 100),
            ShadowColor = Int(e, "ShadowColor", 0),
            Z = Float(e, "Z", 0f),
            Color = Int(e, "Color", 0xFFFFFF),
            BloodProb = Float(e, "BloodProb", 0f),
            BloodColor = Int(e, "BloodColor", 0xFF0000),
            MaxHitPoints = Int(e, "MaxHitPoints", 200),
            Defense = Int(e, "Defense", 0),
            Tex1 = Int(e, "Tex1", 0),
            Tex2 = Int(e, "Tex2", 0),
            Model = Text(e, "Model"),
            HitSound = Text(e, "HitSound") ?? "monster/default_hit",
            DeathSound = Text(e, "DeathSound") ?? "monster/default_death",
            // The XML stores this as eighths of a turn.
            AngleCorrection = Float(e, "AngleCorrection", 0f) * (MathF.PI / 4f),
            // Left raw, because two different things read it. A model object takes it as a yaw in
            // degrees; a projectile divides elapsed milliseconds by it to get a spin in radians, so
            // it is a period there. Converting here would suit one and corrupt the other.
            Rotation = Float(e, "Rotation", 0f),

            SlotType = Int(e, "SlotType", -1),
            Tier = Int(e, "Tier", -1),
            Description = Text(e, "Description"),
            FeedPower = Int(e, "feedPower", 0),
            EquipBonuses = ParseEquipBonuses(e),
            RateOfFire = Float(e, "RateOfFire", 1f),
            NumProjectiles = Int(e, "NumProjectiles", 1),
            MpCost = Int(e, "MpCost", 0),
            MpEndCost = Int(e, "MpEndCost", 0),
            Consumable = Has(e, "Consumable"),
            Usable = Has(e, "Usable"),
            Soulbound = Has(e, "Soulbound"),
            // Declared in seconds; everything downstream works in milliseconds.
            CooldownMs = (int)(Float(e, "Cooldown", 0f) * 1000f),
            MultiPhase = Has(e, "MultiPhase"),
        };

        // The verb is the element's text and the size of what it does is an attribute on it, so
        // both halves are kept: "Heal" alone says a potion heals, "Heal amount=230" says by how
        // much, and only the pair distinguishes Fire Water from a Minor Health Potion.
        desc.Activates = e.Elements("Activate")
            .Select(a => new ActivateDesc(
                a.Value?.Trim(),
                TryParseInt(a.Attribute("amount")?.Value, out int amount) ? amount : 0))
            .ToArray();
        desc.ActivatesShoot = desc.Activates.Any(a => a.Is("Shoot"));

        // The XML gives this in degrees. The default of 11.25 is what produces the familiar even
        // fan on a three-shot weapon.
        desc.ArcGap = Float(e, "ArcGap", 11.25f) * (MathF.PI / 180f);

        // A single Size collapses the range; otherwise the three fields describe it.
        if (e.Element("Size") != null)
        {
            desc.MinSize = desc.MaxSize = Int(e, "Size", 100);
        }
        else
        {
            desc.MinSize = Int(e, "MinSize", 100);
            desc.MaxSize = Int(e, "MaxSize", 100);
            desc.SizeStep = Int(e, "SizeStep", 5);
        }

        // Drawing an object flat on the tile implies it renders beneath everything else on it.
        if (desc.DrawOnGround)
            desc.DrawUnder = true;

        string slotTypes = Text(e, "SlotTypes");
        if (slotTypes != null)
            desc.SlotTypes = SplitInts(slotTypes);

        var whileMoving = e.Element("WhileMoving");
        if (whileMoving != null)
        {
            desc.WhileMoving = new WhileMovingDesc
            {
                Z = Float(whileMoving, "Z", 0f),
                Flying = Has(whileMoving, "Flying"),
            };
        }

        desc.Texture = ParseTexture(e);
        if (e.Element("Top") != null)
            desc.TopTexture = ParseTexture(e.Element("Top"));
        if (e.Element("Portrait") != null)
            desc.Portrait = ParseTexture(e.Element("Portrait"));

        foreach (var projectile in e.Elements("Projectile"))
        {
            desc.Projectiles ??= new Dictionary<int, ProjectileDesc>();
            var parsed = ParseProjectile(projectile);
            desc.Projectiles[parsed.BulletType] = parsed;
        }

        foreach (var sound in e.Elements("Sound"))
        {
            // The id is optional and almost always absent: 915 of the 922 Sound elements in this
            // data are a bare <Sound>, which is a weapon's firing sound and is sound zero. Only the
            // handful that carry several number them, for the PlaySound packet to pick between.
            // Requiring the attribute threw away every weapon sound in the game.
            if (!TryParseInt(sound.Attribute("id")?.Value, out int soundId))
                soundId = 0;

            string name = sound.Value.Trim();
            if (name.Length == 0)
                continue;

            desc.Sounds ??= new Dictionary<int, string>();
            desc.Sounds[soundId] = name;
        }

        return desc;
    }

    private static ProjectileDesc ParseProjectile(XElement e)
    {
        var desc = new ProjectileDesc
        {
            BulletType = TryParseInt(e.Attribute("id")?.Value, out int id) ? id : 0,
            ObjectId = Text(e, "ObjectId"),
            LifetimeMs = Int(e, "LifetimeMS", 0),
            Speed = Float(e, "Speed", 0f),
            Size = Int(e, "Size", -1),
            MultiHit = Has(e, "MultiHit"),
            PassesCover = Has(e, "PassesCover"),
            ArmorPiercing = Has(e, "ArmorPiercing"),
            Wavy = Has(e, "Wavy"),
            Parametric = Has(e, "Parametric"),
            Boomerang = Has(e, "Boomerang"),
            FaceDir = Has(e, "FaceDir"),
            Amplitude = Float(e, "Amplitude", 0f),
            Frequency = Float(e, "Frequency", 1f),
            Magnitude = Float(e, "Magnitude", 3f),
        };

        if (e.Element("Damage") != null)
        {
            desc.MinDamage = desc.MaxDamage = Int(e, "Damage", 0);
        }
        else
        {
            desc.MinDamage = Int(e, "MinDamage", 0);
            desc.MaxDamage = Int(e, "MaxDamage", 0);
        }

        var trail = e.Element("ParticleTrail");
        if (trail != null)
        {
            desc.ParticleTrail = true;
            desc.ParticleTrailColor = TryParseInt(trail.Value.Trim(), out int color) ? color : 0xFF00FF;
            desc.ParticleTrailLifetimeMs =
                TryParseInt(trail.Attribute("lifetimeMS")?.Value, out int life) ? life : 600;
        }

        foreach (var effect in e.Elements("ConditionEffect"))
        {
            if (!TryParseConditionEffect(effect.Value, out var index))
                continue;

            desc.Effects ??= new List<ConditionEffectIndex>();
            desc.Effects.Add(index);

            // target="1" marks an effect a pet applies rather than the shooter, which the damage
            // path has to skip when the projectile is not the pet's.
            if (effect.Attribute("target")?.Value == "1")
            {
                desc.PetEffects ??= new HashSet<ConditionEffectIndex>();
                desc.PetEffects.Add(index);
            }
        }

        return desc;
    }

    // ------------------------------------------------------------------------------------------
    // Ground
    // ------------------------------------------------------------------------------------------

    private static GroundDesc ParseGround(XElement e)
    {
        if (!TryParseInt(e.Attribute("type")?.Value, out int type))
            return null;

        var desc = new GroundDesc
        {
            Type = (ushort)type,
            Id = e.Attribute("id")?.Value,
            NoWalk = Has(e, "NoWalk"),
            MinDamage = Int(e, "MinDamage", 0),
            MaxDamage = Int(e, "MaxDamage", 0),
            Push = Has(e, "Push"),
            BlendPriority = Int(e, "BlendPriority", -1),
            CompositePriority = Int(e, "CompositePriority", 0),
            Speed = Float(e, "Speed", 1f),
            SlideAmount = Float(e, "SlideAmount", 0f),
            XOffset = Float(e, "XOffset", 0f),
            YOffset = Float(e, "YOffset", 0f),
            Sink = Has(e, "Sink"),
            Sinking = Has(e, "Sinking"),
            RandomOffset = Has(e, "RandomOffset"),
            SameTypeEdgeMode = Has(e, "SameTypeEdgeMode"),
            Color = Int(e, "Color", -1),
            Texture = ParseTexture(e),
        };

        var animate = e.Element("Animate");
        if (animate != null)
        {
            desc.Animation = ParseAnimation(animate.Value);
            desc.AnimationDx = FloatAttr(animate, "dx");
            desc.AnimationDy = FloatAttr(animate, "dy");
        }

        if (e.Element("Edge") != null)
            desc.EdgeTexture = ParseTexture(e.Element("Edge"));
        if (e.Element("Corner") != null)
            desc.CornerTexture = ParseTexture(e.Element("Corner"));
        if (e.Element("InnerCorner") != null)
            desc.InnerCornerTexture = ParseTexture(e.Element("InnerCorner"));
        if (e.Element("Top") != null)
            desc.TopTexture = ParseTexture(e.Element("Top"));

        var topAnimate = e.Element("TopAnimate");
        if (topAnimate != null)
        {
            desc.TopAnimation = ParseAnimation(topAnimate.Value);
            desc.TopAnimationDx = FloatAttr(topAnimate, "dx");
            desc.TopAnimationDy = FloatAttr(topAnimate, "dy");
        }

        return desc;
    }

    private static GroundAnimation ParseAnimation(string value) => value?.Trim() switch
    {
        "Wave" => GroundAnimation.Wave,
        "Flow" => GroundAnimation.Flow,
        _ => GroundAnimation.None,
    };

    // ------------------------------------------------------------------------------------------
    // Textures
    // ------------------------------------------------------------------------------------------

    /// <summary>
    /// Reads whichever texture form <paramref name="owner"/> declares. The forms are mutually
    /// exclusive and checked in the same order the original used, because a few objects declare
    /// more than one and the first wins.
    /// </summary>
    private static TextureSpec ParseTexture(XElement owner)
    {
        TextureSpec spec = null;

        if (owner.Element("Texture") != null)
            spec = FileIndex(owner.Element("Texture"), TextureKind.Sprite);
        else if (owner.Element("AnimatedTexture") != null)
            spec = FileIndex(owner.Element("AnimatedTexture"), TextureKind.AnimatedChar);
        else if (owner.Element("RemoteTexture") != null)
            spec = ParseRemote(owner.Element("RemoteTexture"));
        else if (owner.Element("RandomTexture") != null)
            spec = ParseRandom(owner.Element("RandomTexture"));
        else if (owner.Element("File") != null)
            spec = FileIndex(owner, TextureKind.Sprite);

        if (spec == null)
            return null;

        foreach (var alt in owner.Elements("AltTexture"))
        {
            if (!TryParseInt(alt.Attribute("id")?.Value, out int altId))
                continue;
            spec.Alternates ??= new Dictionary<int, TextureSpec>();
            var altSpec = ParseTexture(alt);
            if (altSpec != null)
                spec.Alternates[altId] = altSpec;
        }

        if (owner.Element("Mask") != null)
            spec.Mask = FileIndex(owner.Element("Mask"), TextureKind.Sprite);

        var effect = owner.Element("Effect");
        if (effect != null)
            spec.EffectId = effect.Attribute("id")?.Value ?? effect.Value.Trim();

        return spec;
    }

    private static TextureSpec FileIndex(XElement e, TextureKind kind) => new()
    {
        Kind = kind,
        File = Text(e, "File"),
        Index = Int(e, "Index", 0),
    };

    private static TextureSpec ParseRemote(XElement e) => new()
    {
        Kind = TextureKind.Remote,
        // The id is a "source:name" pair; the client only ever needs the whole string as a key.
        RemoteId = Text(e, "Id"),
        RemoteFacesRight = e.Element("Right") != null,
    };

    private static TextureSpec ParseRandom(XElement e)
    {
        var spec = new TextureSpec { Kind = TextureKind.Random, Variants = new List<TextureSpec>() };
        foreach (var child in e.Elements())
        {
            var variant = child.Name.LocalName switch
            {
                "Texture" => FileIndex(child, TextureKind.Sprite),
                "AnimatedTexture" => FileIndex(child, TextureKind.AnimatedChar),
                _ => null,
            };
            if (variant != null)
                spec.Variants.Add(variant);
        }
        return spec;
    }

    // ------------------------------------------------------------------------------------------
    // Primitives
    // ------------------------------------------------------------------------------------------

    /// <summary>
    /// The eight stat ceilings for a player class, in the order the interface shows them.
    /// </summary>
    /// <remarks>
    /// Each stat is an element carrying the starting value with the ceiling on a max attribute:
    /// <c>&lt;Attack max="75"&gt;12&lt;/Attack&gt;</c>. Two are named after their effect rather than
    /// their name on screen -- HpRegen is Vitality, MpRegen is Wisdom.
    /// </remarks>
    private static int[] ParseStatMaxima(XElement e)
    {
        string[] names = { "MaxHitPoints", "MaxMagicPoints", "Attack", "Defense", "Speed", "Dexterity", "HpRegen", "MpRegen" };
        var maxima = new int[names.Length];

        for (int i = 0; i < names.Length; i++)
        {
            var attribute = e.Element(names[i])?.Attribute("max");
            maxima[i] = attribute != null && int.TryParse(attribute.Value, out int value) ? value : 0;
        }

        return maxima;
    }

    /// <summary>
    /// What equipping an item adds to each stat.
    /// </summary>
    /// <remarks>
    /// The stat attribute uses the old client's numbering -- 20 Attack, 21 Defense, 22 Speed,
    /// 26 Vitality, 27 Wisdom, 28 Dexterity -- which is not the wire enum's, where those same
    /// numbers are inventory slots. Translating here means everything downstream can speak one
    /// language. MaxHP and MaxMP agree in both and pass through.
    /// </remarks>
    private static (int Stat, int Amount)[] ParseEquipBonuses(XElement e)
    {
        var bonuses = new List<(int, int)>();

        foreach (var element in e.Elements("ActivateOnEquip"))
        {
            if (element.Value?.Trim() != "IncrementStat")
                continue;

            if (!int.TryParse(element.Attribute("stat")?.Value, out int stat) ||
                !int.TryParse(element.Attribute("amount")?.Value, out int amount))
                continue;

            bonuses.Add((XmlStat(stat), amount));
        }

        return bonuses.Count == 0 ? System.Array.Empty<(int, int)>() : bonuses.ToArray();
    }

    private static int XmlStat(int stat) => stat switch
    {
        20 => (int)StatsType.Attack,
        21 => (int)StatsType.Defense,
        22 => (int)StatsType.Speed,
        26 => (int)StatsType.Vitality,
        27 => (int)StatsType.Wisdom,
        28 => (int)StatsType.Dexterity,
        _ => stat,
    };

    private static bool Has(XElement e, string name) => e.Element(name) != null;

    private static string Text(XElement e, string name)
    {
        string value = e.Element(name)?.Value;
        return string.IsNullOrWhiteSpace(value) ? null : value.Trim();
    }

    private static int Int(XElement e, string name, int fallback) =>
        TryParseInt(e.Element(name)?.Value, out int value) ? value : fallback;

    private static float Float(XElement e, string name, float fallback) =>
        TryParseFloat(e.Element(name)?.Value, out float value) ? value : fallback;

    private static float FloatAttr(XElement e, string name) =>
        TryParseFloat(e.Attribute(name)?.Value, out float value) ? value : 0f;

    /// <summary>Parses decimal or <c>0x</c>-prefixed hex, both of which appear throughout the data.</summary>
    internal static bool TryParseInt(string text, out int value)
    {
        value = 0;
        if (string.IsNullOrWhiteSpace(text))
            return false;

        text = text.Trim();
        bool negative = text.StartsWith('-');
        if (negative)
            text = text[1..];

        bool parsed = text.StartsWith("0x", StringComparison.OrdinalIgnoreCase)
            ? int.TryParse(text[2..], NumberStyles.HexNumber, CultureInfo.InvariantCulture, out value)
            : int.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out value);

        if (parsed && negative)
            value = -value;
        return parsed;
    }

    /// <summary>
    /// Parses a float with invariant culture. The data uses forms like <c>.5</c>, and a
    /// culture-sensitive parse would read those as zero on a machine that uses a comma separator.
    /// </summary>
    internal static bool TryParseFloat(string text, out float value) =>
        float.TryParse(text?.Trim(), NumberStyles.Float, CultureInfo.InvariantCulture, out value);

    private static int[] SplitInts(string text) =>
        text.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .Select(part => TryParseInt(part, out int value) ? value : 0)
            .ToArray();

    private static bool TryParseConditionEffect(string name, out ConditionEffectIndex index)
    {
        // The XML spells these with spaces ("Armor Broken"); the enum does not.
        string cleaned = name?.Replace(" ", string.Empty).Trim();
        return Enum.TryParse(cleaned, ignoreCase: true, out index);
    }
}

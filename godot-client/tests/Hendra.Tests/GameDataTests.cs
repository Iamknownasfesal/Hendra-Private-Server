using System;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using Hendra.Data;
using Hendra.Resources;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// Parses the real extracted game data rather than a fixture.
///
/// Hand-written XML samples would only prove the parser handles the shapes I thought to write down.
/// The actual data is 82 files accumulated over years, full of hex attributes, decimal values
/// written as <c>.5</c>, entries that declare two texture forms at once, and a Latin-1 declaration
/// the files do not always honour. Running the parser over all of it is the only way to know it
/// copes.
/// </summary>
public sealed class GameDataTests : IDisposable
{
    private static readonly string AssetRoot =
        Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "../../../../../assets"));

    private static readonly string XmlRoot = Path.Combine(AssetRoot, "xml");

    private readonly GameData _data = new();
    private readonly bool _available;

    public GameDataTests()
    {
        _available = Directory.Exists(XmlRoot) && Directory.EnumerateFiles(XmlRoot, "*.xml").Any();
        if (!_available)
            return;

        var manifest = JsonDocument.Parse(File.ReadAllText(Path.Combine(AssetRoot, "manifest.json")));
        var xml = manifest.RootElement.GetProperty("xml");

        foreach (var name in xml.GetProperty("ground").EnumerateArray())
            _data.AddGround(ReadXml(name.GetString()));

        foreach (var name in xml.GetProperty("objects").EnumerateArray())
            _data.AddObjects(ReadXml(name.GetString()));
    }

    public void Dispose() { }

    /// <summary>
    /// The files declare ISO-8859-1, so they are read as Latin-1. Reading them as UTF-8 turns the
    /// stray high bytes a few of them contain into replacement characters.
    /// </summary>
    private static string ReadXml(string fileName) =>
        File.ReadAllText(Path.Combine(XmlRoot, fileName), Encoding.Latin1);

    private void RequireAssets()
    {
        Assert.True(_available,
            $"No extracted game data at {XmlRoot}. Run tools/extract_assets.py first.");
    }

    [Fact]
    public void EveryShippedXmlFileParses()
    {
        RequireAssets();

        // The constructor already parsed the ground and object lists; reaching here without an
        // exception is the assertion. The counts guard against a silent no-op.
        Assert.True(_data.Objects.Count > 1000, $"Only parsed {_data.Objects.Count} objects.");
        Assert.True(_data.Ground.Count > 300, $"Only parsed {_data.Ground.Count} ground types.");
    }

    [Fact]
    public void KnownObjectsResolveWithTheExpectedFields()
    {
        RequireAssets();

        var flayer = _data.GetObject("Flayer");
        Assert.NotNull(flayer);
        Assert.Equal(0x647, flayer.Type);
        Assert.True(flayer.IsEnemy);
        Assert.Equal("Character", flayer.Class);
        Assert.Equal(140, flayer.MaxHitPoints);
        Assert.Equal(8, flayer.Defense);

        // Its artwork is an animated character sheet, not a flat sprite.
        Assert.NotNull(flayer.Texture);
        Assert.Equal(TextureKind.AnimatedChar, flayer.Texture.Kind);
        Assert.Equal("chars8x8rHigh", flayer.Texture.File);
        Assert.Equal(0x0b, flayer.Texture.Index);

        // A single <Damage> collapses the min/max range.
        var projectile = flayer.Projectiles[0];
        Assert.Equal(40, projectile.MinDamage);
        Assert.Equal(40, projectile.MaxDamage);
        Assert.Equal(2700, projectile.LifetimeMs);
        Assert.Equal(50f, projectile.Speed);
    }

    [Fact]
    public void GroundParsesAnimationAndMovementFields()
    {
        RequireAssets();

        var water = _data.GetGround("Black Water");
        Assert.NotNull(water);
        Assert.Equal(0, water.Type);
        Assert.True(water.NoWalk);
        Assert.Equal(GroundAnimation.Wave, water.Animation);

        // Written as ".5" in the XML, which a culture-sensitive parse would read as zero.
        Assert.Equal(0.5f, water.AnimationDx);
        Assert.Equal(0.5f, water.AnimationDy);

        var lightWater = _data.GetGround("Black Water Light");
        Assert.NotNull(lightWater);
        Assert.True(lightWater.Sink);
        Assert.Equal(2, lightWater.BlendPriority);
        Assert.Equal(0.7f, lightWater.Speed, 3);
    }

    [Fact]
    public void PlayerClassesAreCollected()
    {
        RequireAssets();

        Assert.NotEmpty(_data.PlayerClasses);
        Assert.All(_data.PlayerClasses, playerClass =>
        {
            Assert.True(playerClass.IsPlayer);
            Assert.NotNull(playerClass.SlotTypes);
            Assert.NotEmpty(playerClass.SlotTypes);
        });

        // The Wizard is the default selection on the character-creation screen.
        Assert.Contains(_data.PlayerClasses, playerClass => playerClass.Type == 782);
    }

    [Fact]
    public void DrawOnGroundImpliesDrawUnder()
    {
        RequireAssets();

        // The original derived one from the other rather than requiring both in the XML; anything
        // painted flat on the tile has to sort beneath what stands on it.
        var flat = _data.Objects.Values.Where(o => o.DrawOnGround).ToList();
        Assert.NotEmpty(flat);
        Assert.All(flat, o => Assert.True(o.DrawUnder));
    }

    [Fact]
    public void ConditionEffectNamesWithSpacesResolve()
    {
        RequireAssets();

        // The XML spells these with spaces ("Armor Broken"), the enum does not. A failure to match
        // would silently drop the effect rather than erroring.
        var withEffects = _data.Objects.Values
            .Where(o => o.Projectiles != null)
            .SelectMany(o => o.Projectiles.Values)
            .Where(p => p.Effects is { Count: > 0 })
            .ToList();

        Assert.NotEmpty(withEffects);
        Assert.Contains(withEffects, p => p.Effects.Contains(ConditionEffectIndex.Slowed));
        Assert.Contains(withEffects, p => p.Effects.Contains(ConditionEffectIndex.ArmorBroken));
    }

    [Fact]
    public void HexAndDecimalAttributesBothParse()
    {
        Assert.True(GameData.TryParseInt("0x647", out int hex));
        Assert.Equal(1607, hex);

        Assert.True(GameData.TryParseInt("140", out int dec));
        Assert.Equal(140, dec);

        Assert.True(GameData.TryParseInt("-1", out int negative));
        Assert.Equal(-1, negative);

        Assert.False(GameData.TryParseInt("", out _));
        Assert.False(GameData.TryParseInt(null, out _));
    }

    [Fact]
    public void FloatsParseInvariantOfCulture()
    {
        // Guards against a comma-separator locale reading ".5" as zero.
        var previous = System.Globalization.CultureInfo.CurrentCulture;
        try
        {
            System.Globalization.CultureInfo.CurrentCulture =
                new System.Globalization.CultureInfo("de-DE");

            Assert.True(GameData.TryParseFloat(".5", out float half));
            Assert.Equal(0.5f, half);

            Assert.True(GameData.TryParseFloat("0.7", out float value));
            Assert.Equal(0.7f, value, 4);
        }
        finally
        {
            System.Globalization.CultureInfo.CurrentCulture = previous;
        }
    }

    [Fact]
    public void LaterFilesOverrideEarlierOnesForTheSameType()
    {
        // Dungeon overlays reuse base type numbers to replace their definitions, so merge order is
        // part of the contract rather than an accident.
        var data = new GameData();
        data.AddObjects("<Objects><Object type=\"0x01\" id=\"First\"><MaxHitPoints>10</MaxHitPoints></Object></Objects>");
        Assert.Equal(10, data.GetObject(1).MaxHitPoints);

        data.AddObjects("<Objects><Object type=\"0x01\" id=\"Second\"><MaxHitPoints>99</MaxHitPoints></Object></Objects>");
        Assert.Equal(99, data.GetObject(1).MaxHitPoints);
        Assert.Equal("Second", data.GetObject(1).Id);
    }
}

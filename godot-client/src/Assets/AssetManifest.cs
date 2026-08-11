using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;
using Godot;

namespace Hendra.Assets;

/// <summary>
/// The index written by <c>tools/extract_assets.py</c>: which file backs each logical spritesheet,
/// how it is sliced, and in what order the game XML has to be parsed.
/// </summary>
/// <remarks>
/// The AS3 client encoded all of this as ~95 hand-written <c>addImageSet</c> / <c>AnimatedChars.add</c>
/// calls in <c>AssetLoader.as</c>. Keeping it as generated data instead means the slicing geometry
/// stays derived from that same source of truth rather than being transcribed by hand and left to
/// drift.
/// </remarks>
public sealed class AssetManifest
{
    public const string ManifestPath = "res://assets/manifest.json";
    public const string SheetDirectory = "res://assets/sheets/";
    public const string ModelDirectory = "res://assets/models/";
    public const string XmlDirectory = "res://assets/xml/";

    private static readonly JsonSerializerOptions Options = new()
    {
        PropertyNameCaseInsensitive = true,
        ReadCommentHandling = JsonCommentHandling.Skip,
    };

    /// <summary>Spritesheets sliced into a uniform grid, keyed by the name the XML refers to.</summary>
    [JsonPropertyName("sheets")]
    public Dictionary<string, SheetInfo> Sheets { get; set; } = new();

    /// <summary>Character sheets, keyed by the name the XML refers to.</summary>
    [JsonPropertyName("animated_chars")]
    public Dictionary<string, AnimatedCharInfo> AnimatedChars { get; set; } = new();

    /// <summary>Whole images used as-is rather than sliced.</summary>
    [JsonPropertyName("images")]
    public Dictionary<string, string> Images { get; set; } = new();

    /// <summary>In-XML model name to the .obj file that backs it.</summary>
    [JsonPropertyName("models")]
    public Dictionary<string, string> Models { get; set; } = new();

    /// <summary>The particle definition XML.</summary>
    [JsonPropertyName("particles")]
    public string Particles { get; set; }

    [JsonPropertyName("xml")]
    public XmlLists Xml { get; set; } = new();

    public sealed class SheetInfo
    {
        [JsonPropertyName("file")] public string File { get; set; }
        [JsonPropertyName("tile_w")] public int TileWidth { get; set; }
        [JsonPropertyName("tile_h")] public int TileHeight { get; set; }

        /// <summary>True for the generated transparent "invisible" sheet.</summary>
        [JsonPropertyName("synthetic")] public bool Synthetic { get; set; }
    }

    public sealed class AnimatedCharInfo
    {
        [JsonPropertyName("file")] public string File { get; set; }

        /// <summary>The recolour mask, or null when the sheet has none.</summary>
        [JsonPropertyName("mask")] public string Mask { get; set; }

        /// <summary>Size of one animation frame.</summary>
        [JsonPropertyName("frame_w")] public int FrameWidth { get; set; }

        [JsonPropertyName("frame_h")] public int FrameHeight { get; set; }

        /// <summary>Size of one character's block of frames within the sheet.</summary>
        [JsonPropertyName("strip_w")] public int StripWidth { get; set; }

        [JsonPropertyName("strip_h")] public int StripHeight { get; set; }

        /// <summary>Which facing the first row holds: 0 right, 1 left, 2 down, 3 up.</summary>
        [JsonPropertyName("first_dir")] public int FirstDirection { get; set; }
    }

    public sealed class XmlLists
    {
        [JsonPropertyName("ground")] public List<string> Ground { get; set; } = new();

        /// <summary>
        /// Object definitions, in load order. The first <see cref="BaseObjectCount"/> entries are
        /// base data; everything after is a per-dungeon overlay, which the original parsed through
        /// a different path.
        /// </summary>
        [JsonPropertyName("objects")] public List<string> Objects { get; set; } = new();

        [JsonPropertyName("regions")] public List<string> Regions { get; set; } = new();
        [JsonPropertyName("spawnRegions")] public List<string> SpawnRegions { get; set; } = new();
        [JsonPropertyName("baseObjectCount")] public int BaseObjectCount { get; set; }
    }

    /// <summary>Reads the manifest, or throws if it is missing or malformed.</summary>
    public static AssetManifest Load()
    {
        using var file = FileAccess.Open(ManifestPath, FileAccess.ModeFlags.Read);
        if (file == null)
        {
            throw new System.IO.FileNotFoundException(
                $"{ManifestPath} is missing. Run tools/extract_assets.py to generate the assets.");
        }

        var manifest = JsonSerializer.Deserialize<AssetManifest>(file.GetAsText(), Options);
        if (manifest == null || manifest.Sheets.Count == 0)
            throw new System.IO.InvalidDataException($"{ManifestPath} parsed but contained no sheets.");

        return manifest;
    }

    public string SheetPath(string fileName) => SheetDirectory + fileName;
    public string ModelPath(string fileName) => ModelDirectory + fileName;
    public string XmlPath(string fileName) => XmlDirectory + fileName;
}

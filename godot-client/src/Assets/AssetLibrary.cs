using System;
using System.Collections.Generic;
using Godot;

namespace Hendra.Assets;

/// <summary>
/// Every sprite in the game, addressed the way the XML addresses it: a sheet name and an index.
/// </summary>
/// <remarks>
/// The AS3 equivalent was a set of static <c>Dictionary</c> fields on <c>AssetLibrary</c>, populated
/// by a hundred hand-written registration calls and reachable from anywhere. This is the same
/// lookup table, built from the generated manifest and passed explicitly to whoever needs it.
/// </remarks>
public sealed class AssetLibrary
{
    private readonly Dictionary<string, SpriteSheet> _sheets = new();
    private readonly Dictionary<string, AnimatedChar[]> _animatedChars = new();
    private readonly Dictionary<string, Texture2D> _images = new();

    public AssetManifest Manifest { get; private set; }

    /// <summary>Loads every sheet named by the manifest.</summary>
    public static AssetLibrary Load()
    {
        var manifest = AssetManifest.Load();
        var library = new AssetLibrary { Manifest = manifest };

        foreach (var (name, info) in manifest.Sheets)
        {
            var texture = LoadTexture(manifest.SheetPath(info.File));
            if (texture != null)
                library._sheets[name] = new SpriteSheet(name, texture, info.TileWidth, info.TileHeight);
        }

        foreach (var (name, info) in manifest.AnimatedChars)
            library.LoadAnimatedCharSet(manifest, name, info);

        foreach (var (name, file) in manifest.Images)
        {
            var texture = LoadTexture(manifest.SheetPath(file));
            if (texture != null)
                library._images[name] = texture;
        }

        return library;
    }

    private static Texture2D LoadTexture(string path)
    {
        var texture = ResourceLoader.Load<Texture2D>(path);
        if (texture == null)
            GD.PushError($"[assets] missing texture {path}; run tools/extract_assets.py.");
        return texture;
    }

    private void LoadAnimatedCharSet(AssetManifest manifest, string name, AssetManifest.AnimatedCharInfo info)
    {
        var texture = LoadTexture(manifest.SheetPath(info.File));
        if (texture == null)
            return;

        Texture2D maskTexture = info.Mask == null ? null : LoadTexture(manifest.SheetPath(info.Mask));

        var size = texture.GetSize();
        int stripsPerRow = Mathf.Max(1, (int)size.X / info.StripWidth);
        int stripRows = Mathf.Max(1, (int)size.Y / info.StripHeight);
        int cellsPerStripRow = Mathf.Max(1, info.StripWidth / info.FrameWidth);
        int cellRowsPerStrip = Mathf.Max(1, info.StripHeight / info.FrameHeight);
        int cellsPerStrip = cellsPerStripRow * cellRowsPerStrip;

        // A fully transparent cell is the original's way of saying "this pose does not exist", so
        // it has to be detected from the pixels. Done once here rather than per frame.
        var image = texture.GetImage();
        var facing = (CharFacing)info.FirstDirection;

        var characters = new AnimatedChar[stripsPerRow * stripRows];
        for (int strip = 0; strip < characters.Length; strip++)
        {
            int originX = strip % stripsPerRow * info.StripWidth;
            int originY = strip / stripsPerRow * info.StripHeight;

            Rect2I CellRect(int cell, int spanCells) => new(
                originX + cell % cellsPerStripRow * info.FrameWidth,
                originY + cell / cellsPerStripRow * info.FrameHeight,
                info.FrameWidth * spanCells,
                info.FrameHeight);

            Sprite Cell(int cell, int spanCells) =>
                cell < 0 || cell >= cellsPerStrip ? default : new Sprite(texture, CellRect(cell, spanCells));

            Func<int, int, Sprite> maskCell = maskTexture == null
                ? null
                : (cell, spanCells) =>
                    cell < 0 || cell >= cellsPerStrip ? default : new Sprite(maskTexture, CellRect(cell, spanCells));

            characters[strip] = AnimatedChar.Build(
                Cell,
                maskCell,
                cellsPerStrip,
                cell => cell < 0 || cell >= cellsPerStrip || IsFullyTransparent(image, CellRect(cell, 1)),
                facing);
        }

        _animatedChars[name] = characters;
    }

    private static bool IsFullyTransparent(Image image, Rect2I region)
    {
        int right = Mathf.Min(region.Position.X + region.Size.X, image.GetWidth());
        int bottom = Mathf.Min(region.Position.Y + region.Size.Y, image.GetHeight());

        for (int y = region.Position.Y; y < bottom; y++)
        {
            for (int x = region.Position.X; x < right; x++)
            {
                if (image.GetPixel(x, y).A > 0f)
                    return false;
            }
        }
        return true;
    }

    /// <summary>
    /// The sprite at <paramref name="index"/> of <paramref name="setName"/>. Returns an invalid
    /// sprite for an unknown set or an out-of-range index; the game XML contains both.
    /// </summary>
    public Sprite GetSprite(string setName, int index) =>
        setName != null && _sheets.TryGetValue(setName, out var sheet) ? sheet[index] : default;

    public SpriteSheet GetSheet(string setName) =>
        setName != null && _sheets.TryGetValue(setName, out var sheet) ? sheet : null;

    /// <summary>The character animation at <paramref name="index"/> of <paramref name="setName"/>.</summary>
    public AnimatedChar GetAnimatedChar(string setName, int index)
    {
        if (setName == null || !_animatedChars.TryGetValue(setName, out var set))
            return null;
        return index >= 0 && index < set.Length ? set[index] : null;
    }

    /// <summary>A whole image used as-is, such as a menu background.</summary>
    public Texture2D GetImage(string name) =>
        name != null && _images.TryGetValue(name, out var texture) ? texture : null;

    public IReadOnlyDictionary<string, SpriteSheet> Sheets => _sheets;
}

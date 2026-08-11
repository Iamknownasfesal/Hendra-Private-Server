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

    /// <summary>
    /// Registers a character strip fetched from the app server.
    /// </summary>
    /// <remarks>
    /// <para>
    /// A remote texture is a single row of seven frames — one direction's worth — which the game
    /// data refers to by a numeric id rather than a sheet and index. Its width is therefore seven
    /// frames and its height one, and <see cref="AnimatedChar.Build"/> fills the other directions in
    /// by mirroring and falling back.
    /// </para>
    /// <para>
    /// The reference client has this path commented out and falls back to a placeholder sprite for
    /// all of them, so anything using one shows as the same little box. Fetching them is strictly
    /// better and costs one request at startup.
    /// </para>
    /// </remarks>
    public void AddRemoteTexture(string id, Texture2D texture, Texture2D mask, bool facesRight)
    {
        if (string.IsNullOrEmpty(id) || texture == null)
            return;

        var size = texture.GetSize();
        int frameWidth = Mathf.Max(1, (int)size.X / AnimatedChar.CellsPerRow);
        int frameHeight = Mathf.Max(1, (int)size.Y);
        var image = texture.GetImage();

        Rect2I CellRect(int cell, int spanCells) =>
            new(cell * frameWidth, 0, frameWidth * spanCells, frameHeight);

        Sprite Cell(int cell, int spanCells) =>
            cell < 0 || cell >= AnimatedChar.CellsPerRow ? default : new Sprite(texture, CellRect(cell, spanCells));

        Func<int, int, Sprite> maskCell = mask == null
            ? null
            : (cell, spanCells) =>
                cell < 0 || cell >= AnimatedChar.CellsPerRow ? default : new Sprite(mask, CellRect(cell, spanCells));

        _animatedChars[RemoteKey(id)] = new[]
        {
            AnimatedChar.Build(
                Cell,
                maskCell,
                AnimatedChar.CellsPerRow,
                cell => cell < 0 || cell >= AnimatedChar.CellsPerRow ||
                        (image != null && IsFullyTransparent(image, CellRect(cell, 1))),
                facesRight ? CharFacing.Right : CharFacing.Down),
        };
    }

    /// <summary>The animated character for a remote texture id, or null if it was never fetched.</summary>
    public AnimatedChar GetRemoteTexture(string id) =>
        string.IsNullOrEmpty(id) ? null : GetAnimatedChar(RemoteKey(id), 0);

    /// <summary>
    /// Namespaced so a numeric remote id can never collide with a sheet name from the manifest.
    /// </summary>
    private static string RemoteKey(string id) => "remote:" + id;

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

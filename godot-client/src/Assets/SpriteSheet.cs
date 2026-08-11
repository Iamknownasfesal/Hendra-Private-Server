using Godot;

namespace Hendra.Assets;

/// <summary>
/// One sprite: the sheet it lives on, and the pixel rectangle it occupies.
/// </summary>
/// <remarks>
/// The AS3 client sliced every sheet into thousands of individual <c>BitmapData</c> objects at
/// startup and then handed those around by reference. Keeping a region into the original texture
/// instead means no per-sprite allocation, and — more importantly — every sprite in the game shares
/// a handful of textures, so the renderer can batch by sheet instead of issuing one draw per
/// object.
/// </remarks>
public readonly struct Sprite
{
    public readonly Texture2D Sheet;
    public readonly Rect2I Region;

    public Sprite(Texture2D sheet, Rect2I region)
    {
        Sheet = sheet;
        Region = region;
    }

    public bool IsValid => Sheet != null;

    /// <summary>The region in normalised texture coordinates, ready for a shader or a quad's UVs.</summary>
    public Rect2 Uv
    {
        get
        {
            var size = Sheet.GetSize();
            return new Rect2(
                Region.Position.X / size.X,
                Region.Position.Y / size.Y,
                Region.Size.X / size.X,
                Region.Size.Y / size.Y);
        }
    }
}

/// <summary>
/// A texture sliced into a uniform grid, addressed by the linear index the game XML uses.
/// </summary>
public sealed class SpriteSheet
{
    public string Name { get; }
    public Texture2D Texture { get; }
    public int TileWidth { get; }
    public int TileHeight { get; }
    public int Columns { get; }
    public int Rows { get; }

    public SpriteSheet(string name, Texture2D texture, int tileWidth, int tileHeight)
    {
        Name = name;
        Texture = texture;
        TileWidth = tileWidth;
        TileHeight = tileHeight;

        var size = texture.GetSize();
        Columns = Mathf.Max(1, (int)size.X / tileWidth);
        Rows = Mathf.Max(1, (int)size.Y / tileHeight);
    }

    public int Count => Columns * Rows;

    /// <summary>
    /// The sprite at <paramref name="index"/>, counting left to right then top to bottom.
    /// Returns an invalid sprite when the index is out of range rather than throwing, because the
    /// game XML does contain indices that point past the end of their sheet.
    /// </summary>
    public Sprite this[int index]
    {
        get
        {
            if (index < 0 || index >= Count)
                return default;

            int column = index % Columns;
            int row = index / Columns;
            return new Sprite(
                Texture,
                new Rect2I(column * TileWidth, row * TileHeight, TileWidth, TileHeight));
        }
    }

    /// <summary>A rectangular block of tiles, used to address one character's strip.</summary>
    public Rect2I Block(int column, int row, int widthInTiles, int heightInTiles) =>
        new(column * TileWidth, row * TileHeight, widthInTiles * TileWidth, heightInTiles * TileHeight);
}

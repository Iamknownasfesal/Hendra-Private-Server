using Godot;
using Hendra.Assets;

namespace Hendra.Render;

/// <summary>
/// A growing texture that blended terrain tiles are baked into.
/// </summary>
/// <remarks>
/// Blended tiles cannot come from the shipped sheets — they are composed at runtime from several
/// terrains at once — but the ground renderer batches by texture, so putting each one in its own
/// texture would mean a draw call per distinct boundary. Packing them into a single atlas keeps the
/// whole map to one surface.
///
/// The atlas never shrinks and entries are never evicted. A map has a bounded number of distinct
/// terrain boundaries, and in the worst case observed the count is in the hundreds, so a fixed
/// allocation is simpler than any reclamation scheme.
/// </remarks>
public sealed class TileAtlas
{
    private const int CellSize = 8;
    private const int Columns = 64;
    private const int Rows = 64;

    private readonly Image _image;
    private readonly ImageTexture _texture;
    private int _next;
    private bool _dirty;

    public TileAtlas()
    {
        _image = Image.CreateEmpty(Columns * CellSize, Rows * CellSize, false, Image.Format.Rgba8);
        _image.Fill(new Color(0f, 0f, 0f, 0f));
        _texture = ImageTexture.CreateFromImage(_image);
    }

    public int Capacity => Columns * Rows;

    public int Count => _next;

    /// <summary>
    /// Copies a baked tile in and returns where it landed. An invalid sprite means the atlas is
    /// full, which the caller should treat as "draw it unblended".
    /// </summary>
    public Sprite Add(Image tile)
    {
        if (tile == null || _next >= Capacity)
            return default;

        int column = _next % Columns;
        int row = _next / Columns;
        _next++;

        var region = new Rect2I(column * CellSize, row * CellSize, CellSize, CellSize);
        _image.BlitRect(tile, new Rect2I(0, 0, CellSize, CellSize), region.Position);
        _dirty = true;

        return new Sprite(_texture, region);
    }

    /// <summary>
    /// Uploads whatever was baked this frame. Call once per frame, before drawing.
    /// </summary>
    /// <remarks>
    /// Batched deliberately: walking into a new area can bake dozens of tiles in one Update, and
    /// uploading the atlas after each would stall on the same texture repeatedly.
    /// </remarks>
    public void Flush()
    {
        if (!_dirty)
            return;

        _texture.Update(_image);
        _dirty = false;
    }
}

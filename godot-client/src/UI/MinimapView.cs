using Godot;
using Hendra.Render;
using Hendra.World;

namespace Hendra.UI;

/// <summary>
/// The minimap: one pixel per tile, revealed as the server streams terrain in.
/// </summary>
/// <remarks>
/// <para>
/// Backed by an <see cref="Image"/> the size of the world, written to as tiles arrive and uploaded
/// only on the frames it actually changed. The original kept the same one-pixel-per-tile idea, but
/// rebuilt its zoom levels by repeatedly halving a bitmap and drew through two masked Shape layers;
/// here the whole thing is one texture drawn into a clipped frame.
/// </para>
/// <para>
/// The map rotates with the camera, so north on the minimap is whichever way the player is
/// currently looking. That matches the original's behaviour and is what makes it usable at all
/// while the camera turns.
/// </para>
/// </remarks>
public partial class MinimapView : Control
{
    private const int Diameter = 192;
    private const int Margin = 12;

    /// <summary>
    /// The interface column the minimap sits at the top of, and the gap around it.
    /// </summary>
    /// <remarks>
    /// The original's: a two-hundred-pixel column with a 192-pixel map inset four pixels either
    /// side, so the map is very nearly the full width of the column and everything else in the
    /// interface hangs below it.
    /// </remarks>
    private const int ColumnWidth = 200;

    private const int ColumnInset = 4;

    /// <summary>How many tiles fit across the minimap by default. Smaller shows more detail.</summary>
    private const float DefaultTilesAcross = 96f;

    /// <summary>The zoom range, in tiles across. Beyond these the map is either useless or a dot.</summary>
    private const float MinTilesAcross = 24f;

    private const float MaxTilesAcross = 384f;

    private float _tilesAcross = DefaultTilesAcross;

    private Image _image;
    private ImageTexture _texture;
    private bool _dirty;

    private GameMap _map;
    private TileColors _colours;

    private float _cameraAngle;

    /// <summary>
    /// Zooms a step in or out.
    /// </summary>
    /// <param name="steps">Positive zooms in, showing fewer tiles.</param>
    /// <remarks>
    /// A halving each step rather than a fixed number of tiles, so the same key press feels the
    /// same whether the map is showing a room or a realm.
    /// </remarks>
    public void Zoom(int steps)
    {
        _tilesAcross = Mathf.Clamp(
            _tilesAcross * Mathf.Pow(0.5f, steps), MinTilesAcross, MaxTilesAcross);
        QueueRedraw();
    }

    public void Configure(GameMap map, TileColors colours)
    {
        _map = map;
        _colours = colours;
        _image = null;
        _texture = null;
    }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        SetAnchorsPreset(LayoutPreset.TopLeft);
        Size = new Vector2(Diameter, Diameter);

        // Clips this node's own drawing as well as its children, which is what keeps the rotated
        // map inside the frame. Godot has no arbitrary-shape scissor, so the frame is square.
        ClipContents = true;

        GetViewport().SizeChanged += PlaceInColumn;
        PlaceInColumn();
    }

    /// <summary>
    /// Puts the map at the top of the interface column, as the original does.
    /// </summary>
    /// <remarks>
    /// It was floating clear of the column before, which left a gap the original does not have and
    /// pushed everything else down a screen it did not need to be pushed down.
    /// </remarks>
    private void PlaceInColumn()
    {
        var viewport = GetViewportRect().Size;
        // Hard into the top-right corner, which is where the layout puts it: the currencies sit to
        // its left and the nearby list under it.
        Position = new Vector2(viewport.X - Diameter - Margin, Margin);
    }

    /// <summary>Records a revealed tile. Cheap enough to call for every tile of every Update.</summary>
    public void SetTile(int x, int y, Square square)
    {
        EnsureImage();
        if (_image == null || x < 0 || y < 0 || x >= _image.GetWidth() || y >= _image.GetHeight())
            return;

        _image.SetPixel(x, y, _colours.Get(square.Desc));
        _dirty = true;
    }

    private void EnsureImage()
    {
        if (_image != null || _map == null || _map.Width <= 0)
            return;

        _image = Image.CreateEmpty(_map.Width, _map.Height, false, Image.Format.Rgba8);
        _image.Fill(new Color(0f, 0f, 0f, 0f));
        _texture = ImageTexture.CreateFromImage(_image);
    }

    public void Refresh(float cameraAngle)
    {
        _cameraAngle = cameraAngle;

        if (_dirty && _texture != null)
        {
            // One upload per frame at most, however many tiles arrived.
            _texture.Update(_image);
            _dirty = false;
        }

        QueueRedraw();
    }

    public override void _Draw()
    {
        var frame = new Rect2(Vector2.Zero, Size);
        var centre = Size / 2f;

        DrawRect(frame, new Color(0.05f, 0.05f, 0.06f, 0.9f));

        var player = _map?.Player;
        if (_texture != null && player != null)
        {
            DrawSetTransformMatrix(MapTransform(player, centre));
            DrawTexture(_texture, Vector2.Zero);
            DrawSetTransform(Vector2.Zero, 0f, Vector2.One);
        }

        DrawRect(frame, new Color(0.45f, 0.45f, 0.5f), filled: false, width: 2f);

        // The player is always at the centre and always facing up, which is the point of rotating
        // the map rather than a marker.
        DrawCircle(centre, 2.5f, Colors.White);
    }

    /// <summary>
    /// Maps world tiles onto the minimap: centred on the player, scaled down, and turned so that
    /// the direction the player is facing points up.
    /// </summary>
    /// <remarks>
    /// Built explicitly rather than by chaining the Transform2D helpers, whose composition order is
    /// easy to get backwards. A Transform2D maps local to global as <c>basis * local + origin</c>,
    /// so the origin is whatever puts the player's tile at the centre.
    /// </remarks>
    private Transform2D MapTransform(LocalPlayer player, Vector2 centre)
    {
        float scale = Diameter / _tilesAcross;

        var transform = new Transform2D(-_cameraAngle, new Vector2(scale, scale), 0f, Vector2.Zero);
        transform.Origin = centre - transform.BasisXform(new Vector2(player.X, player.Y));
        return transform;
    }
}

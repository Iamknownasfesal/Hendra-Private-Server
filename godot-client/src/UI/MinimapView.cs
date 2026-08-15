using System;
using System.Collections.Generic;
using Godot;
using Hendra.Render;
using Hendra.Resources;
using Hendra.World;

namespace Hendra.UI;

/// <summary>
/// The minimap: the world's floor painted a pixel to a tile, with a mark on it for everything
/// nearby.
/// </summary>
/// <remarks>
/// <para>
/// Terrain is back after revision one dropped it. It is one <see cref="Image"/> the size of the
/// world, written to as tiles are revealed and uploaded to the GPU only when the player crosses
/// into a new chunk -- not per frame, and not per tile. A realm streams tens of thousands of tiles
/// in over a couple of minutes, and re-uploading a texture that size on every one of them is the
/// difference between a map that costs nothing and one that costs more than the world does.
/// </para>
/// <para>
/// Zooming is the = and - keys, as the original had it, and a pair of buttons in the bottom corner
/// of the map, because a control nobody can see is a control nobody finds.
/// </para>
/// <para>
/// The map does not rotate. The original's did, and turning with the camera makes a map easier to
/// follow while the camera is turning and much harder to remember afterwards; a fixed north means
/// the shape of a dungeon is the same shape every time you look at it.
/// </para>
/// </remarks>
public partial class MinimapView : Control
{
    /// <summary>How many tiles fit across the map at each zoom step. Three, as the brief asks.</summary>
    private static readonly float[] ZoomLevels = { 48f, 96f, 192f };

    /// <summary>The most blips drawn in a frame, nearest first.</summary>
    private const int MostBlips = 64;

    private readonly List<(float Distance, float X, float Y, BlipKind Kind)> _blips = new(96);

    private MapFrame _panel;
    private Terrain _terrain;
    private Blips _canvas;
    private ZoomButton _zoomIn;
    private ZoomButton _zoomOut;

    private GameMap _map;
    private TileColors _colours;

    private Image _image;
    private ImageTexture _texture;

    /// <summary>Whether a tile has been written since the last upload.</summary>
    private bool _dirty;

    private int _level = 1;

    /// <summary>How many times the map has been sent to the GPU. Zero while nothing is revealed.</summary>
    public int TerrainUploads { get; private set; }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;

        _panel = new MapFrame();
        AddChild(_panel);

        _terrain = new Terrain(this);
        _panel.AddChild(_terrain);

        _canvas = new Blips(this);
        _panel.AddChild(_canvas);

        // The keys are still there, but a control nobody can see is a control nobody uses.
        _zoomIn = new ZoomButton(HudIcons.Plus, "Zoom in [=]");
        _zoomIn.Pressed += () => Zoom(1);
        _panel.AddChild(_zoomIn);

        _zoomOut = new ZoomButton(HudIcons.Minus, "Zoom out [-]");
        _zoomOut.Pressed += () => Zoom(-1);
        _panel.AddChild(_zoomOut);

        _level = Mathf.Clamp(App.ServiceLocator.Settings?.MinimapZoom ?? 1, 0, ZoomLevels.Length - 1);
        ShowSteps();

        Resized += Reflow;
        if (GetParent() is HudLayer layer)
            layer.Reflowed += Reflow;

        Reflow();
    }

    private void Reflow()
    {
        if (_panel == null)
            return;

        var layout = new HudLayout(Size.X > 0f && Size.Y > 0f
            ? Size
            : new Vector2(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight));

        var rect = layout.Minimap;
        _panel.Position = rect.Position;
        _panel.Size = rect.Size;

        // The painted floor sits inside the frame; the frame is drawn by the panel around it.
        var face = layout.MinimapFace;
        foreach (var child in new Control[] { _terrain, _canvas })
        {
            child.Position = face.Position - rect.Position;
            child.Size = face.Size;
        }

        // Stacked against the map's right edge at the top, where the reference has them: the
        // player's own mark is in the middle of the map and the bottom corner is where a dungeon's
        // entrance usually ends up.
        if (_zoomIn == null)
            return;

        var side = new Vector2(HudLayout.ZoomButtonWidth, HudLayout.ZoomButtonHeight);
        float left = rect.Size.X - HudLayout.MinimapFrame - 5f - side.X;

        _zoomIn.Size = side;
        _zoomOut.Size = side;
        _zoomIn.Position = new Vector2(left, 7f);
        _zoomOut.Position = new Vector2(left, 7f + side.Y + 6f);
    }

    /// <summary>Greys whichever button has no step left in it, as the reference shows them.</summary>
    private void ShowSteps()
    {
        if (_zoomIn == null)
            return;

        _zoomIn.Spent = _level <= 0;
        _zoomOut.Spent = _level >= ZoomLevels.Length - 1;
    }

    public void Configure(GameMap map, TileColors colours)
    {
        _map = map;
        _colours = colours;

        _image = null;
        _texture = null;
        _dirty = false;
    }

    /// <summary>Records a revealed tile. Cheap enough to call for every tile of every Update.</summary>
    public void SetTile(int x, int y, Square square) => Paint(x, y, _colours.Get(square.Desc));

    /// <summary>
    /// Paints a static object onto the map, over the ground it stands on.
    /// </summary>
    /// <remarks>
    /// What makes a dungeon's minimap a floor plan rather than a wash of floor colour. The original
    /// bakes an object in on exactly one condition -- static, occupying its square, and not marked
    /// no-minimap -- and never takes it off again, which is why a wall you have blown up is still
    /// on your map.
    /// </remarks>
    public void SetObject(int x, int y, ObjectDesc desc) => Paint(x, y, _colours.Get(desc));

    private void Paint(int x, int y, Color colour)
    {
        EnsureImage();

        if (_image == null || x < 0 || y < 0 || x >= _image.GetWidth() || y >= _image.GetHeight())
            return;

        _image.SetPixel(x, y, colour);
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

    /// <summary>
    /// Zooms a step in or out.
    /// </summary>
    /// <param name="steps">Positive zooms in, showing fewer tiles.</param>
    public void Zoom(int steps)
    {
        int level = Mathf.Clamp(_level - steps, 0, ZoomLevels.Length - 1);
        if (level == _level)
            return;

        _level = level;
        ShowSteps();

        var settings = App.ServiceLocator.Settings;
        if (settings != null)
        {
            settings.MinimapZoom = level;
            settings.Save();
        }

        _terrain?.QueueRedraw();
        _canvas?.QueueRedraw();
    }

    /// <summary>
    /// Call once a frame.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Both layers are redrawn every frame, because both move every frame: the map is centred on
    /// the player, so the ground slides under the marks exactly as the marks slide over it. The
    /// original does the same -- its <c>draw()</c> clears both layers and re-fills the ground from
    /// its bitmap at the player's offset on every HUD update.
    /// </para>
    /// <para>
    /// What is <i>not</i> per-frame is the upload. Drawing the ground is one textured quad; sending
    /// a map-sized image to the GPU is not, so that happens only on the frames a tile actually
    /// arrived. Redrawing only on a chunk boundary -- which is what this did -- made the ground jump
    /// eight tiles at a time while the marks on it moved smoothly.
    /// </para>
    /// </remarks>
    public void Refresh()
    {
        _terrain?.QueueRedraw();
        _canvas?.QueueRedraw();

        if (!_dirty || _texture == null)
            return;

        _texture.Update(_image);
        _dirty = false;
        TerrainUploads++;
    }

    /// <summary>
    /// The map's plate: black under the floor, with a light frame around all four sides.
    /// </summary>
    /// <remarks>
    /// The frame's bottom edge is also the rule between the map and the icon row under it -- there
    /// is one line there in the reference, not two touching ones, which is why the row below draws
    /// no top edge of its own.
    /// </remarks>
    private sealed partial class MapFrame : Control
    {
        public MapFrame()
        {
            // The map is the one part of the column that answers a click with something other than
            // the world, so it takes the pointer.
            MouseFilter = MouseFilterEnum.Stop;
        }

        public override void _Draw()
        {
            var full = new Rect2(Vector2.Zero, Size);

            DrawRect(full, ColumnInk.Frame);
            DrawRect(full.Grow(-HudLayout.MinimapFrame), Style.PanelSolid);
        }
    }

    /// <summary>
    /// One of the two zoom steps, stacked against the map's right edge.
    /// </summary>
    /// <remarks>
    /// Two states and no hover plate: a step that can still be taken is a light plate with a white
    /// mark, and one that has run out is a dark plate with a grey mark. The reference draws them
    /// exactly this way, and it means the pair say which end of the range you are at without the
    /// pointer having to be anywhere near them.
    /// </remarks>
    private sealed partial class ZoomButton : Control
    {
        private readonly Action<CanvasItem, Rect2, Color> _icon;

        private bool _spent;

        public ZoomButton(Action<CanvasItem, Rect2, Color> icon, string tooltip)
        {
            _icon = icon;
            TooltipText = tooltip;
            MouseFilter = MouseFilterEnum.Stop;
            FocusMode = FocusModeEnum.None;
        }

        public event Action Pressed;

        /// <summary>Whether there is no step left in this direction.</summary>
        public bool Spent
        {
            get => _spent;
            set { _spent = value; QueueRedraw(); }
        }

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is not InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
                return;

            AcceptEvent();

            if (!_spent)
                Pressed?.Invoke();
        }

        public override void _Draw()
        {
            DrawRect(new Rect2(Vector2.Zero, Size),
                _spent ? ColumnInk.ButtonPlateSpent : ColumnInk.ButtonPlate);

            // The mark is about half the plate, which is what stops a plus reading as a window
            // divided into four.
            var box = new Rect2(Vector2.Zero, Size).Grow(-Mathf.Round(Size.X * 0.26f));
            _icon(this, box, _spent ? ColumnInk.ButtonPlate : Style.Text);
        }
    }

    /// <summary>The painted floor, under the marks.</summary>
    private sealed partial class Terrain : Control
    {
        private readonly MinimapView _owner;

        public Terrain(MinimapView owner)
        {
            _owner = owner;
            MouseFilter = MouseFilterEnum.Ignore;
            ClipContents = true;

            // One pixel per tile blown up eight times over: it has to stay square-edged, or the
            // map turns into a watercolour of itself.
            TextureFilter = TextureFilterEnum.Nearest;
        }

        public override void _Draw() => _owner.DrawTerrain(this, Size);
    }

    /// <summary>
    /// The marks themselves, over the floor.
    /// </summary>
    /// <remarks>
    /// A separate control from the terrain so the two redraw on their own schedules -- which is the
    /// whole point of caching the floor.
    /// </remarks>
    private sealed partial class Blips : Control
    {
        private readonly MinimapView _owner;

        public Blips(MinimapView owner)
        {
            _owner = owner;
            MouseFilter = MouseFilterEnum.Ignore;
            ClipContents = true;
        }

        public override void _Draw() => _owner.DrawBlips(this, Size);
    }

    private float PixelsPerTile(Vector2 size) => size.X / ZoomLevels[_level];

    private void DrawTerrain(CanvasItem into, Vector2 size)
    {
        var player = _map?.Player;
        if (_texture == null || player == null)
            return;

        float scale = PixelsPerTile(size);
        var origin = size / 2f - new Vector2(player.X, player.Y) * scale;

        into.DrawTextureRect(_texture,
            new Rect2(origin, new Vector2(_image.GetWidth(), _image.GetHeight()) * scale), false);
    }

    /// <summary>
    /// Everything worth knowing the position of, as coloured squares.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Colour carries the kind, and it is the game's own code rather than a new one: yellow for
    /// other players, green for guildmates, purple for the party, red for anything hostile, blue
    /// for a way out, and white for whatever the current quest is pointing at. Read at a glance and
    /// never legended, which only works because it is the code every player already knows.
    /// </para>
    /// <para>
    /// Capped, and sorted by distance before the cap, so a crowded realm draws the sixty nearest
    /// things rather than every entity the client knows about.
    /// </para>
    /// </remarks>
    private void DrawBlips(CanvasItem into, Vector2 size)
    {
        var player = _map?.Player;
        if (player == null)
            return;

        var centre = size / 2f;
        float scale = PixelsPerTile(size);

        _blips.Clear();

        foreach (var entity in _map.Entities)
        {
            if (ReferenceEquals(entity, player) || entity.Dead || entity.Desc == null)
                continue;

            var kind = KindOf(entity, player);
            if (kind == BlipKind.None)
                continue;

            float dx = entity.X - player.X;
            float dy = entity.Y - player.Y;
            _blips.Add((dx * dx + dy * dy, dx, dy, kind));
        }

        _blips.Sort(static (a, b) => a.Distance.CompareTo(b.Distance));

        int drawn = 0;
        foreach (var blip in _blips)
        {
            // The cap is for the crowd. A boss or a quest target is never one of the things
            // crowded off, however many monsters are standing between here and it.
            bool always = AlwaysShown(blip.Kind);
            if (!always && drawn >= MostBlips)
                continue;

            float side = SizeOf(blip.Kind);
            var at = centre + new Vector2(blip.X, blip.Y) * scale;

            if (always)
                at = PinToEdge(at, size, side);
            else if (at.X < 0f || at.Y < 0f || at.X > size.X || at.Y > size.Y)
                continue;

            // Counted where it is spent, so the budget buys sixty-four marks the player can see
            // rather than being used up by things sorted nearest but still off the edge.
            if (!always)
                drawn++;

            var box = new Rect2(
                Mathf.Round(at.X - side / 2f), Mathf.Round(at.Y - side / 2f), side, side);

            var colour = ColourOf(blip.Kind);

            switch (blip.Kind)
            {
                case BlipKind.Boss:
                    HudIcons.Helmet(into, box, colour);
                    break;

                case BlipKind.Quest:
                    HudIcons.Skull(into, box, colour);
                    break;

                default:
                    // A dark surround, so a yellow mark still reads over a sunlit floor.
                    into.DrawRect(box.Grow(1f), Style.PanelEdge);
                    into.DrawRect(box, colour);
                    break;
            }
        }

        // The player is always at the centre, which is the other half of not rotating the map.
        var self = new Rect2(Mathf.Round(centre.X - 4f), Mathf.Round(centre.Y - 4f), 8f, 8f);
        into.DrawRect(self.Grow(1f), Style.PanelEdge);
        into.DrawRect(self, Style.Text);
    }

    /// <summary>
    /// Holds a mark inside the map, so something off the edge still says which way it lies.
    /// </summary>
    private static Vector2 PinToEdge(Vector2 at, Vector2 size, float side)
    {
        float margin = side / 2f + 1f;
        return new Vector2(
            Mathf.Clamp(at.X, margin, size.X - margin),
            Mathf.Clamp(at.Y, margin, size.Y - margin));
    }

    private enum BlipKind : byte
    {
        None,
        Player,
        Guildmate,
        Enemy,

        /// <summary>A god. Numerous enough to want telling apart from the rank and file.</summary>
        God,
        Portal,

        /// <summary>A Hero of Oryx or an encounter boss: worth crossing the map for.</summary>
        Boss,

        /// <summary>Whatever the current quest is pointing at.</summary>
        Quest,
    }

    /// <summary>
    /// Whether a mark is shown however far away it is, rather than only inside the zoom.
    /// </summary>
    /// <remarks>
    /// The point of a map is to say where to go next, and the two things worth going to are the
    /// quest and a boss. Both are pinned to the edge when they fall outside the view, which is the
    /// only way a player standing in the wrong corner of the Realm ever learns they exist.
    /// </remarks>
    private static bool AlwaysShown(BlipKind kind) => kind is BlipKind.Boss or BlipKind.Quest;

    /// <summary>What an entity counts as, or None if it is not worth a mark.</summary>
    private BlipKind KindOf(Entity entity, LocalPlayer player)
    {
        var desc = entity.Desc;

        if (desc.IsPlayer)
        {
            // Guild before player, so a guildmate is green rather than yellow. There is no purple
            // for a party: this server has no party system, and the list beside the map is a
            // proximity list -- colouring it would make every player on screen purple.
            return !string.IsNullOrEmpty(player.Guild) && entity.Guild == player.Guild
                ? BlipKind.Guildmate
                : BlipKind.Player;
        }

        if (desc.IsEnemy)
        {
            if (entity.IsInvisible)
                return BlipKind.None;

            if (entity.ObjectId == QuestTargetId)
                return BlipKind.Quest;

            if (desc.IsHero || desc.IsEncounter)
                return BlipKind.Boss;

            return desc.IsGod ? BlipKind.God : BlipKind.Enemy;
        }

        return desc.Class is "Portal" or "GuildHallPortal" ? BlipKind.Portal : BlipKind.None;
    }

    /// <summary>The object the quest arrow is pointing at, which gets its own mark. Zero for none.</summary>
    public int QuestTargetId { get; set; }

    /// <summary>Six to twelve pixels, by how much it matters that you noticed it.</summary>
    private static float SizeOf(BlipKind kind) => kind switch
    {
        BlipKind.Boss => 12f,
        BlipKind.Quest => 11f,
        BlipKind.Portal => 10f,
        BlipKind.God => 9f,
        BlipKind.Enemy => 8f,
        BlipKind.Guildmate => 8f,
        _ => 6f,
    };

    private static Color ColourOf(BlipKind kind) => kind switch
    {
        BlipKind.Guildmate => Style.BlipGuild,
        BlipKind.Enemy => Style.BlipEnemy,
        BlipKind.God => Style.BlipGod,
        BlipKind.Portal => Style.BlipPortal,
        BlipKind.Boss => Style.BlipBoss,
        BlipKind.Quest => Style.BlipQuest,
        _ => Style.BlipPlayer,
    };

}

using Godot;

namespace Hendra.Render;

/// <summary>
/// The 3D scene the world is drawn into: the camera, the terrain surface and the sprite surface.
/// </summary>
/// <remarks>
/// <para>
/// Deliberately just two mesh instances. The original maintained two entire parallel renderers — a
/// CPU rasteriser building a flat display list, and a Stage3D path that smuggled shader parameters
/// through a side table keyed by fill object — and switched between them at runtime, with mouse
/// hit-testing changing behaviour depending on which was active. Both collapse into this.
/// </para>
/// <para>
/// The camera is orthographic and points straight down; see <see cref="WorldProjection"/> for why
/// that reproduces the original's oblique projection rather than approximating it. Because the
/// projection is orthographic, depth is linear, so the tightly packed sort keys the projection
/// hands out resolve cleanly across the whole depth range.
/// </para>
/// </remarks>
public partial class WorldRoot : Node3D
{
    /// <summary>Camera height. Arbitrary under an orthographic projection; it only has to clear the sort keys.</summary>
    private const float CameraHeight = 64f;

    private const float NearPlane = 1f;
    private const float FarPlane = 256f;

    private Camera3D _camera;
    private MeshInstance3D _groundInstance;
    private MeshInstance3D _spriteInstance;
    private ImmediateMesh _groundMesh;
    private ImmediateMesh _spriteMesh;

    /// <summary>Queue terrain here before calling <see cref="Render"/>.</summary>
    public GroundDrawList Ground { get; } = new();

    /// <summary>Queue sprites here before calling <see cref="Render"/>.</summary>
    public SpriteDrawList Sprites { get; } = new();

    /// <summary>The current projection, rebuilt whenever the camera angle changes.</summary>
    public WorldProjection Projection { get; private set; } = new(0f);

    /// <summary>Extra magnification on top of the fixed 50 pixels per tile.</summary>
    public float Zoom { get; set; } = 1f;

    public override void _Ready()
    {
        _camera = new Camera3D
        {
            Projection = Camera3D.ProjectionType.Orthogonal,
            Near = NearPlane,
            Far = FarPlane,
            Current = true,
        };
        AddChild(_camera);

        _groundMesh = new ImmediateMesh();
        _groundInstance = new MeshInstance3D
        {
            Mesh = _groundMesh,
            CastShadow = GeometryInstance3D.ShadowCastingSetting.Off,
        };
        AddChild(_groundInstance);

        _spriteMesh = new ImmediateMesh();
        _spriteInstance = new MeshInstance3D
        {
            Mesh = _spriteMesh,
            CastShadow = GeometryInstance3D.ShadowCastingSetting.Off,
        };
        AddChild(_spriteInstance);
    }

    /// <summary>
    /// Points the camera at a world position and sets the viewing angle.
    /// </summary>
    /// <param name="focusX">Focus point, in tiles.</param>
    /// <param name="focusY">Focus point, in tiles.</param>
    /// <param name="angle">Camera rotation, in radians. The original's cameraAngle.</param>
    public void Configure(float focusX, float focusY, float angle)
    {
        Projection = new WorldProjection(angle);

        // Pitch straight down, then yaw. Godot composes Euler angles in YXZ order, so the yaw is
        // applied in world space and the pitch in the camera's own.
        _camera.Position = Projection.CameraPosition(focusX, focusY);
        _camera.Rotation = new Vector3(-Mathf.Pi / 2f, Projection.CameraYaw, 0f);

        // Size is the vertical extent of the view in world units, i.e. in tiles.
        float viewportHeight = GetViewport().GetVisibleRect().Size.Y;
        _camera.Size = viewportHeight / (WorldProjection.PixelsPerTile * Mathf.Max(Zoom, 0.01f));
    }

    /// <summary>
    /// How far from the focus point anything can still be on screen, in tiles. Used to bound the
    /// terrain sweep, and matching the original's maxDist.
    /// </summary>
    public float VisibleRadius()
    {
        var size = GetViewport().GetVisibleRect().Size;
        float scale = WorldProjection.PixelsPerTile * Mathf.Max(Zoom, 0.01f);
        float halfWidth = size.X / (2f * scale);
        float halfHeight = size.Y / (2f * scale);
        return Mathf.Sqrt(halfWidth * halfWidth + halfHeight * halfHeight) + 1f;
    }

    /// <summary>Turns the queued terrain and sprites into geometry, then empties the queues.</summary>
    public void Render()
    {
        var projection = Projection;
        Ground.Build(_groundMesh, projection);
        Sprites.Build(_spriteMesh, projection);

        Ground.Clear();
        Sprites.Clear();
    }
}

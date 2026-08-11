using System;
using System.Collections.Generic;
using Godot;
using Hendra.Assets;

namespace Hendra.Render;

/// <summary>One model instance queued for this frame.</summary>
public struct ModelDraw
{
    public Model3D Model;

    /// <summary>Where it stands, in tiles.</summary>
    public float TileX;

    public float TileY;

    /// <summary>Rotation about the vertical, in radians. The object's declared Rotation.</summary>
    public float Rotation;

    /// <summary>The sprite its textured faces sample.</summary>
    public Sprite Sprite;

    /// <summary>The colour its untextured faces take. The object's declared Color.</summary>
    public Color SolidColor;

    /// <summary>Nudges depth without moving the model.</summary>
    public float SortBias;
}

/// <summary>
/// Real geometry for the objects the game data gives a <c>&lt;Model&gt;</c>.
/// </summary>
/// <remarks>
/// <para>
/// These are the only things in the world that are genuinely three-dimensional: pillars, arches,
/// crates, the odd fountain. Everything else is a flat quad. The original rasterised them on the
/// CPU — projecting every vertex by hand, sorting the faces, and pushing a fill, a path and an
/// end-fill per face into the same display list it used for sprites.
/// </para>
/// <para>
/// The geometry is rebuilt every frame rather than kept as a static mesh, and that is not laziness.
/// The projection folds height into the ground plane along the camera's up vector, so a model's
/// vertices land in different scene positions as the camera turns; a mesh built once would shear
/// the wrong way the moment the player pressed Q. Rebuilding costs a few hundred vertices for the
/// handful of models on screen at a time.
/// </para>
/// <para>
/// Depth comes from each vertex's own ground point, so a model sorts against the world *and*
/// against itself — the near face of a pillar is drawn over its far face without needing the face
/// ordering the original had to precompute.
/// </para>
/// </remarks>
public sealed class ModelDrawList
{
    /// <summary>
    /// The light the original shades faces by, in game axes, normalised.
    /// </summary>
    /// <remarks>
    /// It points up, south and east, so the tops of things are brightest and the north-west faces
    /// darkest. Fixed in world space and not affected by the camera, which is what stops a rotating
    /// camera making the world appear to light up and go dark.
    /// </remarks>
    private static readonly Vector3 Light = new Vector3(1f, 3f, 2f).Normalized();

    /// <summary>How dark an unlit face is. The rest of the range is filled in by the light.</summary>
    private const float AmbientShade = 0.75f;

    private readonly List<ModelDraw> _draws = new(32);
    private readonly Dictionary<Texture2D, List<ModelDraw>> _textured = new();
    private readonly List<Texture2D> _order = new();
    private readonly Dictionary<Texture2D, StandardMaterial3D> _materials = new();

    private StandardMaterial3D _solidMaterial;

    /// <summary>One finished vertex, waiting for its surface to be opened.</summary>
    private readonly record struct Emitted(Vector3 Position, Vector2 Uv, Color Tint);

    private readonly List<Emitted> _scratch = new(1024);

    public void Clear()
    {
        _draws.Clear();
        foreach (var list in _textured.Values)
            list.Clear();
    }

    public void Add(in ModelDraw draw)
    {
        if (draw.Model is not { IsValid: true })
            return;

        _draws.Add(draw);

        if (!draw.Sprite.IsValid)
            return;

        if (!_textured.TryGetValue(draw.Sprite.Sheet, out var list))
        {
            list = new List<ModelDraw>(16);
            _textured[draw.Sprite.Sheet] = list;
            _order.Add(draw.Sprite.Sheet);
        }

        list.Add(draw);
    }

    /// <summary>Writes the queued models into <paramref name="mesh"/>, textured faces then solid.</summary>
    public void Build(ImmediateMesh mesh, in WorldProjection projection)
    {
        mesh.ClearSurfaces();

        foreach (var sheet in _order)
        {
            var draws = _textured[sheet];
            if (draws.Count == 0)
                continue;

            _scratch.Clear();
            foreach (var draw in draws)
                Gather(draw, projection, textured: true);

            Flush(mesh, MaterialFor(sheet));
        }

        // Untextured faces are rare enough -- a couple of models use them -- to be worth one extra
        // surface rather than a white texel in every sheet.
        _scratch.Clear();
        foreach (var draw in _draws)
            Gather(draw, projection, textured: false);

        Flush(mesh, SolidMaterial());
    }

    /// <summary>
    /// Writes the gathered vertices as one surface, or nothing at all when there are none.
    /// </summary>
    /// <remarks>
    /// The emptiness check is why the vertices are collected before the surface is opened rather
    /// than written straight into it: back-face culling can reject every triangle a model has, and
    /// closing a surface that received no vertices is an error.
    /// </remarks>
    private void Flush(ImmediateMesh mesh, Material material)
    {
        if (_scratch.Count == 0)
            return;

        mesh.SurfaceBegin(Mesh.PrimitiveType.Triangles);

        foreach (var vertex in _scratch)
        {
            mesh.SurfaceSetColor(vertex.Tint);
            mesh.SurfaceSetUV(vertex.Uv);
            mesh.SurfaceAddVertex(vertex.Position);
        }

        mesh.SurfaceEnd();
        mesh.SurfaceSetMaterial(mesh.GetSurfaceCount() - 1, material);
    }

    /// <summary>Collects one model's faces of the requested kind into the scratch buffer.</summary>
    private void Gather(in ModelDraw draw, in WorldProjection projection, bool textured)
    {
        // Textured faces need a sprite to sample; without one they are simply not drawn, which is
        // what the original does when an object has a model but no artwork.
        if (textured && !draw.Sprite.IsValid)
            return;

        var model = draw.Model;
        float sin = Mathf.Sin(draw.Rotation);
        float cos = Mathf.Cos(draw.Rotation);
        var region = draw.Sprite.IsValid ? draw.Sprite.Uv : new Rect2();

        foreach (var triangle in model.Triangles)
        {
            if (triangle.Textured != textured)
                continue;

            var a = World(model.Vertices[triangle.A], sin, cos, draw);
            var b = World(model.Vertices[triangle.B], sin, cos, draw);
            var c = World(model.Vertices[triangle.C], sin, cos, draw);

            if (IsBackFacing(a, b, c, projection))
                continue;

            var normal = Rotate(triangle.Normal, sin, cos);
            float shade = AmbientShade + (1f - AmbientShade) * Mathf.Max(0f, normal.Dot(Light));

            var tint = textured
                ? new Color(shade, shade, shade)
                : new Color(draw.SolidColor.R * shade, draw.SolidColor.G * shade, draw.SolidColor.B * shade);

            Add(a, model.Uvs[triangle.A], region, projection, draw.SortBias, tint, textured);
            Add(b, model.Uvs[triangle.B], region, projection, draw.SortBias, tint, textured);
            Add(c, model.Uvs[triangle.C], region, projection, draw.SortBias, tint, textured);
        }
    }

    private void Add(
        Vector3 world,
        Vector2 uv,
        in Rect2 region,
        in WorldProjection projection,
        float sortBias,
        Color tint,
        bool textured)
    {
        // Model coordinates run 0 to 1 over the object's own sprite, which is one small rectangle
        // of a shared sheet. Clamped first: a coordinate outside the unit square would sample
        // whatever sprite happens to sit next door on the atlas.
        var mapped = textured
            ? new Vector2(
                region.Position.X + Mathf.Clamp(uv.X, 0f, 1f) * region.Size.X,
                region.Position.Y + Mathf.Clamp(uv.Y, 0f, 1f) * region.Size.Y)
            : Vector2.Zero;

        _scratch.Add(new Emitted(projection.ToScene(world.X, world.Y, world.Z, sortBias), mapped, tint));
    }

    /// <summary>Places a model vertex in the world: rotated about the vertical, then moved to the object.</summary>
    private static Vector3 World(Vector3 vertex, float sin, float cos, in ModelDraw draw)
    {
        var rotated = Rotate(vertex, sin, cos);
        return new Vector3(rotated.X + draw.TileX, rotated.Y + draw.TileY, rotated.Z);
    }

    private static Vector3 Rotate(Vector3 v, float sin, float cos) =>
        new(v.X * cos - v.Y * sin, v.X * sin + v.Y * cos, v.Z);

    /// <summary>
    /// Whether a triangle faces away from the viewer, by the sign of its area on screen.
    /// </summary>
    /// <remarks>
    /// Done here rather than by the graphics card because the projection is oblique: height is
    /// sheared into the ground plane, and a card culling by the winding of the sheared geometry
    /// would disagree with the original at every camera angle but one. This is the original's own
    /// test, on the original's own screen coordinates.
    /// </remarks>
    private static bool IsBackFacing(Vector3 a, Vector3 b, Vector3 c, in WorldProjection projection)
    {
        var (ax, ay) = Screen(a, projection);
        var (bx, by) = Screen(b, projection);
        var (cx, cy) = Screen(c, projection);

        return (bx - ax) * (cy - ay) - (by - ay) * (cx - ax) < 0f;
    }

    /// <summary>A world point in the original's screen axes: X to the right, Y downward.</summary>
    private static (float X, float Y) Screen(Vector3 world, in WorldProjection projection)
    {
        float sin = Mathf.Sin(projection.Angle);
        float cos = Mathf.Cos(projection.Angle);
        return (world.X * cos + world.Y * sin, projection.ScreenDepth(world.X, world.Y) - world.Z);
    }

    private StandardMaterial3D MaterialFor(Texture2D sheet)
    {
        if (_materials.TryGetValue(sheet, out var material))
            return material;

        material = new StandardMaterial3D
        {
            AlbedoTexture = sheet,
            ShadingMode = BaseMaterial3D.ShadingModeEnum.Unshaded,
            TextureFilter = BaseMaterial3D.TextureFilterEnum.Nearest,

            // Culled on the processor instead; see IsBackFacing.
            CullMode = BaseMaterial3D.CullModeEnum.Disabled,
            VertexColorUseAsAlbedo = true,
        };

        _materials[sheet] = material;
        return material;
    }

    private StandardMaterial3D SolidMaterial() =>
        _solidMaterial ??= new StandardMaterial3D
        {
            ShadingMode = BaseMaterial3D.ShadingModeEnum.Unshaded,
            CullMode = BaseMaterial3D.CullModeEnum.Disabled,
            VertexColorUseAsAlbedo = true,
        };
}

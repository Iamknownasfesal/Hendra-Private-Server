using System.Collections.Generic;
using Godot;
using Hendra.Assets;

namespace Hendra.Render;

/// <summary>
/// One screen-aligned quad queued for this frame.
/// </summary>
public struct SpriteDraw
{
    /// <summary>The object's ground position, in tiles.</summary>
    public float TileX;

    public float TileY;

    /// <summary>Height above the ground, in tiles.</summary>
    public float Height;

    /// <summary>Nudges depth without moving the quad. Negative draws beneath, positive above.</summary>
    public float SortBias;

    /// <summary>The pixels to sample.</summary>
    public Sprite Sprite;

    /// <summary>Quad size in tiles, before mirroring.</summary>
    public float WidthTiles;

    public float HeightTiles;

    /// <summary>
    /// Where the quad's anchor sits horizontally, as a fraction of its width. 0.5 centres it.
    /// The attack composite uses this to keep the character centred while the weapon overhangs.
    /// </summary>
    public float AnchorX;

    public bool Mirrored;

    /// <summary>Multiplied into the sampled colour. Carries tints, fades and flashes.</summary>
    public Color Modulate;
}

/// <summary>
/// Collects the frame's sprites and emits them as screen-aligned quads, grouped by texture.
/// </summary>
/// <remarks>
/// <para>
/// Sprites are horizontal quads rather than upright billboards. That falls out of the projection:
/// the camera looks straight down, so a quad lying flat in the ground plane is seen face-on and
/// maps directly to a screen rectangle. Orienting it by the camera yaw keeps it axis-aligned on
/// screen no matter which way the world is turned.
/// </para>
/// <para>
/// Grouping by texture is the point of the exercise. The original appended a fill, a path and an
/// end-fill to a flat display list for every sprite, tile face, particle, health bar and name plate,
/// then walked the whole list again in its GPU path to pull out the entries that had to be drawn in
/// software. Here each sheet becomes one surface, so a crowded screen is a handful of draws.
/// </para>
/// </remarks>
public sealed class SpriteDrawList
{
    private readonly Dictionary<Texture2D, List<SpriteDraw>> _byTexture = new();
    private readonly List<Texture2D> _order = new();

    public void Clear()
    {
        foreach (var list in _byTexture.Values)
            list.Clear();
    }

    public void Add(in SpriteDraw draw)
    {
        if (!draw.Sprite.IsValid)
            return;

        if (!_byTexture.TryGetValue(draw.Sprite.Sheet, out var list))
        {
            list = new List<SpriteDraw>(256);
            _byTexture[draw.Sprite.Sheet] = list;
            _order.Add(draw.Sprite.Sheet);
        }

        list.Add(draw);
    }

    /// <summary>
    /// Writes the queued quads into <paramref name="mesh"/>, one surface per texture.
    /// </summary>
    public void Build(ImmediateMesh mesh, in WorldProjection projection)
    {
        mesh.ClearSurfaces();

        // Screen right and screen down, expressed in the ground plane. Every quad is built from
        // these two, which is what keeps sprites upright on screen as the camera rotates.
        float sin = Mathf.Sin(projection.Angle);
        float cos = Mathf.Cos(projection.Angle);
        var right = new Vector3(cos, 0f, sin);
        var down = new Vector3(-sin, 0f, cos);

        foreach (var texture in _order)
        {
            var draws = _byTexture[texture];
            if (draws.Count == 0)
                continue;

            mesh.SurfaceBegin(Mesh.PrimitiveType.Triangles);

            foreach (var draw in draws)
                Emit(mesh, draw, projection, right, down);

            mesh.SurfaceEnd();
            mesh.SurfaceSetMaterial(mesh.GetSurfaceCount() - 1, MaterialFor(texture));
        }
    }

    private static void Emit(
        ImmediateMesh mesh,
        in SpriteDraw draw,
        in WorldProjection projection,
        Vector3 right,
        Vector3 down)
    {
        var uv = draw.Sprite.Uv;
        float u0 = uv.Position.X;
        float u1 = uv.Position.X + uv.Size.X;
        if (draw.Mirrored)
            (u0, u1) = (u1, u0);

        float v0 = uv.Position.Y;
        float v1 = uv.Position.Y + uv.Size.Y;

        // The anchor sits on the ground point: horizontally by AnchorX, vertically at the bottom
        // edge, so a sprite stands on its tile rather than straddling it.
        var anchor = projection.ToScene(draw.TileX, draw.TileY, draw.Height, draw.SortBias);
        float left = -draw.WidthTiles * draw.AnchorX;
        float rightEdge = left + draw.WidthTiles;

        var topLeft = anchor + right * left + down * -draw.HeightTiles;
        var topRight = anchor + right * rightEdge + down * -draw.HeightTiles;
        var bottomRight = anchor + right * rightEdge;
        var bottomLeft = anchor + right * left;

        mesh.SurfaceSetColor(draw.Modulate);

        Vertex(mesh, topLeft, u0, v0);
        Vertex(mesh, topRight, u1, v0);
        Vertex(mesh, bottomRight, u1, v1);

        Vertex(mesh, topLeft, u0, v0);
        Vertex(mesh, bottomRight, u1, v1);
        Vertex(mesh, bottomLeft, u0, v1);
    }

    private static void Vertex(ImmediateMesh mesh, Vector3 position, float u, float v)
    {
        mesh.SurfaceSetUV(new Vector2(u, v));
        mesh.SurfaceAddVertex(position);
    }

    private readonly Dictionary<Texture2D, StandardMaterial3D> _materials = new();

    private StandardMaterial3D MaterialFor(Texture2D texture)
    {
        if (_materials.TryGetValue(texture, out var material))
            return material;

        material = new StandardMaterial3D
        {
            AlbedoTexture = texture,
            ShadingMode = BaseMaterial3D.ShadingModeEnum.Unshaded,
            // The art is 8x8 pixel sprites scaled up; anything but nearest turns them to mush.
            TextureFilter = BaseMaterial3D.TextureFilterEnum.Nearest,
            Transparency = BaseMaterial3D.TransparencyEnum.AlphaScissor,
            AlphaScissorThreshold = 0.5f,
            CullMode = BaseMaterial3D.CullModeEnum.Disabled,
            VertexColorUseAsAlbedo = true,
        };

        _materials[texture] = material;
        return material;
    }
}

using System.Collections.Generic;
using Godot;
using Hendra.Assets;

namespace Hendra.Render;

/// <summary>One tile queued for this frame.</summary>
public struct GroundDraw
{
    public int TileX;
    public int TileY;
    public Sprite Sprite;

    /// <summary>Scrolls the texture, for flowing water and conveyor belts.</summary>
    public Vector2 UvOffset;

    public Color Modulate;
}

/// <summary>
/// Collects the frame's terrain and emits it as world-aligned quads, grouped by texture.
/// </summary>
/// <remarks>
/// Unlike sprites, tiles are aligned to the world rather than the screen — they are the ground, so
/// they turn with it as the camera rotates.
///
/// All terrain shares a single depth well behind everything else, mirroring the original's fixed
/// pass order: the whole map is painted first, then objects on top of it. Tiles never overlap each
/// other, so they need no ordering among themselves.
/// </remarks>
public sealed class GroundDrawList
{
    /// <summary>Far enough behind that no object, however tall, can sort under the ground.</summary>
    private const float GroundSortBias = -4096f;

    private readonly Dictionary<Texture2D, List<GroundDraw>> _byTexture = new();
    private readonly List<Texture2D> _order = new();
    private readonly Dictionary<Texture2D, StandardMaterial3D> _materials = new();

    public void Clear()
    {
        foreach (var list in _byTexture.Values)
            list.Clear();
    }

    public void Add(in GroundDraw draw)
    {
        if (!draw.Sprite.IsValid)
            return;

        if (!_byTexture.TryGetValue(draw.Sprite.Sheet, out var list))
        {
            list = new List<GroundDraw>(1024);
            _byTexture[draw.Sprite.Sheet] = list;
            _order.Add(draw.Sprite.Sheet);
        }

        list.Add(draw);
    }

    public void Build(ImmediateMesh mesh, in WorldProjection projection)
    {
        mesh.ClearSurfaces();

        foreach (var texture in _order)
        {
            var draws = _byTexture[texture];
            if (draws.Count == 0)
                continue;

            mesh.SurfaceBegin(Mesh.PrimitiveType.Triangles);

            foreach (var draw in draws)
                Emit(mesh, draw, projection);

            mesh.SurfaceEnd();
            mesh.SurfaceSetMaterial(mesh.GetSurfaceCount() - 1, MaterialFor(texture));
        }
    }

    private static void Emit(ImmediateMesh mesh, in GroundDraw draw, in WorldProjection projection)
    {
        var uv = draw.Sprite.Uv;

        // Scrolling wraps within the tile's own region, so a flowing tile does not bleed into its
        // neighbours on the sheet.
        float u0 = uv.Position.X + draw.UvOffset.X * uv.Size.X;
        float v0 = uv.Position.Y + draw.UvOffset.Y * uv.Size.Y;
        float u1 = u0 + uv.Size.X;
        float v1 = v0 + uv.Size.Y;

        var topLeft = projection.ToScene(draw.TileX, draw.TileY, 0f, GroundSortBias);
        var topRight = projection.ToScene(draw.TileX + 1, draw.TileY, 0f, GroundSortBias);
        var bottomRight = projection.ToScene(draw.TileX + 1, draw.TileY + 1, 0f, GroundSortBias);
        var bottomLeft = projection.ToScene(draw.TileX, draw.TileY + 1, 0f, GroundSortBias);

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

    private StandardMaterial3D MaterialFor(Texture2D texture)
    {
        if (_materials.TryGetValue(texture, out var material))
            return material;

        material = new StandardMaterial3D
        {
            AlbedoTexture = texture,
            ShadingMode = BaseMaterial3D.ShadingModeEnum.Unshaded,
            TextureFilter = BaseMaterial3D.TextureFilterEnum.Nearest,
            // Terrain is fully opaque, so it needs no alpha handling at all.
            CullMode = BaseMaterial3D.CullModeEnum.Disabled,
            VertexColorUseAsAlbedo = true,
        };

        _materials[texture] = material;
        return material;
    }
}

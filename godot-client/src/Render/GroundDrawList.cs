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

    /// <summary>
    /// Surfaces are keyed by rectangle size as well as sheet, so the shader can take the size as a
    /// uniform. Every ground tile is eight pixels square, so in practice this is one group a sheet.
    /// </summary>
    private readonly record struct SurfaceKey(Texture2D Texture, Vector2I RegionSize);

    private readonly Dictionary<SurfaceKey, List<GroundDraw>> _bySurface = new();
    private readonly List<SurfaceKey> _order = new();
    private readonly Dictionary<SurfaceKey, ShaderMaterial> _materials = new();

    private static Shader _shader;

    public void Clear()
    {
        foreach (var list in _bySurface.Values)
            list.Clear();
    }

    public void Add(in GroundDraw draw)
    {
        if (!draw.Sprite.IsValid)
            return;

        var key = new SurfaceKey(draw.Sprite.Sheet, draw.Sprite.Region.Size);
        if (!_bySurface.TryGetValue(key, out var list))
        {
            list = new List<GroundDraw>(1024);
            _bySurface[key] = list;
            _order.Add(key);
        }

        list.Add(draw);
    }

    public void Build(ImmediateMesh mesh, in WorldProjection projection)
    {
        mesh.ClearSurfaces();

        foreach (var key in _order)
        {
            var draws = _bySurface[key];
            if (draws.Count == 0)
                continue;

            mesh.SurfaceBegin(Mesh.PrimitiveType.Triangles);

            foreach (var draw in draws)
                Emit(mesh, draw, projection);

            mesh.SurfaceEnd();
            mesh.SurfaceSetMaterial(mesh.GetSurfaceCount() - 1, MaterialFor(key));
        }
    }

    private static void Emit(ImmediateMesh mesh, in GroundDraw draw, in WorldProjection projection)
    {
        var uv = draw.Sprite.Uv;

        // The offset is applied here and wrapped in the shader, which is given the rectangle's
        // origin through UV2 so it knows what to wrap within.
        float u0 = uv.Position.X + draw.UvOffset.X * uv.Size.X;
        float v0 = uv.Position.Y + draw.UvOffset.Y * uv.Size.Y;
        float u1 = u0 + uv.Size.X;
        float v1 = v0 + uv.Size.Y;

        var topLeft = projection.ToScene(draw.TileX, draw.TileY, 0f, GroundSortBias);
        var topRight = projection.ToScene(draw.TileX + 1, draw.TileY, 0f, GroundSortBias);
        var bottomRight = projection.ToScene(draw.TileX + 1, draw.TileY + 1, 0f, GroundSortBias);
        var bottomLeft = projection.ToScene(draw.TileX, draw.TileY + 1, 0f, GroundSortBias);

        mesh.SurfaceSetColor(draw.Modulate);
        mesh.SurfaceSetUV2(uv.Position);

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

    private ShaderMaterial MaterialFor(SurfaceKey key)
    {
        if (_materials.TryGetValue(key, out var material))
            return material;

        _shader ??= ResourceLoader.Load<Shader>("res://shaders/ground.gdshader");

        material = new ShaderMaterial { Shader = _shader };
        material.SetShaderParameter("sheet", key.Texture);

        var sheetSize = key.Texture.GetSize();
        material.SetShaderParameter("region_size",
            new Vector2(key.RegionSize.X / sheetSize.X, key.RegionSize.Y / sheetSize.Y));

        _materials[key] = material;
        return material;
    }
}

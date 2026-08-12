using System;
using System.Collections.Generic;
using Godot;
using Hendra.Assets;

namespace Hendra.Render;

/// <summary>Whole-sprite colour treatments, matching the filters the original applied.</summary>
public enum SpriteTint
{
    None = 0,

    /// <summary>Greyscale. Paused, in stasis, or petrified.</summary>
    Greyscale = 1,

    /// <summary>Red. Cursed.</summary>
    Red = 2,
}

/// <summary>One screen-aligned quad queued for this frame.</summary>
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

    /// <summary>
    /// Where the quad's anchor sits vertically, as a fraction of its height measured up from the
    /// bottom. Zero stands the sprite on its tile; a half centres it, which is what a shadow wants.
    /// </summary>
    public float AnchorY;

    public bool Mirrored;

    /// <summary>Multiplied into the sampled colour. Carries fades and flashes.</summary>
    public Color Modulate;

    /// <summary>Whole-sprite colour treatment.</summary>
    public SpriteTint Tint;

    /// <summary>Whether this sprite should be outlined. Off for effects and particles.</summary>
    public bool Outlined;

    /// <summary>
    /// Spins the quad on screen, clockwise, in radians. Zero leaves it upright.
    /// </summary>
    /// <remarks>
    /// Only projectiles use this. A character never turns on screen -- it turns by being drawn from
    /// a different row of its sheet -- but a bullet is one sprite pointed wherever it is flying.
    /// The rotation is applied about the anchor and *after* the screen axes are chosen, so a
    /// spinning bullet keeps spinning at the same rate however the camera is turned.
    /// </remarks>
    public float Rotation;
}

/// <summary>
/// Collects the frame's sprites and emits them as screen-aligned quads, grouped by material.
/// </summary>
/// <remarks>
/// <para>
/// Sprites are horizontal quads rather than upright billboards. That falls out of the projection:
/// the camera looks straight down, so a quad lying flat in the ground plane is seen face-on and
/// maps directly to a screen rectangle. Orienting it by the camera yaw keeps it axis-aligned on
/// screen no matter which way the world is turned.
/// </para>
/// <para>
/// Grouping is the point of the exercise. The original appended a fill, a path and an end-fill to a
/// flat display list for every sprite, tile face, particle, health bar and name plate, then walked
/// the whole list again in its GPU path to pull out the entries that had to be drawn in software.
/// Here each sheet-and-treatment pair becomes one surface, so a crowded screen is a handful of
/// draws.
/// </para>
/// </remarks>
public sealed class SpriteDrawList
{
    /// <summary>
    /// How far the quad is grown, as a fraction of the sprite's size, to leave room for the
    /// outline.
    /// </summary>
    /// <remarks>
    /// The outline itself is measured in screen pixels by the shader, so this only has to be
    /// comfortably larger than the outline can ever be. Anything left over is discarded, and the
    /// sprite's own content is unaffected because the UV range grows by the same fraction.
    /// </remarks>
    private const float OutlinePadding = 0.12f;

/// <summary>
    /// What forces sprites apart into separate draw calls.
    /// </summary>
    /// <remarks>
    /// Sprite size is deliberately absent. It used to be here because the shader took the region
    /// size as a uniform, which meant every distinct sprite size on screen became its own surface --
    /// six hundred of them in a busy frame, each one a fresh GPU buffer every frame and, measured,
    /// the single largest cost in the frame. The size now rides along per vertex, so sprites only
    /// separate when they genuinely cannot share a draw: a different sheet, a different tint, or an
    /// outline.
    /// </remarks>
    private readonly record struct SurfaceKey(Texture2D Texture, SpriteTint Tint, bool Outlined);

    private readonly Dictionary<SurfaceKey, List<SpriteDraw>> _bySurface = new();
    private readonly List<SurfaceKey> _order = new();
    private readonly Dictionary<SurfaceKey, ShaderMaterial> _materials = new();

    private static Shader _shader;

    /// <summary>Surfaces the last build produced: the sprite half of the frame's draw calls.</summary>
    public int SurfaceCount { get; private set; }

    public void Clear()
    {
        foreach (var list in _bySurface.Values)
            list.Clear();
    }

    public void Add(in SpriteDraw draw)
    {
        if (!draw.Sprite.IsValid)
            return;

        var key = new SurfaceKey(draw.Sprite.Sheet, draw.Tint, draw.Outlined);
        if (!_bySurface.TryGetValue(key, out var list))
        {
            list = new List<SpriteDraw>(256);
            _bySurface[key] = list;
            _order.Add(key);
        }

        list.Add(draw);
    }

    /// <summary>Writes the queued quads into <paramref name="mesh"/>, one surface per material.</summary>
    /// <remarks>
    /// <para>
    /// Built into plain arrays and handed over one surface at a time, rather than fed vertex by
    /// vertex. That distinction is the whole performance story of this class: an ImmediateMesh
    /// takes each vertex, UV and colour through a separate call across the managed boundary, so a
    /// screen holding a few thousand sprites was making the better part of a million of them every
    /// frame and spending nearly all its time in the marshalling rather than the drawing. Filling
    /// a C# array costs nothing by comparison, and the whole surface crosses over once.
    /// </para>
    /// <para>
    /// The arrays are kept between frames and only ever grown, because the count is stable from one
    /// frame to the next and a per-frame allocation of this size would land in the garbage
    /// collector's path.
    /// </para>
    /// </remarks>
    public void Build(ArrayMesh mesh, in WorldProjection projection)
    {
        mesh.ClearSurfaces();

        // Screen right and screen down, expressed in the ground plane. Every quad is built from
        // these two, which is what keeps sprites upright on screen as the camera rotates.
        float sin = Mathf.Sin(projection.Angle);
        float cos = Mathf.Cos(projection.Angle);
        var right = new Vector3(cos, 0f, sin);
        var down = new Vector3(-sin, 0f, cos);

        SurfaceCount = 0;

        foreach (var key in _order)
        {
            var draws = _bySurface[key];
            if (draws.Count == 0)
                continue;

            // Four corners a quad, not six. Two of a triangle pair's corners are shared, and
            // sending them twice meant a third of everything crossing to the GPU each frame -- five
            // arrays' worth, for vertices the GPU already had. The index list says which corners
            // make which triangle and costs four bytes where a duplicated vertex costs sixty.
            int vertices = draws.Count * 4;
            int indices = draws.Count * 6;

            // Filled at exactly the right length and handed straight over. Godot wants arrays sized
            // to the vertex count, so a shared scratch buffer would have to be copied into a
            // right-sized one anyway -- this writes once instead of writing and then copying.
            _positions = new Vector3[vertices];
            _uvs = new Vector2[vertices];
            _uv2s = new Vector2[vertices];
            _colors = new Color[vertices];
            _extents = new float[vertices * 4];
            _indices = new int[indices];

            _at = 0;
            _index = 0;
            foreach (var draw in draws)
                Emit(draw, projection, right, down);

            var arrays = new Godot.Collections.Array();
            arrays.Resize((int)Mesh.ArrayType.Max);
            arrays[(int)Mesh.ArrayType.Vertex] = _positions;
            arrays[(int)Mesh.ArrayType.TexUV] = _uvs;
            arrays[(int)Mesh.ArrayType.TexUV2] = _uv2s;
            arrays[(int)Mesh.ArrayType.Color] = _colors;
            arrays[(int)Mesh.ArrayType.Custom0] = _extents;
            arrays[(int)Mesh.ArrayType.Index] = _indices;

            const Mesh.ArrayFormat custom0 =
                (Mesh.ArrayFormat)((uint)Mesh.ArrayCustomFormat.RgbaFloat
                                   << (int)Mesh.ArrayFormat.FormatCustom0Shift);

            mesh.AddSurfaceFromArrays(Mesh.PrimitiveType.Triangles, arrays, null, null, custom0);
            mesh.SurfaceSetMaterial(mesh.GetSurfaceCount() - 1, MaterialFor(key));
            SurfaceCount++;
        }
    }

    private Vector3[] _positions = new Vector3[6 * 256];
    private Vector2[] _uvs = new Vector2[6 * 256];
    private Vector2[] _uv2s = new Vector2[6 * 256];
    private Color[] _colors = new Color[6 * 256];

    /// <summary>Four floats a vertex, of which the shader reads the first two.</summary>
    private float[] _extents = new float[6 * 256 * 4];

    /// <summary>Which corners make which triangles: two per quad, sharing a diagonal.</summary>
    private int[] _indices = new int[6 * 256];

    private int _at;
    private int _index;

    private void Emit(
        in SpriteDraw draw,
        in WorldProjection projection,
        Vector3 right,
        Vector3 down)
    {
        var region = draw.Sprite.Uv;

        // The outline is drawn on transparent texels next to solid ones, so the quad has to reach
        // beyond the sprite for it to land anywhere. Both the geometry and the UV range grow by the
        // same fraction, and the shader treats anything outside the region as empty.
        float pad = draw.Outlined ? OutlinePadding : 0f;
        float padU = region.Size.X * pad;
        float padV = region.Size.Y * pad;

        float u0 = region.Position.X - padU;
        float u1 = region.Position.X + region.Size.X + padU;
        if (draw.Mirrored)
            (u0, u1) = (u1, u0);

        float v0 = region.Position.Y - padV;
        float v1 = region.Position.Y + region.Size.Y + padV;

        float padWidth = draw.WidthTiles * pad;
        float padHeight = draw.HeightTiles * pad;

        // The anchor sits on the ground point: horizontally by AnchorX, vertically at the bottom
        // edge, so a sprite stands on its tile rather than straddling it.
        var anchor = projection.ToScene(draw.TileX, draw.TileY, draw.Height, draw.SortBias);
        float left = -draw.WidthTiles * draw.AnchorX - padWidth;
        float rightEdge = left + draw.WidthTiles + 2f * padWidth;
        float top = -draw.HeightTiles * (1f - draw.AnchorY) - padHeight;
        float bottom = draw.HeightTiles * draw.AnchorY + padHeight;

        // A rotated sprite spins about its anchor, so the corners are turned in the screen plane
        // before being mapped onto the ground axes.
        if (draw.Rotation != 0f)
        {
            float spin = Mathf.Sin(draw.Rotation);
            float spun = Mathf.Cos(draw.Rotation);
            var turnedRight = right * spun + down * spin;
            var turnedDown = down * spun - right * spin;
            right = turnedRight;
            down = turnedDown;
        }

        var topLeft = anchor + right * left + down * top;
        var topRight = anchor + right * rightEdge + down * top;
        var bottomRight = anchor + right * rightEdge + down * bottom;
        var bottomLeft = anchor + right * left + down * bottom;

        // The origin of this sprite's rectangle within the sheet; its size comes from a uniform,
        // since surfaces are grouped by size anyway.
        var origin = region.Position;
        var extent = region.Size;
        var tint = draw.Modulate;

        int corner = _at;

        Vertex(topLeft, u0, v0, origin, extent, tint);
        Vertex(topRight, u1, v0, origin, extent, tint);
        Vertex(bottomRight, u1, v1, origin, extent, tint);
        Vertex(bottomLeft, u0, v1, origin, extent, tint);

        _indices[_index] = corner;
        _indices[_index + 1] = corner + 1;
        _indices[_index + 2] = corner + 2;
        _indices[_index + 3] = corner;
        _indices[_index + 4] = corner + 2;
        _indices[_index + 5] = corner + 3;
        _index += 6;
    }

    private void Vertex(Vector3 position, float u, float v, Vector2 origin, Vector2 extent, Color tint)
    {
        _positions[_at] = position;
        _uvs[_at] = new Vector2(u, v);
        _uv2s[_at] = origin;
        _colors[_at] = tint;

        int custom = _at * 4;
        _extents[custom] = extent.X;
        _extents[custom + 1] = extent.Y;
        _extents[custom + 2] = 0f;
        _extents[custom + 3] = 0f;

        _at++;
    }

    private ShaderMaterial MaterialFor(SurfaceKey key)
    {
        if (_materials.TryGetValue(key, out var material))
            return material;

        _shader ??= ResourceLoader.Load<Shader>("res://shaders/sprite.gdshader");

        material = new ShaderMaterial { Shader = _shader };
        material.SetShaderParameter("sheet", key.Texture);

        // No region size here any more: it travels per vertex, which is what lets every sprite on
        // a sheet share one surface regardless of how big it is.
        material.SetShaderParameter("tint_mode", (int)key.Tint);
        material.SetShaderParameter("outline_pixels", key.Outlined ? 2.0f : 0f);

        _materials[key] = material;
        return material;
    }
}

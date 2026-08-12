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

    private readonly record struct SurfaceKey(Texture2D Texture, SpriteTint Tint, bool Outlined, Vector2I RegionSize);

    private readonly Dictionary<SurfaceKey, List<SpriteDraw>> _bySurface = new();
    private readonly List<SurfaceKey> _order = new();
    private readonly Dictionary<SurfaceKey, ShaderMaterial> _materials = new();

    private static Shader _shader;

    public void Clear()
    {
        foreach (var list in _bySurface.Values)
            list.Clear();
    }

    public void Add(in SpriteDraw draw)
    {
        if (!draw.Sprite.IsValid)
            return;

        var key = new SurfaceKey(draw.Sprite.Sheet, draw.Tint, draw.Outlined, draw.Sprite.Region.Size);
        if (!_bySurface.TryGetValue(key, out var list))
        {
            list = new List<SpriteDraw>(256);
            _bySurface[key] = list;
            _order.Add(key);
        }

        list.Add(draw);
    }

    /// <summary>Writes the queued quads into <paramref name="mesh"/>, one surface per material.</summary>
    public void Build(ImmediateMesh mesh, in WorldProjection projection)
    {
        mesh.ClearSurfaces();

        // Screen right and screen down, expressed in the ground plane. Every quad is built from
        // these two, which is what keeps sprites upright on screen as the camera rotates.
        float sin = Mathf.Sin(projection.Angle);
        float cos = Mathf.Cos(projection.Angle);
        var right = new Vector3(cos, 0f, sin);
        var down = new Vector3(-sin, 0f, cos);

        foreach (var key in _order)
        {
            var draws = _bySurface[key];
            if (draws.Count == 0)
                continue;

            mesh.SurfaceBegin(Mesh.PrimitiveType.Triangles);

            foreach (var draw in draws)
                Emit(mesh, draw, projection, right, down);

            mesh.SurfaceEnd();
            mesh.SurfaceSetMaterial(mesh.GetSurfaceCount() - 1, MaterialFor(key));
        }
    }

    private static void Emit(
        ImmediateMesh mesh,
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

        mesh.SurfaceSetColor(draw.Modulate);

        // The origin of this sprite's rectangle within the sheet; its size comes from a uniform,
        // since surfaces are grouped by size anyway.
        mesh.SurfaceSetUV2(region.Position);

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

        _shader ??= ResourceLoader.Load<Shader>("res://shaders/sprite.gdshader");

        material = new ShaderMaterial { Shader = _shader };
        material.SetShaderParameter("sheet", key.Texture);

        var textureSize = key.Texture.GetSize();
        material.SetShaderParameter("region_size",
            new Vector2(key.RegionSize.X / textureSize.X, key.RegionSize.Y / textureSize.Y));
        material.SetShaderParameter("tint_mode", (int)key.Tint);
        material.SetShaderParameter("outline_pixels", key.Outlined ? 2.0f : 0f);

        _materials[key] = material;
        return material;
    }
}

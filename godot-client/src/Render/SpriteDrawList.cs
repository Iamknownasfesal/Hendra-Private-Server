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

    /// <summary>
    /// Turns the queued sprites into one batch of instances per surface.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Every sprite is the same unit quad; what differs is where it lands, how big it is, which
    /// rectangle of the sheet it shows and what colour it is tinted. All four fit in a
    /// MultiMesh instance -- twelve floats of transform, four of colour, four naming the
    /// rectangle -- against roughly sixty floats for the four corners it used to send. The
    /// instance buffer is kept between frames and only grown, so a steady state uploads without
    /// allocating, where the vertex arrays had to be built fresh at exactly the vertex count and
    /// were several megabytes a frame of garbage.
    /// </para>
    /// <para>
    /// Depth ordering is unaffected: it has always ridden on the position handed back by the
    /// projection, which is now the instance's origin rather than its corners.
    /// </para>
    /// </remarks>
    public void Build(SpriteBatches batches, in WorldProjection projection)
    {
        // Screen right and screen down, expressed in the ground plane. Every quad is built from
        // these two, which is what keeps sprites upright on screen as the camera rotates.
        float sin = Mathf.Sin(projection.Angle);
        float cos = Mathf.Cos(projection.Angle);
        var right = new Vector3(cos, 0f, sin);
        var down = new Vector3(-sin, 0f, cos);

        SurfaceCount = 0;
        batches.Begin();

        foreach (var key in _order)
        {
            var draws = _bySurface[key];
            if (draws.Count == 0)
                continue;

            int needed = draws.Count * FloatsPerInstance;
            if (_instances.Length < needed)
            {
                int size = _instances.Length;
                while (size < needed)
                    size *= 2;

                _instances = new float[size];
            }

            _at = 0;
            foreach (var draw in draws)
                Emit(draw, projection, right, down);

            batches.Submit(key.Outlined, MaterialFor(key), _instances, draws.Count);
            SurfaceCount++;
        }

        batches.End();
    }

    /// <summary>Twelve of transform, four of colour, four naming the sheet rectangle.</summary>
    private const int FloatsPerInstance = 20;

    private float[] _instances = new float[256 * FloatsPerInstance];
    private int _at;

    /// <summary>Places one sprite as an instance of the shared quad.</summary>
    /// <remarks>
    /// The quad spans nought to one in both directions, so the transform's two in-plane axes are
    /// the sprite's own width and height along screen right and screen down, and its origin is the
    /// sprite's top-left corner. An outlined sprite's quad reaches a little beyond that on every
    /// side, which the base mesh carries rather than the transform, so the size here is the
    /// sprite's own either way.
    /// </remarks>
    private void Emit(
        in SpriteDraw draw,
        in WorldProjection projection,
        Vector3 right,
        Vector3 down)
    {
        var region = draw.Sprite.Uv;

        // A rotated sprite spins about its anchor, so the screen axes are turned before the quad is
        // laid on them.
        if (draw.Rotation != 0f)
        {
            float spin = Mathf.Sin(draw.Rotation);
            float spun = Mathf.Cos(draw.Rotation);
            var turnedRight = right * spun + down * spin;
            var turnedDown = down * spun - right * spin;
            right = turnedRight;
            down = turnedDown;
        }

        // The anchor sits on the ground point: horizontally by AnchorX, vertically at the bottom
        // edge, so a sprite stands on its tile rather than straddling it.
        var anchor = projection.ToScene(draw.TileX, draw.TileY, draw.Height, draw.SortBias);
        float left = -draw.WidthTiles * draw.AnchorX;
        float top = -draw.HeightTiles * (1f - draw.AnchorY);

        // Mirroring flips the quad across its own vertical. The far edge becomes the origin and the
        // width runs back towards the near one, so the sprite covers the same ground and only its
        // artwork turns round -- starting from the same corner and negating the width would slide
        // it a whole width sideways, which is every character that walks left.
        float start = draw.Mirrored ? left + draw.WidthTiles : left;

        var origin = anchor + right * start + down * top;
        var axisX = (draw.Mirrored ? -right : right) * draw.WidthTiles;
        var axisY = down * draw.HeightTiles;

        // Godot lays a Transform3D out as three rows of four: the basis columns, then the origin.
        int at = _at;
        _instances[at] = axisX.X;
        _instances[at + 1] = axisY.X;
        _instances[at + 2] = 0f;
        _instances[at + 3] = origin.X;

        _instances[at + 4] = axisX.Y;
        _instances[at + 5] = axisY.Y;
        _instances[at + 6] = 1f;
        _instances[at + 7] = origin.Y;

        _instances[at + 8] = axisX.Z;
        _instances[at + 9] = axisY.Z;
        _instances[at + 10] = 0f;
        _instances[at + 11] = origin.Z;

        var tint = draw.Modulate;
        _instances[at + 12] = tint.R;
        _instances[at + 13] = tint.G;
        _instances[at + 14] = tint.B;
        _instances[at + 15] = tint.A;

        _instances[at + 16] = region.Position.X;
        _instances[at + 17] = region.Position.Y;
        _instances[at + 18] = region.Size.X;
        _instances[at + 19] = region.Size.Y;

        _at = at + FloatsPerInstance;
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

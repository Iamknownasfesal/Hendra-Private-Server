using System.Collections.Generic;
using Godot;

namespace Hendra.Assets;

/// <summary>
/// A sprite with its colour taken out: the shape, and nothing else.
/// </summary>
/// <remarks>
/// <para>
/// For marks rather than for items. The vault's filter rail wants a sword to stand for weapons, and
/// the sword the game already has is the right shape and the wrong picture -- a wooden hilt, a steel
/// highlight and a gold pommel, all of it fighting for room in a thirty-two pixel square next to six
/// of its neighbours. Flattened to one tone it stops being a picture of a particular sword and
/// becomes the idea of one.
/// </para>
/// <para>
/// Baked into a texture rather than tinted at draw time, because a tint multiplies: a modulate
/// cannot lift a black pixel to grey, so the shape would keep its own shading no matter what colour
/// it was asked for. Here every solid texel becomes white, and whoever draws it picks the colour by
/// modulating that -- which is also what lets the rail's mark brighten when its filter is on.
/// </para>
/// <para>
/// Cached by sheet and rectangle. There are eight of these in the whole interface and they are
/// built the first time the panel is, so the cache is a handful of eight-pixel textures and never
/// needs evicting.
/// </para>
/// </remarks>
public static class SpriteSilhouette
{
    /// <summary>How opaque a texel must be to count as part of the shape.</summary>
    private const float Solid = 0.5f;

    private static readonly Dictionary<(Texture2D, Rect2I), Texture2D> Made = new();

    /// <summary>The sprite's shape in white, or null if it has none to give.</summary>
    public static Texture2D Of(in Sprite sprite)
    {
        if (!sprite.IsValid || sprite.Region.Size.X <= 0 || sprite.Region.Size.Y <= 0)
            return null;

        var key = (sprite.Sheet, sprite.Region);
        if (Made.TryGetValue(key, out var already))
            return already;

        var sheet = sprite.Sheet.GetImage();
        if (sheet == null)
            return null;

        if (sheet.IsCompressed())
            sheet.Decompress();

        var shape = sheet.GetRegion(sprite.Region);
        shape.Convert(Image.Format.Rgba8);

        for (int y = 0; y < shape.GetHeight(); y++)
            for (int x = 0; x < shape.GetWidth(); x++)
                shape.SetPixel(x, y, shape.GetPixel(x, y).A >= Solid ? Colors.White : Colors.Transparent);

        var texture = ImageTexture.CreateFromImage(shape);
        Made[key] = texture;
        return texture;
    }
}

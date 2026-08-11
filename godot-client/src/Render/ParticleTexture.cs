using Godot;
using Hendra.Assets;

namespace Hendra.Render;

/// <summary>
/// The single texture every particle is drawn with: a white square inside a dark glow.
/// </summary>
/// <remarks>
/// <para>
/// The original baked one bitmap per colour and size combination — a square filled with the colour,
/// run through a glow filter, and kept in a two-level dictionary that was never cleared. A screen of
/// mixed effects walks through a great many of those.
/// </para>
/// <para>
/// One texture serves for all of them because the colour arrives as a vertex tint. The trick is that
/// the glow is black, and black survives being multiplied by any tint, so the same texels give a
/// green particle a dark edge and a red one a dark edge without either bleeding into the other.
/// </para>
/// <para>
/// The one divergence: the original padded every particle with a fixed four pixels of glow whatever
/// its size, and a single texture cannot express a constant pad and a variable core. Here the glow
/// scales with the particle, which at the sizes actually used — a few pixels across — is the same
/// picture.
/// </para>
/// </remarks>
public static class ParticleTexture
{
    private const int Size = 16;

    /// <summary>Width of the solid core, in texels.</summary>
    private const int Core = 10;

    private const float GlowAlpha = 0.8f;

    /// <summary>Softness of the glow's outer edge, in texels.</summary>
    private const float Falloff = 2.5f;

    /// <summary>
    /// How much wider the quad is than the particle's own size, to leave room for the glow.
    /// </summary>
    public const float QuadToCore = (float)Size / Core;

    private static Texture2D _texture;

    /// <summary>The sprite to hand the draw list. Built once, on first use.</summary>
    public static Sprite Sprite => new(_texture ??= Build(), new Rect2I(0, 0, Size, Size));

    private static Texture2D Build()
    {
        var image = Image.CreateEmpty(Size, Size, false, Image.Format.Rgba8);

        const int inset = (Size - Core) / 2;
        const int last = inset + Core - 1;

        for (int y = 0; y < Size; y++)
        {
            for (int x = 0; x < Size; x++)
            {
                // Distance out of the core rectangle, zero anywhere inside it.
                float dx = Mathf.Max(Mathf.Max(inset - x, x - last), 0);
                float dy = Mathf.Max(Mathf.Max(inset - y, y - last), 0);
                float distance = Mathf.Sqrt(dx * dx + dy * dy);

                var colour = distance <= 0f
                    ? new Color(1f, 1f, 1f, 1f)
                    : new Color(0f, 0f, 0f, GlowAlpha * Mathf.Clamp(1f - distance / Falloff, 0f, 1f));

                image.SetPixel(x, y, colour);
            }
        }

        return ImageTexture.CreateFromImage(image);
    }
}

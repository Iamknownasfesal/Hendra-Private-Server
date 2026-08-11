using System.Collections.Generic;
using Godot;
using Hendra.Assets;
using Hendra.Resources;

namespace Hendra.Render;

/// <summary>
/// One representative colour per terrain type, for the minimap.
/// </summary>
/// <remarks>
/// Most terrain declares a colour in XML. The rest have it derived from their artwork by taking the
/// most common opaque pixel, which is what the original did — and is why water reads as blue on the
/// minimap without anyone having written that down anywhere.
///
/// Derived once per type and kept. There are a few hundred types and the scan is over an 8x8 tile,
/// so the whole table costs less than a single frame.
/// </remarks>
public sealed class TileColors
{
    private readonly Dictionary<ushort, Color> _cache = new();
    private readonly Dictionary<Texture2D, Image> _images = new();
    private readonly AssetLibrary _assets;

    public TileColors(AssetLibrary assets)
    {
        _assets = assets;
    }

    public Color Get(GroundDesc desc)
    {
        if (desc == null)
            return Colors.Black;

        if (_cache.TryGetValue(desc.Type, out var cached))
            return cached;

        var colour = Derive(desc);
        _cache[desc.Type] = colour;
        return colour;
    }

    private Color Derive(GroundDesc desc)
    {
        if (desc.Color >= 0)
        {
            return new Color(
                (desc.Color >> 16 & 0xFF) / 255f,
                (desc.Color >> 8 & 0xFF) / 255f,
                (desc.Color & 0xFF) / 255f);
        }

        var spec = desc.Texture;

        // A random-variant tile is representative enough in its first variant.
        if (spec is { Kind: TextureKind.Random, Variants.Count: > 0 })
            spec = spec.Variants[0];

        if (spec == null || spec.File == null)
            return Colors.Black;

        var sprite = _assets.GetSprite(spec.File, spec.Index);
        return sprite.IsValid ? MostCommonColour(sprite) : Colors.Black;
    }

    /// <summary>The most frequent fully-opaque colour in the sprite, or black if it has none.</summary>
    private Color MostCommonColour(Sprite sprite)
    {
        if (!_images.TryGetValue(sprite.Sheet, out var image))
        {
            image = sprite.Sheet.GetImage();
            _images[sprite.Sheet] = image;
        }

        if (image == null)
            return Colors.Black;

        var counts = new Dictionary<uint, int>();
        int right = Mathf.Min(sprite.Region.Position.X + sprite.Region.Size.X, image.GetWidth());
        int bottom = Mathf.Min(sprite.Region.Position.Y + sprite.Region.Size.Y, image.GetHeight());

        for (int y = sprite.Region.Position.Y; y < bottom; y++)
        {
            for (int x = sprite.Region.Position.X; x < right; x++)
            {
                var pixel = image.GetPixel(x, y);
                if (pixel.A < 0.5f)
                    continue;

                uint key = pixel.ToRgba32();
                counts[key] = counts.GetValueOrDefault(key) + 1;
            }
        }

        uint best = 0;
        int bestCount = 0;
        foreach (var (colour, count) in counts)
        {
            if (count <= bestCount)
                continue;
            bestCount = count;
            best = colour;
        }

        return bestCount == 0
            ? Colors.Black
            : new Color(
                (best >> 24 & 0xFF) / 255f,
                (best >> 16 & 0xFF) / 255f,
                (best >> 8 & 0xFF) / 255f);
    }
}

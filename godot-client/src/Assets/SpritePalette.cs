using System;
using System.Collections.Generic;
using Godot;

namespace Hendra.Assets;

/// <summary>
/// The colours an object throws off when it is hit: its own pixels, with some proportion replaced
/// by the blood colour its definition names.
/// </summary>
/// <remarks>
/// <para>
/// The original called this a blood composition, and it did the work on every single hit: it walked
/// the whole sprite pixel by pixel, built a colour list, then built a second randomised list from
/// that. It meant to cache the result — there is a dictionary keyed by object type, and it is read
/// on the way in — but nothing ever writes to it, so a monster under sustained fire rescanned its
/// own texture several times a second. Caching it as intended is the entire fix.
/// </para>
/// <para>
/// The scan itself has to read the texture back from the GPU, which is slow enough to be worth
/// doing once per sheet rather than once per sprite, so decoded sheets are held too.
/// </para>
/// </remarks>
public sealed class SpritePalette
{
    /// <summary>
    /// A ceiling on how many colours are kept per object.
    /// </summary>
    /// <remarks>
    /// Sprites are usually eight pixels square, so this never bites for a character. It exists for
    /// the handful of large decorations, where a thousand-entry list would be sampled at random
    /// anyway and a hundred is indistinguishable.
    /// </remarks>
    private const int MaxColors = 128;

    private static readonly int[] Empty = Array.Empty<int>();

    private readonly Dictionary<ushort, int[]> _byObjectType = new();
    private readonly Dictionary<Texture2D, Image> _decodedSheets = new();
    private readonly Random _random = new();

    /// <summary>
    /// The palette for an object type, sampled from its sprite the first time it is asked for.
    /// </summary>
    /// <param name="bloodProbability">
    /// How much of the palette is replaced by <paramref name="bloodColor"/>. An object with no
    /// declared probability sprays its own colours only, which is what makes a shattering crystal
    /// look different from a bleeding animal.
    /// </param>
    public IReadOnlyList<int> For(ushort objectType, in Sprite sprite, float bloodProbability, int bloodColor)
    {
        if (_byObjectType.TryGetValue(objectType, out var cached))
            return cached;

        var colors = Sample(sprite);
        if (colors.Count == 0)
        {
            _byObjectType[objectType] = Empty;
            return Empty;
        }

        var palette = new int[colors.Count];
        for (int i = 0; i < palette.Length; i++)
        {
            palette[i] = _random.NextDouble() < bloodProbability
                ? bloodColor
                : colors[_random.Next(colors.Count)];
        }

        _byObjectType[objectType] = palette;
        return palette;
    }

    private List<int> Sample(in Sprite sprite)
    {
        var colors = new List<int>(64);
        if (!sprite.IsValid)
            return colors;

        var image = Decoded(sprite.Sheet);
        if (image == null)
            return colors;

        var region = sprite.Region;
        int right = Math.Min(region.Position.X + region.Size.X, image.GetWidth());
        int bottom = Math.Min(region.Position.Y + region.Size.Y, image.GetHeight());

        // Stepped rather than exhaustive on anything large, so a two-hundred-pixel decoration costs
        // the same as an eight-pixel monster. The colours are sampled at random afterwards, so an
        // even subset is as good as the whole.
        int stepX = Math.Max(1, region.Size.X / 16);
        int stepY = Math.Max(1, region.Size.Y / 16);

        for (int y = Math.Max(0, region.Position.Y); y < bottom; y += stepY)
        {
            for (int x = Math.Max(0, region.Position.X); x < right; x += stepX)
            {
                var pixel = image.GetPixel(x, y);
                if (pixel.A <= 0f)
                    continue;

                colors.Add(((int)(pixel.R8) << 16) | ((int)(pixel.G8) << 8) | pixel.B8);

                if (colors.Count >= MaxColors)
                    return colors;
            }
        }

        return colors;
    }

    /// <summary>
    /// The sheet's pixels, decompressed if the importer left them in a GPU format.
    /// </summary>
    /// <remarks>
    /// Held for the life of the session. Sheets are small and few, and the alternative is stalling
    /// the render thread to read one back the first time each new kind of monster is hit.
    /// </remarks>
    private Image Decoded(Texture2D sheet)
    {
        if (_decodedSheets.TryGetValue(sheet, out var cached))
            return cached;

        var image = sheet.GetImage();

        if (image != null && image.IsCompressed() && image.Decompress() != Error.Ok)
            image = null;

        _decodedSheets[sheet] = image;
        return image;
    }
}

using System;
using System.Collections.Generic;
using Godot;
using Hendra.Assets;
using Hendra.Resources;
using Hendra.World;

namespace Hendra.Render;

/// <summary>
/// The nine terrain types around a tile, after priority substitution. Identifies a blend result.
/// </summary>
/// <remarks>
/// Row-major from the north-west corner, so index 4 is the tile itself. Any neighbour that does not
/// outrank the centre is recorded as the centre's own type, which is what makes the signature
/// describe the *appearance* rather than the layout: two different arrangements that look the same
/// share a baked tile.
/// </remarks>
public readonly struct BlendSignature : IEquatable<BlendSignature>
{
    private readonly ulong _low;
    private readonly ulong _high;

    public BlendSignature(ReadOnlySpan<ushort> types)
    {
        // Nine 16-bit values packed into two words, so the key hashes and compares without
        // allocating. The original used the array's string form as a dictionary key.
        _low = types[0] | ((ulong)types[1] << 16) | ((ulong)types[2] << 32) | ((ulong)types[3] << 48);
        _high = types[4] | ((ulong)types[5] << 16) | ((ulong)types[6] << 32) | ((ulong)types[7] << 48);
        Extra = types[8];
    }

    public ushort Extra { get; }

    public bool Equals(BlendSignature other) =>
        _low == other._low && _high == other._high && Extra == other.Extra;

    public override bool Equals(object obj) => obj is BlendSignature other && Equals(other);

    public override int GetHashCode() => HashCode.Combine(_low, _high, Extra);

    /// <summary>A stable value derived from the signature, used where the original drew at random.</summary>
    public int Seed => (int)(_low ^ (_low >> 32) ^ _high ^ (_high >> 32) ^ Extra) & 0x7FFFFFFF;
}

/// <summary>
/// Blends terrain edges, porting the original's TileRedrawer.
/// </summary>
/// <remarks>
/// <para>
/// Terrain does not simply butt up against its neighbours: where two types meet, the higher-priority
/// one bleeds a few pixels into the lower. The original did this by compositing 8x8 tiles out of
/// four 4x4 quadrants, each masked by one of five hand-drawn shapes chosen by which sides differ,
/// and rotated per quadrant. Sand meeting grass looks like sand meeting grass because of this, and
/// without it every terrain boundary is a hard pixel step.
/// </para>
/// <para>
/// The results are baked once per distinct neighbourhood into a shared atlas, so the whole map
/// costs a few dozen 8x8 composites rather than one per tile. The original cached them too, in an
/// unbounded dictionary keyed by the stringified signature array.
/// </para>
/// <para>
/// Where the original picked a random mask variant per call, this derives the choice from the
/// signature. Same visual variety, but a given neighbourhood looks the same on every run and in
/// every session, which matters because the result is cached: a random pick would be frozen at
/// first sight anyway.
/// </para>
/// </remarks>
public sealed class TileBlender
{
    private const int TileSize = 8;
    private const int Half = TileSize / 2;

    // Mask roles, in the order the original's per-quadrant vector holds them.
    private const int Inner = 0;
    private const int Side0 = 1;
    private const int Side1 = 2;
    private const int Outer = 3;
    private const int InnerP1 = 4;
    private const int InnerP2 = 5;

    /// <summary>Quarter-turn rotation applied to each quadrant's masks. Straight from the original.</summary>
    private static readonly int[] QuadrantRotation = { -1, 0, 2, 1 };

    /// <summary>Destination corner of each quadrant.</summary>
    private static readonly Vector2I[] QuadrantOrigin =
    {
        new(0, 0), new(Half, 0), new(0, Half), new(Half, Half),
    };

    /// <summary>
    /// For each quadrant: the two neighbouring sides and the diagonal between them, as indices into
    /// the signature. Order matters — the first is the "A" side the mask rotations assume.
    /// </summary>
    private static readonly (int SideA, int Corner, int SideB)[] QuadrantNeighbours =
    {
        (3, 0, 1), // north-west: west, north-west, north
        (1, 2, 5), // north-east: north, north-east, east
        (7, 6, 3), // south-west: south, south-west, west
        (5, 8, 7), // south-east: east, south-east, south
    };

    private readonly GameData _data;
    private readonly AssetLibrary _assets;
    private readonly TileAtlas _atlas;

    // [quadrant][role] -> the variants of that mask, already rotated for that quadrant.
    private readonly Image[][][] _masks = new Image[4][][];

    private readonly Dictionary<ushort, Image> _baseTiles = new();
    private readonly Dictionary<Texture2D, Image> _sheets = new();
    private readonly Dictionary<BlendSignature, Sprite> _baked = new();

    private readonly ushort[] _signature = new ushort[9];

    public TileBlender(GameData data, AssetLibrary assets, TileAtlas atlas)
    {
        _data = data;
        _assets = assets;
        _atlas = atlas;
        BuildMasks();
    }

    /// <summary>Whether the masks loaded. Without them, blending is skipped rather than crashing.</summary>
    public bool IsReady { get; private set; }

    private void BuildMasks()
    {
        var inner = LoadMaskSet("inner_mask");
        var sides = LoadMaskSet("sides_mask");
        var outer = LoadMaskSet("outer_mask");
        var innerP1 = LoadMaskSet("innerP1_mask");
        var innerP2 = LoadMaskSet("innerP2_mask");

        if (inner == null || sides == null || outer == null || innerP1 == null || innerP2 == null)
            return;

        for (int quadrant = 0; quadrant < 4; quadrant++)
        {
            int rotation = QuadrantRotation[quadrant];
            _masks[quadrant] = new[]
            {
                Rotate(inner, rotation),
                // The two side masks are the same artwork a quarter turn apart, which is how one
                // drawing serves both edges of a corner.
                Rotate(sides, rotation - 1),
                Rotate(sides, rotation),
                Rotate(outer, rotation),
                Rotate(innerP1, rotation),
                Rotate(innerP2, rotation),
            };
        }

        IsReady = true;
    }

    private Image[] LoadMaskSet(string name)
    {
        var sheet = _assets.GetSheet(name);
        if (sheet == null)
            return null;

        var variants = new List<Image>(sheet.Count);
        for (int i = 0; i < sheet.Count; i++)
        {
            var sprite = sheet[i];
            if (sprite.IsValid)
                variants.Add(Crop(sprite));
        }

        return variants.Count == 0 ? null : variants.ToArray();
    }

    private static Image[] Rotate(Image[] set, int quarterTurns)
    {
        var rotated = new Image[set.Length];
        for (int i = 0; i < set.Length; i++)
            rotated[i] = Rotate(set[i], quarterTurns);
        return rotated;
    }

    private static Image Rotate(Image source, int quarterTurns)
    {
        int turns = ((quarterTurns % 4) + 4) % 4;
        if (turns == 0)
            return source;

        int size = source.GetWidth();
        var result = Image.CreateEmpty(size, size, false, Image.Format.Rgba8);

        for (int y = 0; y < size; y++)
        {
            for (int x = 0; x < size; x++)
            {
                var (sx, sy) = turns switch
                {
                    1 => (y, size - 1 - x),
                    2 => (size - 1 - x, size - 1 - y),
                    _ => (size - 1 - y, x),
                };
                result.SetPixel(x, y, source.GetPixel(sx, sy));
            }
        }

        return result;
    }

    /// <summary>
    /// Produces the blended artwork for a tile, or an invalid sprite when it needs none.
    /// </summary>
    public Sprite Blend(GameMap map, int tileX, int tileY, Square square)
    {
        if (!IsReady || square?.Desc == null)
            return default;

        // Edge and composite terrain use different schemes that are not implemented; they fall back
        // to their plain artwork rather than being blended wrongly.
        if (square.Desc.HasEdge || square.TileType == 253)
            return default;

        if (!BuildSignature(map, tileX, tileY, square))
            return default;

        var signature = new BlendSignature(_signature);
        if (_baked.TryGetValue(signature, out var cached))
            return cached;

        var sprite = Bake(signature, square.TileType);
        _baked[signature] = sprite;
        return sprite;
    }

    /// <summary>
    /// Fills the signature buffer. Returns false when nothing around the tile outranks it, in which
    /// case there is nothing to blend.
    /// </summary>
    private bool BuildSignature(GameMap map, int tileX, int tileY, Square square)
    {
        ushort self = square.TileType;
        int priority = square.Desc.BlendPriority;
        bool anyDifferent = false;

        int index = 0;
        for (int dy = -1; dy <= 1; dy++)
        {
            for (int dx = -1; dx <= 1; dx++, index++)
            {
                if (dx == 0 && dy == 0)
                {
                    _signature[index] = self;
                    continue;
                }

                var neighbour = map.GetSquare(tileX + dx, tileY + dy);

                // A neighbour that does not outrank us leaves no mark, and neither does one we have
                // not been told about yet.
                if (neighbour?.Desc == null || neighbour.Desc.BlendPriority <= priority)
                {
                    _signature[index] = self;
                    continue;
                }

                _signature[index] = neighbour.TileType;
                anyDifferent = true;
            }
        }

        return anyDifferent;
    }

    private Sprite Bake(BlendSignature signature, ushort baseType)
    {
        var baseTile = GetBaseTile(baseType);
        if (baseTile == null)
            return default;

        var result = Image.CreateEmpty(TileSize, TileSize, false, Image.Format.Rgba8);
        result.BlitRect(baseTile, new Rect2I(0, 0, TileSize, TileSize), Vector2I.Zero);

        ushort self = _signature[4];
        var random = new Random(signature.Seed);

        for (int quadrant = 0; quadrant < 4; quadrant++)
        {
            var (sideAIndex, cornerIndex, sideBIndex) = QuadrantNeighbours[quadrant];
            ushort sideA = _signature[sideAIndex];
            ushort sideB = _signature[sideBIndex];
            ushort corner = _signature[cornerIndex];

            // A quadrant only needs work if one of the two sides it touches differs, or -- when both
            // sides match -- if the diagonal between them does.
            bool needed = sideA != self || sideB != self || corner != self;
            if (!needed)
                continue;

            DrawQuadrant(result, quadrant, self, sideA, corner, sideB, random);
        }

        return _atlas.Add(result);
    }

    /// <summary>
    /// Composites one quadrant, choosing the mask from which of its neighbours differ.
    /// </summary>
    /// <remarks>
    /// The five cases are the original's, and each corresponds to a shape a boundary can take
    /// through a quarter tile: a corner poking in, a corner cut off, one straight edge or the other,
    /// or two different terrains meeting diagonally.
    /// </remarks>
    private void DrawQuadrant(
        Image destination, int quadrant, ushort self, ushort sideA, ushort corner, ushort sideB, Random random)
    {
        var masks = _masks[quadrant];
        var origin = QuadrantOrigin[quadrant];

        bool differsA = sideA != self;
        bool differsB = sideB != self;

        if (!differsA && !differsB)
        {
            // Both sides match, so only the diagonal is intruding: a small corner of it shows.
            Composite(destination, GetBaseTile(corner), Pick(masks[Outer], random), origin);
            return;
        }

        if (differsA && differsB)
        {
            if (sideA != sideB)
            {
                // Two different terrains meeting here, each taking half the quadrant.
                Composite(destination, GetBaseTile(sideA), Pick(masks[InnerP1], random), origin);
                Composite(destination, GetBaseTile(sideB), Pick(masks[InnerP2], random), origin);
                return;
            }

            // The same terrain on both sides: it wraps around the corner.
            Composite(destination, GetBaseTile(sideA), Pick(masks[Inner], random), origin);
            return;
        }

        // Exactly one side differs, giving a straight edge.
        if (differsA)
            Composite(destination, GetBaseTile(sideA), Pick(masks[Side0], random), origin);
        else
            Composite(destination, GetBaseTile(sideB), Pick(masks[Side1], random), origin);
    }

    private static Image Pick(Image[] variants, Random random) =>
        variants.Length == 1 ? variants[0] : variants[random.Next(variants.Length)];

    /// <summary>
    /// Blends one quadrant of <paramref name="source"/> over <paramref name="destination"/>, using
    /// the mask's alpha.
    /// </summary>
    /// <remarks>
    /// The mask is the whole point: its soft edge is what makes one terrain appear to seep into the
    /// next rather than stopping at a straight line.
    /// </remarks>
    private static void Composite(Image destination, Image source, Image mask, Vector2I origin)
    {
        if (source == null || mask == null)
            return;

        for (int y = 0; y < Half; y++)
        {
            for (int x = 0; x < Half; x++)
            {
                float alpha = mask.GetPixel(x, y).A;
                if (alpha <= 0f)
                    continue;

                int dx = origin.X + x;
                int dy = origin.Y + y;

                var over = source.GetPixel(dx, dy);
                if (alpha >= 1f)
                {
                    destination.SetPixel(dx, dy, over);
                    continue;
                }

                destination.SetPixel(dx, dy, destination.GetPixel(dx, dy).Lerp(over, alpha));
            }
        }
    }

    /// <summary>The plain 8x8 artwork for a terrain type, cropped out of its sheet.</summary>
    private Image GetBaseTile(ushort type)
    {
        if (_baseTiles.TryGetValue(type, out var cached))
            return cached;

        Image image = null;
        var spec = _data.GetGround(type)?.Texture;

        if (spec is { Kind: TextureKind.Random, Variants.Count: > 0 })
            spec = spec.Variants[0];

        if (spec?.File != null)
        {
            var sprite = _assets.GetSprite(spec.File, spec.Index);
            if (sprite.IsValid)
                image = Crop(sprite);
        }

        _baseTiles[type] = image;
        return image;
    }

    private Image Crop(Sprite sprite)
    {
        if (!_sheets.TryGetValue(sprite.Sheet, out var sheet))
        {
            sheet = sprite.Sheet.GetImage();
            _sheets[sprite.Sheet] = sheet;
        }

        if (sheet == null)
            return null;

        var region = sprite.Region;
        var image = Image.CreateEmpty(region.Size.X, region.Size.Y, false, Image.Format.Rgba8);
        image.BlitRect(sheet, region, Vector2I.Zero);
        return image;
    }
}

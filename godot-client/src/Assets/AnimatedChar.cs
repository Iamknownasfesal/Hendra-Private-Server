using System;
using Godot;

namespace Hendra.Assets;

/// <summary>Which way a character is facing. The values match the AS3 constants.</summary>
public enum CharFacing
{
    Right = 0,
    Left = 1,
    Down = 2,
    Up = 3,
}

/// <summary>What a character is doing. The values match the AS3 constants.</summary>
public enum CharAction
{
    Stand = 0,
    Walk = 1,
    Attack = 2,
}

/// <summary>
/// One frame of a character animation.
/// </summary>
/// <remarks>
/// Most frames are a single cell, but the second attack frame is not: the original composites cells
/// 5 and 6 of the row into a three-cell-wide image with the first cell left empty, which is what
/// makes a swung weapon extend past the character's own footprint. Rather than baking that
/// composite into a new texture at load time, the region and the quad geometry are kept separate —
/// <see cref="CellsWide"/> is how wide the quad should be and <see cref="ContentCell"/> is where
/// the sampled region sits inside it.
/// </remarks>
public readonly struct CharFrame
{
    /// <summary>The pixels to sample. May span more than one cell.</summary>
    public readonly Sprite Sprite;

    /// <summary>The matching region of the recolour mask, if the sheet has one.</summary>
    public readonly Sprite Mask;

    /// <summary>Whether to draw the sprite flipped horizontally.</summary>
    public readonly bool Mirrored;

    /// <summary>Total quad width in cells.</summary>
    public readonly int CellsWide;

    /// <summary>How many cells the sampled region itself spans.</summary>
    public readonly int RegionCells;

    /// <summary>Which cell of the quad the sampled region begins at, before mirroring.</summary>
    public readonly int ContentCell;

    public CharFrame(Sprite sprite, Sprite mask, bool mirrored, int cellsWide = 1, int regionCells = 1,
        int contentCell = 0)
    {
        Sprite = sprite;
        Mask = mask;
        Mirrored = mirrored;
        CellsWide = cellsWide;
        RegionCells = regionCells;
        ContentCell = contentCell;
    }

    public bool IsValid => Sprite.IsValid;

    /// <summary>
    /// Where the region sits within the quad once mirroring is applied. Flipping the quad moves the
    /// composite's empty cell from the left side to the right.
    /// </summary>
    public int EffectiveContentCell =>
        Mirrored ? CellsWide - ContentCell - RegionCells : ContentCell;
}

/// <summary>
/// One character's animation set: three actions across up to four facings, sliced out of a shared
/// sheet.
/// </summary>
/// <remarks>
/// Layout, which is entirely implicit in the original: a sheet is cut into per-character strips,
/// and each strip is cut into cells. A strip holds up to three rows of seven cells, one row per
/// facing. Within a row the cells are
/// <c>0</c> stand, <c>1</c> and <c>2</c> walk, <c>3</c> unused, <c>4</c> attack, <c>5</c>+<c>6</c>
/// the extended attack.
///
/// Which rows exist depends on how many cells the strip has, and which facing each row represents
/// depends on the sheet's declared first direction. Sheets that only supply one row get their other
/// facings by mirroring, which is why a lot of enemies visibly turn to face you but never show
/// their back.
///
/// Empty cells are meaningful rather than accidental: a fully transparent walk or attack cell means
/// "this character has no such pose", and the original falls back to another frame. That has to be
/// detected by looking at the pixels, so it is done once at load.
/// </remarks>
public sealed class AnimatedChar
{
    private const int CellsPerRow = 7;
    private const int FramesForTwoRows = 14;
    private const int FramesForThreeRows = 21;

    private static readonly float EighthTurn = Mathf.Pi / 4f;

    /// <summary>
    /// For each of the eight 45-degree sectors, the facings to try in order. A sheet that lacks the
    /// preferred facing falls through to the next.
    /// </summary>
    private static readonly CharFacing[][] SectorFacings =
    {
        new[] { CharFacing.Left, CharFacing.Up, CharFacing.Down },
        new[] { CharFacing.Up, CharFacing.Left, CharFacing.Down },
        new[] { CharFacing.Up, CharFacing.Right, CharFacing.Down },
        new[] { CharFacing.Right, CharFacing.Up, CharFacing.Down },
        new[] { CharFacing.Right, CharFacing.Down },
        new[] { CharFacing.Down, CharFacing.Right },
        new[] { CharFacing.Down, CharFacing.Left },
        new[] { CharFacing.Left, CharFacing.Down },
    };

    // [facing][action] -> the frames of that animation, or null when the facing is absent.
    private readonly CharFrame[][][] _frames = new CharFrame[4][][];

    public CharFacing FirstDirection { get; }

    private AnimatedChar(CharFacing firstDirection)
    {
        FirstDirection = firstDirection;
    }

    /// <summary>
    /// Builds one character's animation set from its strip.
    /// </summary>
    /// <param name="cells">Cell lookup for the strip, indexed 0..cellCount-1.</param>
    /// <param name="maskCells">Matching mask lookup, or null.</param>
    /// <param name="cellCount">How many cells the strip holds.</param>
    /// <param name="isEmpty">Whether the given cell is fully transparent.</param>
    /// <param name="firstDirection">Which facing the first row represents.</param>
    public static AnimatedChar Build(
        Func<int, int, Sprite> cells,
        Func<int, int, Sprite> maskCells,
        int cellCount,
        Func<int, bool> isEmpty,
        CharFacing firstDirection)
    {
        var result = new AnimatedChar(firstDirection);

        if (firstDirection == CharFacing.Right)
        {
            result.LoadRow(CharFacing.Right, 0, mirrored: false, mirrorFallback: false, cells, maskCells, isEmpty);
            result.LoadRow(CharFacing.Left, 0, mirrored: true, mirrorFallback: false, cells, maskCells, isEmpty);
            if (cellCount >= FramesForTwoRows)
                result.LoadRow(CharFacing.Down, CellsPerRow, false, true, cells, maskCells, isEmpty);
            if (cellCount >= FramesForThreeRows)
                result.LoadRow(CharFacing.Up, CellsPerRow * 2, false, true, cells, maskCells, isEmpty);
        }
        else
        {
            result.LoadRow(CharFacing.Down, 0, mirrored: false, mirrorFallback: true, cells, maskCells, isEmpty);
            if (cellCount >= FramesForTwoRows)
            {
                result.LoadRow(CharFacing.Right, CellsPerRow, false, false, cells, maskCells, isEmpty);
                result.LoadRow(CharFacing.Left, CellsPerRow, true, false, cells, maskCells, isEmpty);
            }
            if (cellCount >= FramesForThreeRows)
                result.LoadRow(CharFacing.Up, CellsPerRow * 2, false, true, cells, maskCells, isEmpty);
        }

        return result;
    }

    private void LoadRow(
        CharFacing facing,
        int baseCell,
        bool mirrored,
        bool mirrorFallback,
        Func<int, int, Sprite> cells,
        Func<int, int, Sprite> maskCells,
        Func<int, bool> isEmpty)
    {
        CharFrame One(int offset, int cellsWide = 1, int contentCell = 0)
        {
            int regionCells = cellsWide - contentCell;
            return new CharFrame(
                cells(baseCell + offset, regionCells),
                maskCells?.Invoke(baseCell + offset, regionCells) ?? default,
                mirrored,
                cellsWide,
                regionCells,
                contentCell);
        }

        var stand = new[] { One(0) };

        // Walk is always two frames. A missing second frame is filled by mirroring the first —
        // which is how a two-frame gait is faked from a single drawn pose — or, failing that, by
        // reusing the standing frame.
        CharFrame walkSecond;
        if (!isEmpty(baseCell + 2))
            walkSecond = One(2);
        else if (mirrorFallback)
            walkSecond = new CharFrame(cells(baseCell + 1, 1), maskCells?.Invoke(baseCell + 1, 1) ?? default, !mirrored);
        else
            walkSecond = One(0);

        var walk = new[] { One(1), walkSecond };

        bool hasAttack1 = !isEmpty(baseCell + 4);
        bool hasAttack2 = !isEmpty(baseCell + 5);
        bool hasExtension = hasAttack2 && !isEmpty(baseCell + 6);

        CharFrame[] attack;
        if (!hasAttack1 && !hasAttack2)
        {
            // No attack pose at all: the original reuses the walk cycle.
            attack = walk;
        }
        else
        {
            var frames = new System.Collections.Generic.List<CharFrame>(2);
            if (hasAttack1)
                frames.Add(One(4));
            if (hasAttack2)
            {
                // Cells 5 and 6 together, drawn inside a three-cell quad whose first cell is empty
                // so the weapon reaches beyond the character.
                frames.Add(hasExtension ? One(5, cellsWide: 3, contentCell: 1) : One(5));
            }
            attack = frames.ToArray();
        }

        _frames[(int)facing] = new[] { stand, walk, attack };
    }

    /// <summary>Whether this character has artwork for the given facing.</summary>
    public bool Has(CharFacing facing) => _frames[(int)facing] != null;

    /// <summary>
    /// Picks a frame for a world-space facing angle.
    /// </summary>
    /// <param name="facing">The character's heading, in radians.</param>
    /// <param name="cameraAngle">
    /// The camera's yaw. Subtracted from the heading because the camera rotates, so which way a
    /// character appears to face depends on where the viewer is standing.
    /// </param>
    /// <param name="action">Which animation to sample.</param>
    /// <param name="phase">Position within the animation, in [0, 1).</param>
    public CharFrame Frame(float facing, float cameraAngle, CharAction action, float phase)
    {
        int sector = Mathf.PosMod((int)(WrapToPi(facing - cameraAngle) / EighthTurn + 4f), 8);

        CharFrame[][] byAction = null;
        foreach (var candidate in SectorFacings[sector])
        {
            byAction = _frames[(int)candidate];
            if (byAction != null)
                break;
        }

        // A sheet with no usable row at all would be a malformed asset; fall back to whatever
        // exists rather than throwing in the render loop.
        byAction ??= FirstAvailable();
        if (byAction == null)
            return default;

        var frames = byAction[(int)action];
        if (frames == null || frames.Length == 0)
            return default;

        int index = (int)(Mathf.Clamp(phase, 0f, 0.99999f) * frames.Length);
        return frames[index];
    }

    private CharFrame[][] FirstAvailable()
    {
        foreach (var byAction in _frames)
        {
            if (byAction != null)
                return byAction;
        }
        return null;
    }

    /// <summary>Wraps an angle into (-pi, pi]. The AS3 client called this Trig.boundToPI.</summary>
    public static float WrapToPi(float angle)
    {
        while (angle > Mathf.Pi)
            angle -= Mathf.Tau;
        while (angle <= -Mathf.Pi)
            angle += Mathf.Tau;
        return angle;
    }
}

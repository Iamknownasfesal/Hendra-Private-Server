using System;

namespace Hendra.World;

/// <summary>
/// What the movement resolver needs to know about the map. Kept narrow so the collision rules can
/// be tested against a hand-built grid rather than a live world.
/// </summary>
public interface ITileQuery
{
    /// <summary>
    /// Whether the tile containing this point can be stood on: it exists, its terrain is not
    /// no-walk, and it holds no object that occupies its square.
    /// </summary>
    bool IsWalkable(int tileX, int tileY);

    /// <summary>
    /// Whether the tile blocks completely — out of bounds, unrevealed, or holding a full-occupy
    /// object. Note that an *unknown* tile counts as blocking, which is what stops the player
    /// walking off the edge of the streamed region.
    /// </summary>
    bool IsFullOccupy(int tileX, int tileY);
}

/// <summary>
/// Where a movement attempt ended up, and whether it was stopped along the way.
/// </summary>
public readonly struct MoveResult
{
    public readonly float X;
    public readonly float Y;

    /// <summary>True when the mover was blocked on the horizontal axis this step.</summary>
    public readonly bool BlockedX;

    /// <summary>True when the mover was blocked on the vertical axis this step.</summary>
    public readonly bool BlockedY;

    public MoveResult(float x, float y, bool blockedX = false, bool blockedY = false)
    {
        X = x;
        Y = y;
        BlockedX = blockedX;
        BlockedY = blockedY;
    }
}

/// <summary>
/// The player's collision rules, ported from the AS3 client.
/// </summary>
/// <remarks>
/// <para>
/// The shape of this is unusual and worth stating plainly, because it is not a circle or a box
/// sweep. The mover is treated as occupying a half-tile radius around its position, and validity is
/// decided by testing the up-to-three neighbouring tiles that radius reaches into — chosen by which
/// side of the half-tile line the position falls on. Movement is then resolved by walking towards
/// the target in steps of at most 0.4 tiles, and, when a step would cross a half-tile boundary into
/// something blocking, snapping back to that boundary on whichever axis is cheaper.
/// </para>
/// <para>
/// The server does almost no movement validation of its own — its speed check is commented out and
/// it accepts whatever position the client reports — but it does run a no-clip test after every
/// move and disconnects if the new tile is occupied. So these rules are what keep the connection
/// alive, which is why they are reproduced exactly rather than replaced with something tidier.
/// </para>
/// </remarks>
public static class Movement
{
    /// <summary>The largest distance resolved in one collision step.</summary>
    public const float MoveThreshold = 0.4f;

    /// <summary>
    /// Keeps a position snapped to a tile boundary just inside the tile it came from, rather than
    /// exactly on the line where rounding could put it in the next one.
    /// </summary>
    private const float BoundaryEpsilon = 0.01f;

    /// <summary>
    /// Resolves a move from <paramref name="fromX"/>, <paramref name="fromY"/> towards a target,
    /// subdividing it so a fast mover cannot tunnel through a wall.
    /// </summary>
    /// <remarks>
    /// Faithful to one quirk of the original: each sub-step's boundary tests are taken against the
    /// mover's *starting* position, not the position reached by the previous sub-step. Only the
    /// resulting coordinate accumulates. That is almost certainly not what was intended, but it is
    /// what the client has always done, and movement feel — how you slide along a wall, whether a
    /// diagonal into a corner catches — depends on it.
    /// </remarks>
    public static MoveResult Resolve(ITileQuery map, float fromX, float fromY, float targetX, float targetY)
    {
        float dx = targetX - fromX;
        float dy = targetY - fromY;

        if (dx > -MoveThreshold && dx < MoveThreshold && dy > -MoveThreshold && dy < MoveThreshold)
            return Step(map, fromX, fromY, targetX, targetY);

        float fraction = MoveThreshold / Math.Max(Math.Abs(dx), Math.Abs(dy));
        float travelled = 0f;
        float x = fromX;
        float y = fromY;
        bool blockedX = false;
        bool blockedY = false;

        while (true)
        {
            bool last = false;
            if (travelled + fraction >= 1f)
            {
                fraction = 1f - travelled;
                last = true;
            }

            var step = Step(map, fromX, fromY, x + dx * fraction, y + dy * fraction);
            x = step.X;
            y = step.Y;
            blockedX |= step.BlockedX;
            blockedY |= step.BlockedY;

            travelled += fraction;
            if (last)
                break;
        }

        return new MoveResult(x, y, blockedX, blockedY);
    }

    /// <summary>
    /// Resolves a single sub-step, snapping to a half-tile boundary when the destination is blocked.
    /// </summary>
    private static MoveResult Step(ITileQuery map, float fromX, float fromY, float toX, float toY)
    {
        bool crossesX = (fromX % 0.5f == 0f && toX != fromX) || (int)(fromX / 0.5f) != (int)(toX / 0.5f);
        bool crossesY = (fromY % 0.5f == 0f && toY != fromY) || (int)(fromY / 0.5f) != (int)(toY / 0.5f);

        // Staying within the same half-tile cell cannot change what we collide with, so the test is
        // skipped entirely — which is also what makes small movements cheap.
        if ((!crossesX && !crossesY) || IsValidPosition(map, toX, toY, fromX, fromY))
            return new MoveResult(toX, toY);

        float boundaryX = 0f;
        float boundaryY = 0f;

        if (crossesX)
        {
            boundaryX = toX > fromX ? (int)(toX * 2f) / 2f : (int)(fromX * 2f) / 2f;
            if ((int)boundaryX > (int)fromX)
                boundaryX -= BoundaryEpsilon;
        }

        if (crossesY)
        {
            boundaryY = toY > fromY ? (int)(toY * 2f) / 2f : (int)(fromY * 2f) / 2f;
            if ((int)boundaryY > (int)fromY)
                boundaryY -= BoundaryEpsilon;
        }

        if (!crossesX)
            return new MoveResult(toX, boundaryY, blockedX: false, blockedY: true);

        if (!crossesY)
            return new MoveResult(boundaryX, toY, blockedX: true, blockedY: false);

        // Both axes crossed. Try releasing whichever axis overshot further first, so a glancing
        // approach slides along the wall instead of stopping dead.
        float overshootX = toX > fromX ? toX - boundaryX : boundaryX - toX;
        float overshootY = toY > fromY ? toY - boundaryY : boundaryY - toY;

        if (overshootX > overshootY)
        {
            if (IsValidPosition(map, toX, boundaryY, fromX, fromY))
                return new MoveResult(toX, boundaryY, false, true);
            if (IsValidPosition(map, boundaryX, toY, fromX, fromY))
                return new MoveResult(boundaryX, toY, true, false);
        }
        else
        {
            if (IsValidPosition(map, boundaryX, toY, fromX, fromY))
                return new MoveResult(boundaryX, toY, true, false);
            if (IsValidPosition(map, toX, boundaryY, fromX, fromY))
                return new MoveResult(toX, boundaryY, false, true);
        }

        return new MoveResult(boundaryX, boundaryY, true, true);
    }

    /// <summary>
    /// Whether the mover may stand at this exact position.
    /// </summary>
    /// <remarks>
    /// The mover reaches half a tile in each direction, so as well as its own tile it has to clear
    /// whichever neighbours that radius overlaps. Which neighbours those are follows from where the
    /// position sits relative to the half-tile lines: left of centre reaches west, right of centre
    /// reaches east, and exactly on the line reaches neither.
    /// </remarks>
    /// <param name="fromX">The mover's current position, whose own tile is exempt from the
    /// walkability test. Without that exemption a mover standing on a tile that becomes blocking —
    /// a closing door, a newly spawned obstacle — would be frozen in place rather than able to step
    /// off it.</param>
    /// <param name="fromY">See <paramref name="fromX"/>.</param>
    public static bool IsValidPosition(ITileQuery map, float x, float y, float fromX, float fromY)
    {
        int tileX = (int)x;
        int tileY = (int)y;

        bool sameTileAsMover = tileX == (int)fromX && tileY == (int)fromY;
        if (!sameTileAsMover && !map.IsWalkable(tileX, tileY))
            return false;

        float fractionX = x - tileX;
        float fractionY = y - tileY;

        if (fractionX < 0.5f)
        {
            if (map.IsFullOccupy(tileX - 1, tileY))
                return false;

            if (fractionY < 0.5f)
            {
                if (map.IsFullOccupy(tileX, tileY - 1) || map.IsFullOccupy(tileX - 1, tileY - 1))
                    return false;
            }
            else if (fractionY > 0.5f)
            {
                if (map.IsFullOccupy(tileX, tileY + 1) || map.IsFullOccupy(tileX - 1, tileY + 1))
                    return false;
            }
        }
        else if (fractionX > 0.5f)
        {
            if (map.IsFullOccupy(tileX + 1, tileY))
                return false;

            if (fractionY < 0.5f)
            {
                if (map.IsFullOccupy(tileX, tileY - 1) || map.IsFullOccupy(tileX + 1, tileY - 1))
                    return false;
            }
            else if (fractionY > 0.5f)
            {
                if (map.IsFullOccupy(tileX, tileY + 1) || map.IsFullOccupy(tileX + 1, tileY + 1))
                    return false;
            }
        }
        else
        {
            if (fractionY < 0.5f)
            {
                if (map.IsFullOccupy(tileX, tileY - 1))
                    return false;
            }
            else if (fractionY > 0.5f)
            {
                if (map.IsFullOccupy(tileX, tileY + 1))
                    return false;
            }
        }

        return true;
    }
}

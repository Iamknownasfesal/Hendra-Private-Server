using System.Collections.Generic;
using Hendra.World;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// The player's collision rules.
///
/// These matter beyond feel: the server accepts whatever position the client reports, but runs a
/// no-clip check afterwards and disconnects if the reported tile is occupied. So a collision bug
/// here does not show up as clipping through a wall — it shows up as being kicked.
/// </summary>
public sealed class MovementTests
{
    /// <summary>
    /// A small hand-built map. Everything outside the declared bounds is treated as unknown, which
    /// the rules count as blocking.
    /// </summary>
    private sealed class Grid : ITileQuery
    {
        private readonly int _width;
        private readonly int _height;
        private readonly HashSet<(int, int)> _blocked = new();
        private readonly HashSet<(int, int)> _fullOccupy = new();

        public Grid(int width = 10, int height = 10)
        {
            _width = width;
            _height = height;
        }

        /// <summary>Marks a tile as non-walkable but not solid — terrain you cannot stand on.</summary>
        public Grid Block(int x, int y)
        {
            _blocked.Add((x, y));
            return this;
        }

        /// <summary>Marks a tile as solid, which also pushes back on adjacent positions.</summary>
        public Grid Solid(int x, int y)
        {
            _blocked.Add((x, y));
            _fullOccupy.Add((x, y));
            return this;
        }

        private bool InBounds(int x, int y) => x >= 0 && y >= 0 && x < _width && y < _height;

        public bool IsWalkable(int x, int y) => InBounds(x, y) && !_blocked.Contains((x, y));

        public bool IsFullOccupy(int x, int y) => !InBounds(x, y) || _fullOccupy.Contains((x, y));
    }

    [Fact]
    public void OpenGroundLetsAMoveThrough()
    {
        var grid = new Grid();
        var result = Movement.Resolve(grid, 5.5f, 5.5f, 5.6f, 5.5f);

        Assert.Equal(5.6f, result.X, 4);
        Assert.Equal(5.5f, result.Y, 4);
        Assert.False(result.BlockedX);
        Assert.False(result.BlockedY);
    }

    [Fact]
    public void ASolidTileStopsHorizontalMovement()
    {
        // Standing at x=5.5 and walking east into a solid tile at x=7. The half-tile radius means
        // contact happens before the tile is entered.
        var grid = new Grid().Solid(7, 5);
        var result = Movement.Resolve(grid, 5.5f, 5.5f, 8.0f, 5.5f);

        Assert.True(result.X < 7.0f, $"Walked to {result.X}, which is inside or past the solid tile.");
        Assert.True(result.BlockedX);
    }

    [Fact]
    public void MovementSlidesAlongAWall()
    {
        // A diagonal into a wall that only blocks one axis should keep the free axis moving, which
        // is what makes walls feel like walls rather than glue.
        var grid = new Grid().Solid(7, 5);
        var result = Movement.Resolve(grid, 5.5f, 5.5f, 8.0f, 6.5f);

        Assert.True(result.X < 7.0f);
        Assert.True(result.Y > 5.5f, "Vertical movement should have continued while blocked horizontally.");
    }

    [Fact]
    public void AFastMoveCannotTunnelThroughAWall()
    {
        // The whole reason movement is subdivided: a single frame at high speed must not step over
        // a one-tile-thick wall.
        var grid = new Grid(40, 40).Solid(10, 5);
        var result = Movement.Resolve(grid, 5.5f, 5.5f, 30.0f, 5.5f);

        Assert.True(result.X < 10.0f, $"Tunnelled to {result.X}, past the wall at x=10.");
    }

    [Fact]
    public void TheEdgeOfTheKnownMapBlocks()
    {
        // Unknown tiles count as solid. This is what keeps the player inside the streamed region
        // rather than walking into terrain the server has not sent yet.
        var grid = new Grid(10, 10);
        var result = Movement.Resolve(grid, 1.5f, 5.5f, -5.0f, 5.5f);

        Assert.True(result.X >= 0f, $"Walked off the map to {result.X}.");
    }

    [Fact]
    public void TheMoversOwnTileIsExemptFromTheWalkableTest()
    {
        // A tile can become non-walkable underneath the player — a door closing, an object
        // spawning. The original lets them move within it so they can step off; without the
        // exemption they would be stuck permanently.
        var grid = new Grid().Block(5, 5);

        Assert.True(Movement.IsValidPosition(grid, 5.5f, 5.5f, 5.5f, 5.5f));
        Assert.False(Movement.IsValidPosition(grid, 5.5f, 5.5f, 6.5f, 5.5f));
    }

    [Fact]
    public void PositionsOnTheHalfTileLineReachNeitherNeighbour()
    {
        // Exactly on the line the mover reaches into neither side, which is what lets it sit
        // flush against a wall.
        var grid = new Grid().Solid(6, 5);

        Assert.True(Movement.IsValidPosition(grid, 5.5f, 5.5f, 5.5f, 5.5f));
        Assert.False(Movement.IsValidPosition(grid, 5.6f, 5.5f, 5.5f, 5.5f));
    }

    [Fact]
    public void DiagonalNeighboursAreTestedToo()
    {
        // A mover past the centre of its tile on both axes reaches the diagonal, so a solid corner
        // blocks even though neither orthogonal neighbour does.
        var grid = new Grid().Solid(6, 6);

        Assert.False(Movement.IsValidPosition(grid, 5.6f, 5.6f, 5.5f, 5.5f));
        Assert.True(Movement.IsValidPosition(grid, 5.4f, 5.4f, 5.5f, 5.5f));
    }

    [Fact]
    public void NonSolidBlockedTerrainStopsEntryButNotProximity()
    {
        // Terrain that is merely non-walkable — deep water, a chasm — cannot be entered, but does
        // not push back on a mover standing beside it the way a solid object does.
        var grid = new Grid().Block(7, 5);

        Assert.True(Movement.IsValidPosition(grid, 6.9f, 5.5f, 5.5f, 5.5f));
        Assert.False(Movement.IsValidPosition(grid, 7.5f, 5.5f, 5.5f, 5.5f));
    }

    [Fact]
    public void ACorridorCanBeWalkedEndToEnd()
    {
        // A one-tile gap between two walls has to remain passable. The half-tile radius means this
        // is exactly the tightest case that must still work.
        var grid = new Grid(20, 20);
        for (int x = 0; x < 20; x++)
        {
            grid.Solid(x, 4);
            grid.Solid(x, 6);
        }

        float x0 = 1.5f;
        for (int step = 0; step < 200; step++)
        {
            var result = Movement.Resolve(grid, x0, 5.5f, x0 + 0.09f, 5.5f);
            Assert.Equal(5.5f, result.Y, 3);
            x0 = result.X;
        }

        Assert.True(x0 > 15f, $"Only reached x={x0} along an open corridor.");
    }

    [Fact]
    public void ResolvingAZeroLengthMoveIsStable()
    {
        var grid = new Grid();
        var result = Movement.Resolve(grid, 5.5f, 5.5f, 5.5f, 5.5f);

        Assert.Equal(5.5f, result.X, 5);
        Assert.Equal(5.5f, result.Y, 5);
    }
}

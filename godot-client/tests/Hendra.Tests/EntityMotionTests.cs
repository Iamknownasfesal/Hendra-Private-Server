using Hendra.Resources;
using Hendra.World;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// How other people's entities move.
///
/// The client never receives a velocity: a tick carries a position, and everything between two
/// ticks is the client closing the gap. Getting that wrong is not subtle — monsters either stand
/// still or teleport — so it is worth pinning down, especially the part that is easy to mistake for
/// a bug in the other direction: a server that sends the same position every tick is *supposed* to
/// leave the entity where it is.
/// </summary>
public sealed class EntityMotionTests
{
    private static GameMap NewMap()
    {
        var map = new GameMap(new GameData());
        map.Reset(64, 64, "test");
        return map;
    }

    private static Entity AddEntity(GameMap map, float x, float y)
    {
        var entity = new Entity { ObjectId = 1 };
        map.Add(entity, x, y);
        return entity;
    }

    /// <summary>
    /// An entity told to be somewhere else gets there — to within a tenth of a tile.
    /// </summary>
    /// <remarks>
    /// It stops short on purpose. The approach is switched off once the gap is under a tenth of a
    /// tile, which is the original's rule and costs nothing in practice: the next tick moves the
    /// target again long before anyone could notice five pixels of lag. The residual only shows on
    /// something that has stopped, where it is invisible.
    /// </remarks>
    [Fact]
    public void ConvergesOnTheTickPosition()
    {
        var map = NewMap();
        var entity = AddEntity(map, 10f, 10f);

        entity.OnTickPosition(14f, 10f, tickDurationMs: 166);

        for (int elapsed = 0; elapsed < 3000; elapsed += 16)
            map.Update(elapsed, 16);

        Assert.True(entity.DistanceTo(14f, 10f) <= 0.1f, $"stopped at {entity.X}, {entity.Y}");
    }

    /// <summary>
    /// It closes a fixed fraction of the remaining gap each frame rather than travelling at a fixed
    /// speed, which is what gives remote entities their slightly-lagging glide.
    /// </summary>
    [Fact]
    public void ApproachesExponentiallyRatherThanLinearly()
    {
        var map = NewMap();
        var entity = AddEntity(map, 10f, 10f);
        entity.OnTickPosition(20f, 10f, tickDurationMs: 166);

        map.Update(16, 16);
        float firstStep = entity.X - 10f;

        for (int i = 0; i < 20; i++)
            map.Update(32 + i * 16, 16);

        float before = entity.X;
        map.Update(1000, 16);
        float laterStep = entity.X - before;

        Assert.True(firstStep > 0f, "the first frame moved nothing");
        Assert.True(laterStep < firstStep, $"steps are not shrinking: {firstStep} then {laterStep}");
    }

    /// <summary>
    /// A tick that repeats the position leaves the entity alone.
    /// </summary>
    /// <remarks>
    /// This is what a world whose monsters have no behaviours looks like from the client's side, and
    /// it is worth having written down: standing-still enemies are not evidence of a fault here.
    /// </remarks>
    [Fact]
    public void UnchangedTicksLeaveTheEntityWhereItIs()
    {
        var map = NewMap();
        var entity = AddEntity(map, 10f, 10f);

        for (int tick = 0; tick < 10; tick++)
        {
            entity.OnTickPosition(10f, 10f, tickDurationMs: 166);
            for (int frame = 0; frame < 10; frame++)
                map.Update(tick * 166 + frame * 16, 16);
        }

        Assert.Equal(10f, entity.X, 4);
        Assert.Equal(10f, entity.Y, 4);
        Assert.False(entity.IsMoving);
    }

    /// <summary>Motion drives the walk animation, and stops driving it on arrival.</summary>
    [Fact]
    public void MovementFlagFollowsTheGap()
    {
        var map = NewMap();
        var entity = AddEntity(map, 10f, 10f);

        entity.OnTickPosition(13f, 10f, tickDurationMs: 166);
        Assert.True(entity.IsMoving);

        for (int elapsed = 0; elapsed < 4000; elapsed += 16)
            map.Update(elapsed, 16);

        Assert.False(entity.IsMoving);
    }

    /// <summary>
    /// An entity that interpolates onto another tile is re-attached to it.
    /// </summary>
    /// <remarks>
    /// Its square is what the renderer culls on. Left stale, a monster that walked out of the
    /// streamed-in area would keep being drawn and one that walked into it would stay hidden.
    /// </remarks>
    [Fact]
    public void MovingEntitiesFollowTheirSquare()
    {
        var map = NewMap();
        for (int x = 8; x < 20; x++)
            map.SetTile(x, 10, 1);

        var entity = AddEntity(map, 10.5f, 10.5f);
        var startedOn = entity.Square;

        entity.OnTickPosition(15.5f, 10.5f, tickDurationMs: 166);
        for (int elapsed = 0; elapsed < 4000; elapsed += 16)
            map.Update(elapsed, 16);

        Assert.NotNull(entity.Square);
        Assert.NotSame(startedOn, entity.Square);
        Assert.Same(map.GetSquare(15, 10), entity.Square);
    }

    /// <summary>An authoritative correction snaps rather than gliding.</summary>
    [Fact]
    public void GotoSnaps()
    {
        var map = NewMap();
        var entity = AddEntity(map, 10f, 10f);

        entity.OnTickPosition(20f, 20f, tickDurationMs: 166);
        entity.OnGoto(30f, 30f);

        Assert.Equal(30f, entity.X);
        Assert.Equal(30f, entity.Y);
        Assert.False(entity.IsMoving);

        map.Update(16, 16);
        Assert.Equal(30f, entity.X, 4);
    }

    /// <summary>Our own position is never taken from a tick; the client owns it between Moves.</summary>
    [Fact]
    public void TheLocalPlayerIgnoresTickPositions()
    {
        var player = new LocalPlayer();
        player.Place(10f, 10f);

        player.OnTickPosition(50f, 50f, tickDurationMs: 166);
        player.Update(16, 16);

        Assert.Equal(10f, player.X);
        Assert.Equal(10f, player.Y);
    }
}

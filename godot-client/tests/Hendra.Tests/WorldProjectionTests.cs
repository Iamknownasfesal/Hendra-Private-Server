using Godot;
using Hendra.Render;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// Verifies that the 3D scene setup reproduces the AS3 camera's projection.
///
/// This is the part of the port with the least margin for a plausible-looking mistake: a tilted
/// camera, or a sign error in the height axis, still renders something that broadly resembles the
/// game while being subtly wrong everywhere. So the scene mapping is checked against the original's
/// formula directly, by projecting through the camera basis Godot will actually build.
/// </summary>
public sealed class WorldProjectionTests
{
    private const float Tolerance = 1e-4f;

    /// <summary>
    /// Screen position as the AS3 client computes it, in tiles, with Y increasing downward.
    /// Transcribed from Camera.configure: the world-to-view matrix's rows are
    /// (r.x, u.x, ...), (r.y, u.y, ...), (r.z, -1, ...) with a translation of -(p·r), -(p·u).
    /// </summary>
    private static Vector2 As3Screen(float angle, Vector2 camera, Vector2 point, float height)
    {
        var right = new Vector2(Mathf.Cos(angle), Mathf.Sin(angle));
        var up = new Vector2(Mathf.Cos(angle + Mathf.Pi / 2f), Mathf.Sin(angle + Mathf.Pi / 2f));
        var d = point - camera;
        return new Vector2(d.Dot(right), d.Dot(up) - height);
    }

    /// <summary>
    /// Screen position as Godot will compute it: build the camera basis for our claimed yaw, look
    /// straight down, and project orthographically. Screen Y is negated because the view's up axis
    /// points the opposite way from screen-space down.
    /// </summary>
    private static Vector2 GodotScreen(WorldProjection projection, Vector2 camera, Vector3 scenePoint)
    {
        // Pitch straight down, then yaw. Godot composes Euler angles in YXZ order by default.
        var basis = Basis.FromEuler(new Vector3(-Mathf.Pi / 2f, projection.CameraYaw, 0f), EulerOrder.Yxz);
        var cameraPosition = projection.CameraPosition(camera.X, camera.Y);

        var d = scenePoint - cameraPosition;
        float viewX = d.Dot(basis.X);
        float viewY = d.Dot(basis.Y);
        return new Vector2(viewX, -viewY);
    }

    [Theory]
    [InlineData(0f)]
    [InlineData(0.7853981f)]          // an eighth turn
    [InlineData(5.4977871f)]          // the client's default camera angle, 7*pi/4
    [InlineData(2.3f)]
    [InlineData(-1.1f)]
    public void SceneMappingReproducesTheOriginalProjection(float angle)
    {
        var projection = new WorldProjection(angle);
        var camera = new Vector2(100f, 250f);

        (float X, float Y, float Height)[] samples =
        {
            (100f, 250f, 0f),       // the camera's own focus
            (101f, 250f, 0f),       // one tile east
            (100f, 251f, 0f),       // one tile south
            (104.5f, 243.25f, 0f),  // an arbitrary offset
            (100f, 250f, 1f),       // directly overhead, one tile up
            (97f, 262f, 2.5f),      // offset and elevated
        };

        foreach (var (x, y, height) in samples)
        {
            var expected = As3Screen(angle, camera, new Vector2(x, y), height);
            var actual = GodotScreen(projection, camera, projection.ToScene(x, y, height));

            Assert.Equal(expected.X, actual.X, Tolerance);
            Assert.Equal(expected.Y, actual.Y, Tolerance);
        }
    }

    [Fact]
    public void TheGroundPlaneIsNotForeshortened()
    {
        // The defining property of the oblique projection, and the thing a tilted camera would
        // break: a tile is the same size on screen whichever way it is oriented.
        var projection = new WorldProjection(0.9f);
        var camera = new Vector2(0f, 0f);

        var origin = GodotScreen(projection, camera, projection.ToScene(0f, 0f));
        var east = GodotScreen(projection, camera, projection.ToScene(1f, 0f));
        var south = GodotScreen(projection, camera, projection.ToScene(0f, 1f));

        Assert.Equal(1f, (east - origin).Length(), Tolerance);
        Assert.Equal(1f, (south - origin).Length(), Tolerance);
    }

    [Fact]
    public void HeightMovesSpritesUpTheScreenOneForOne()
    {
        // Height is added straight into screen Y at a one-to-one ratio with ground distance, which
        // is what makes walls read as walls rather than as flat decals.
        var projection = new WorldProjection(1.3f);
        var camera = new Vector2(5f, 5f);

        var grounded = GodotScreen(projection, camera, projection.ToScene(6f, 7f));
        var raised = GodotScreen(projection, camera, projection.ToScene(6f, 7f, height: 2f));

        Assert.Equal(grounded.X, raised.X, Tolerance);
        Assert.Equal(grounded.Y - 2f, raised.Y, Tolerance);
    }

    [Fact]
    public void HeightDoesNotAffectDepth()
    {
        // Sort order comes from the ground point alone. If height leaked into the depth key, a tall
        // object would sort in front of things it is actually standing behind.
        var projection = new WorldProjection(0.4f);

        var grounded = projection.ToScene(3f, 4f);
        var raised = projection.ToScene(3f, 4f, height: 5f);

        Assert.Equal(grounded.Y, raised.Y, Tolerance);
    }

    [Theory]
    [InlineData(0f)]
    [InlineData(1.9f)]
    [InlineData(5.4977871f)]
    public void DepthOrderingMatchesScreenY(float angle)
    {
        // The original drew back to front by screen Y of the ground point. Here that ordering has
        // to come out of the depth buffer instead, so the depth key must rise with screen Y —
        // and, because the camera looks down from above, rising depth means closer to the camera.
        var projection = new WorldProjection(angle);
        var camera = new Vector2(0f, 0f);

        (float X, float Y)[] points = { (0f, 0f), (3f, 1f), (-2f, 4f), (5f, -3f), (1f, 1f) };

        for (int i = 0; i < points.Length; i++)
        {
            for (int j = 0; j < points.Length; j++)
            {
                var a = points[i];
                var b = points[j];

                float screenA = GodotScreen(projection, camera, projection.ToScene(a.X, a.Y)).Y;
                float screenB = GodotScreen(projection, camera, projection.ToScene(b.X, b.Y)).Y;

                float depthA = projection.ToScene(a.X, a.Y).Y;
                float depthB = projection.ToScene(b.X, b.Y).Y;

                if (screenA < screenB - Tolerance)
                    Assert.True(depthA < depthB, $"{a} is behind {b} on screen but not in depth.");
                else if (screenA > screenB + Tolerance)
                    Assert.True(depthA > depthB, $"{a} is in front of {b} on screen but not in depth.");
            }
        }
    }

    [Fact]
    public void SortBiasBreaksTiesWithoutMovingAnything()
    {
        // Used for the orderings the original expressed as separate draw passes: shadows beneath
        // objects, tile tops above them.
        var projection = new WorldProjection(0.6f);
        var camera = new Vector2(0f, 0f);

        var normal = projection.ToScene(2f, 3f);
        var beneath = projection.ToScene(2f, 3f, sortBias: -1f);

        Assert.True(beneath.Y < normal.Y);
        Assert.Equal(GodotScreen(projection, camera, normal), GodotScreen(projection, camera, beneath));
    }

    [Theory]
    [InlineData(0f)]
    [InlineData(2.2f)]
    [InlineData(5.4977871f)]
    public void ScreenToWorldOffsetInvertsTheProjection(float angle)
    {
        // Used for aiming: the shot angle comes from the cursor's offset from the player, and that
        // has to be turned back into world space through the same rotation.
        var projection = new WorldProjection(angle);

        foreach (var world in new[] { new Vector2(1f, 0f), new Vector2(0f, 1f), new Vector2(-3.5f, 2.25f) })
        {
            var screenPixels = new Vector2(
                world.Dot(new Vector2(Mathf.Cos(angle), Mathf.Sin(angle))),
                world.Dot(new Vector2(Mathf.Cos(angle + Mathf.Pi / 2f), Mathf.Sin(angle + Mathf.Pi / 2f))))
                * WorldProjection.PixelsPerTile;

            var recovered = projection.ScreenToWorldOffset(screenPixels);
            Assert.Equal(world.X, recovered.X, 3);
            Assert.Equal(world.Y, recovered.Y, 3);
        }
    }
}

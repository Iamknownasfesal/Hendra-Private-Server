using System;
using Hendra.Resources;
using Hendra.World;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// Projectile flight paths.
///
/// The server computes the same closed form in <c>Projectile.GetPosition</c>, so the two agree on
/// where a bullet is without exchanging anything after the shot. That formula is transcribed here
/// rather than compiled in, because pulling in the server's Projectile drags most of its realm
/// layer with it.
/// </summary>
public sealed class ProjectileTests
{
    /// <summary>
    /// wServer's Projectile.GetPosition, transcribed. Deliberately kept in double precision and in
    /// the server's exact expression order, so a divergence shows up as a real difference rather
    /// than as rounding.
    /// </summary>
    private static (double X, double Y) ServerPosition(
        ProjectileDesc desc, double startX, double startY, double angle, int bulletId, long elapsed)
    {
        double x = startX;
        double y = startY;
        double dist = elapsed * desc.Speed / 10000.0;
        double period = bulletId % 2 == 0 ? 0 : Math.PI;

        if (desc.Wavy)
        {
            double theta = angle + (Math.PI * 64) * Math.Sin(period + 6 * Math.PI * (elapsed / 1000.0));
            x += dist * Math.Cos(theta);
            y += dist * Math.Sin(theta);
        }
        else if (desc.Parametric)
        {
            double theta = (double)elapsed / desc.LifetimeMs * 2 * Math.PI;
            double a = Math.Sin(theta) * (bulletId % 2 != 0 ? 1 : -1);
            double b = Math.Sin(theta * 2) * (bulletId % 4 < 2 ? 1 : -1);
            double c = Math.Sin(angle);
            double d = Math.Cos(angle);
            x += (a * d - b * c) * desc.Magnitude;
            y += (a * c + b * d) * desc.Magnitude;
        }
        else
        {
            if (desc.Boomerang)
            {
                double half = desc.LifetimeMs * desc.Speed / 10000.0 / 2;
                if (dist > half)
                    dist = half - (dist - half);
            }

            x += dist * Math.Cos(angle);
            y += dist * Math.Sin(angle);

            if (desc.Amplitude != 0)
            {
                double d = desc.Amplitude *
                    Math.Sin(period + (double)elapsed / desc.LifetimeMs * desc.Frequency * 2 * Math.PI);
                x += d * Math.Cos(angle + Math.PI / 2);
                y += d * Math.Sin(angle + Math.PI / 2);
            }
        }

        return (x, y);
    }

    private static ProjectileDesc Straight() => new()
    {
        Speed = 80f,
        LifetimeMs = 2000,
    };

    private static void AssertMatchesServer(ProjectileDesc desc, byte bulletId, float angle)
    {
        for (int elapsed = 0; elapsed <= desc.LifetimeMs; elapsed += 37)
        {
            Projectile.PositionAt(desc, 10f, 20f, angle, bulletId, elapsed, out float x, out float y);
            var (sx, sy) = ServerPosition(desc, 10.0, 20.0, angle, bulletId, elapsed);

            // Compared as an absolute distance rather than by decimal places: the client works in
            // float and the server in double, so after twenty tiles of travel the two differ by a
            // millionth of a tile, and a decimal-place comparison fails whenever that lands on a
            // rounding boundary. A thousandth of a tile is far tighter than any formula difference
            // could be.
            double drift = Math.Sqrt((x - sx) * (x - sx) + (y - sy) * (y - sy));
            Assert.True(drift < 1e-3,
                $"Diverged from the server by {drift:F6} tiles at {elapsed}ms (bullet {bulletId}).");
        }
    }

    [Theory]
    [InlineData(0)]
    [InlineData(1)]
    [InlineData(7)]
    public void StraightShotsMatchTheServer(byte bulletId) =>
        AssertMatchesServer(Straight(), bulletId, 0.9f);

    [Theory]
    [InlineData(0)]
    [InlineData(1)]
    public void BoomerangsMatchTheServer(byte bulletId)
    {
        var desc = Straight();
        desc.Boomerang = true;
        AssertMatchesServer(desc, bulletId, 2.1f);
    }

    [Theory]
    [InlineData(0)]
    [InlineData(1)]
    [InlineData(2)]
    [InlineData(3)]
    public void ParametricShotsMatchTheServer(byte bulletId)
    {
        var desc = Straight();
        desc.Parametric = true;
        desc.Magnitude = 3f;
        AssertMatchesServer(desc, bulletId, -0.4f);
    }

    [Theory]
    [InlineData(0)]
    [InlineData(1)]
    public void SineOffsetShotsMatchTheServer(byte bulletId)
    {
        var desc = Straight();
        desc.Amplitude = 0.6f;
        desc.Frequency = 2f;
        AssertMatchesServer(desc, bulletId, 1.7f);
    }

    [Fact]
    public void WavyShotsDivergeFromTheServerOnPurpose()
    {
        // The one place the two do not agree. The client wobbles by PI/64, about three degrees;
        // the server sweeps by PI*64, which is a mistyped divide. The client's is the intended
        // visual and the one that matters, because projectile hits are client-authoritative and
        // the server never consults its own path for anything observable.
        //
        // Asserted rather than left implicit, so that if the server is ever fixed this fails and
        // prompts the decision instead of silently drifting.
        var desc = Straight();
        desc.Wavy = true;

        // Sampled across the whole flight rather than at one instant. The two formulas coincide
        // wherever the shared sine term happens to be zero -- at 500ms, for example, the argument
        // is exactly 4*pi -- so a single sample can agree by accident.
        double worst = 0;
        for (int elapsed = 0; elapsed <= desc.LifetimeMs; elapsed += 17)
        {
            Projectile.PositionAt(desc, 0f, 0f, 0f, bulletId: 1, elapsedMs: elapsed, out float x, out float y);
            var (sx, sy) = ServerPosition(desc, 0, 0, 0, 1, elapsed);
            worst = Math.Max(worst, Math.Sqrt((x - sx) * (x - sx) + (y - sy) * (y - sy)));
        }

        Assert.True(worst > 1.0,
            $"Expected the wavy paths to disagree, but they never differed by more than {worst:F4} tiles.");
    }

    [Fact]
    public void WavyShotsStayCloseToTheirNominalHeading()
    {
        // What PI/64 actually buys: a visible weave that never loses the aim. If this ever grew
        // large the projectile would spiral instead.
        var desc = Straight();
        desc.Wavy = true;

        for (int elapsed = 100; elapsed <= desc.LifetimeMs; elapsed += 50)
        {
            Projectile.PositionAt(desc, 0f, 0f, 0f, 1, elapsed, out float x, out float y);
            float drift = Math.Abs(MathF.Atan2(y, x));
            Assert.True(drift < 0.1f, $"Drifted {drift:F3} rad off heading at {elapsed}ms.");
        }
    }

    [Fact]
    public void ABoomerangReturnsToWhereItStarted()
    {
        var desc = Straight();
        desc.Boomerang = true;

        Projectile.PositionAt(desc, 5f, 5f, 0f, 0, desc.LifetimeMs, out float x, out float y);

        Assert.Equal(5f, x, 2);
        Assert.Equal(5f, y, 2);
    }

    [Fact]
    public void ConsecutiveBulletIdsWeaveInOppositeDirections()
    {
        // Alternating the phase by bullet id is what makes a wavy volley spread out instead of
        // travelling as a single thick line.
        var desc = Straight();
        desc.Amplitude = 0.5f;
        desc.Frequency = 1f;

        Projectile.PositionAt(desc, 0f, 0f, 0f, 0, 200, out _, out float evenY);
        Projectile.PositionAt(desc, 0f, 0f, 0f, 1, 200, out _, out float oddY);

        Assert.True(evenY * oddY < 0f, "Even and odd bullet ids should be displaced to opposite sides.");
    }

    [Fact]
    public void SpeedIsTilesPerTenSeconds()
    {
        // The unit is easy to get wrong: Speed 100 travels ten tiles in a second, not a hundred.
        var desc = new ProjectileDesc { Speed = 100f, LifetimeMs = 10_000 };

        Projectile.PositionAt(desc, 0f, 0f, 0f, 0, 1000, out float x, out _);
        Assert.Equal(10f, x, 3);
    }
}

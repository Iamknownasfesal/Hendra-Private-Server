using System;
using Hendra.Data;
using Hendra.Net.Packets;
using Hendra.Resources;
using Hendra.World;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// The particle system.
///
/// Nothing here affects the server, so none of it can get anyone disconnected. What it can do is
/// bury the frame rate — the original's effects scaled their particle count with the frame rate and
/// bounded only some of their kinds — and leave particles chasing targets that have gone away. Both
/// are what these cover.
/// </summary>
public sealed class ParticleTests
{
    private static GameMap NewMap()
    {
        var map = new GameMap(new GameData());
        map.Reset(64, 64, "test");
        return map;
    }

    private static Entity AddEntity(GameMap map, int objectId, float x, float y)
    {
        var entity = new Entity { ObjectId = objectId };
        map.Add(entity, x, y);
        return entity;
    }

    private static ShowEffectPacket Effect(ShowEffectType type, int targetId = 0, float p1x = 0f, float p1y = 0f, float p2x = 0f, float p2y = 0f)
        => new()
        {
            EffectType = type,
            TargetObjectId = targetId,
            Pos1 = new WorldPos(p1x, p1y),
            Pos2 = new WorldPos(p2x, p2y),
            Color = new Argb(0xFF00FF00),
        };

    /// <summary>
    /// The claim that motivated shedding trails on an interval rather than once per frame.
    ///
    /// The original emitted one trail particle per rendered frame, so the same nova was roughly
    /// three times as dense at a hundred and forty frames a second as at fifty. Here the two rates
    /// should agree closely.
    /// </summary>
    [Fact]
    public void TrailDensityDoesNotDependOnFrameRate()
    {
        int Run(int stepMs)
        {
            var map = NewMap();
            AddEntity(map, 1, 20f, 20f);

            var particles = new ParticleSystem(map, seed: 7);
            particles.Show(Effect(ShowEffectType.Nova, targetId: 1, p1x: 4f), nowMs: 0);

            int peak = 0;
            for (int elapsed = 0; elapsed < 200; elapsed += stepMs)
            {
                particles.Update(elapsed, stepMs);
                peak = Math.Max(peak, particles.Count);
            }

            return peak;
        }

        int slow = Run(33);
        int fast = Run(7);

        Assert.InRange(fast, slow * 0.8, slow * 1.2);
    }

    /// <summary>
    /// The cap holds however much is thrown at it.
    ///
    /// The original had per-class counters — two hundred flow particles, four hundred explosion
    /// particles — which left every other kind unbounded.
    /// </summary>
    [Fact]
    public void TotalIsCapped()
    {
        var map = NewMap();
        AddEntity(map, 1, 20f, 20f);

        var particles = new ParticleSystem(map, seed: 3);

        for (int i = 0; i < 500; i++)
        {
            particles.Show(Effect(ShowEffectType.Nova, targetId: 1, p1x: 20f), nowMs: i * 16);
            particles.Update(i * 16, 16);
            Assert.True(particles.Count <= ParticleSystem.MaxParticles);
        }

        // And it really was pressed up against the ceiling, so the assertion above was not vacuous.
        // Not exactly at it: the last frame retires whatever expired on it before anything new is
        // admitted.
        Assert.True(particles.Count > ParticleSystem.MaxParticles * 0.9,
            $"only reached {particles.Count} particles");
    }

    /// <summary>A flowing particle reaches what it is being drawn into, and stops existing there.</summary>
    [Fact]
    public void FlowParticlesReachTheirTarget()
    {
        var map = NewMap();
        AddEntity(map, 1, 20f, 20f);

        var particles = new ParticleSystem(map, seed: 11);
        particles.Show(Effect(ShowEffectType.Flow, targetId: 1, p1x: 26f, p1y: 20f), nowMs: 0);

        Assert.True(particles.Count > 0);

        for (int elapsed = 0; elapsed < 10_000 && particles.Count > 0; elapsed += 16)
            particles.Update(elapsed, 16);

        Assert.Equal(0, particles.Count);
    }

    /// <summary>
    /// A flowing particle catches a target that is running away from it.
    ///
    /// Its allowance only ever shrinks, which is what stops it settling into an orbit at whatever
    /// distance the target is holding.
    /// </summary>
    [Fact]
    public void FlowParticlesCatchAMovingTarget()
    {
        var map = NewMap();
        var target = AddEntity(map, 1, 20f, 20f);

        var particles = new ParticleSystem(map, seed: 13);
        particles.Show(Effect(ShowEffectType.Flow, targetId: 1, p1x: 24f, p1y: 20f), nowMs: 0);

        for (int elapsed = 0; elapsed < 10_000 && particles.Count > 0; elapsed += 16)
        {
            // Half a tile a second, away from where the particles started.
            target.X -= 0.008f;
            particles.Update(elapsed, 16);
        }

        Assert.Equal(0, particles.Count);
    }

    /// <summary>Particles tied to an entity go away with it rather than hanging in space.</summary>
    [Fact]
    public void ParticlesDieWithTheirAnchor()
    {
        var map = NewMap();
        AddEntity(map, 1, 20f, 20f);

        var particles = new ParticleSystem(map, seed: 17);
        particles.Show(Effect(ShowEffectType.Heal, targetId: 1), nowMs: 0);
        particles.Update(16, 16);

        Assert.True(particles.Count > 0);

        map.Remove(1);
        particles.Update(32, 16);

        Assert.Equal(0, particles.Count);
    }

    /// <summary>An effect whose target has already gone produces nothing, and does not throw.</summary>
    [Theory]
    [InlineData(ShowEffectType.Heal)]
    [InlineData(ShowEffectType.Nova)]
    [InlineData(ShowEffectType.Poison)]
    [InlineData(ShowEffectType.Ring)]
    [InlineData(ShowEffectType.Lightning)]
    [InlineData(ShowEffectType.ConeBlast)]
    [InlineData(ShowEffectType.Flow)]
    [InlineData(ShowEffectType.Shocker)]
    [InlineData(ShowEffectType.RisingFury)]
    public void EffectsWithNoTargetDoNothing(ShowEffectType type)
    {
        var map = NewMap();
        var particles = new ParticleSystem(map, seed: 19);

        particles.Show(Effect(type, targetId: 999, p1x: 3f, p1y: 3f, p2x: 3f), nowMs: 0);
        particles.Update(16, 16);

        Assert.Equal(0, particles.Count);
    }

    /// <summary>
    /// A path particle finishes where it was aimed.
    ///
    /// The position is rebuilt from the ideal line each frame rather than integrated, so this holds
    /// whatever the frame times were along the way.
    /// </summary>
    [Fact]
    public void PathParticlesArriveWhereTheyWereAimed()
    {
        var map = NewMap();
        AddEntity(map, 1, 10f, 10f);

        var particles = new ParticleSystem(map, seed: 23);

        // Collapse runs its motes inward from the edge, so they should converge on the centre.
        particles.Show(Effect(ShowEffectType.Collapse, p1x: 10f, p1y: 10f, p2x: 14f, p2y: 10f), nowMs: 0);

        // The effect lasts 200ms; stop just short so the motes are still alive to be inspected.
        for (int elapsed = 0; elapsed < 190; elapsed += 10)
            particles.Update(elapsed, 10);

        int arrived = 0;
        foreach (var particle in particles.Particles)
        {
            if (particle.Motion != ParticleMotion.Path)
                continue;

            arrived++;
            Assert.True(MathF.Abs(particle.X - 10f) < 0.3f, $"x was {particle.X}");
            Assert.True(MathF.Abs(particle.Y - 10f) < 0.3f, $"y was {particle.Y}");
        }

        Assert.True(arrived > 0, "no path particles survived to be checked");
    }

    /// <summary>
    /// Camera shake winds down.
    ///
    /// In the original it did not: the Jitter effect set a flag that has no other assignment
    /// anywhere in the client, so one earthquake left the camera shaking until the session ended.
    /// </summary>
    [Fact]
    public void JitterDecaysToNothing()
    {
        var map = NewMap();
        var particles = new ParticleSystem(map, seed: 29);

        particles.Show(Effect(ShowEffectType.Jitter), nowMs: 0);
        Assert.True(particles.Jitter > 0f);

        for (int elapsed = 0; elapsed < 5_000; elapsed += 16)
            particles.Update(elapsed, 16);

        Assert.Equal(0f, particles.Jitter);
    }

    /// <summary>
    /// The flash pulse: a half-strength sine, for as many cycles as were asked for and no more.
    /// </summary>
    [Fact]
    public void FlashPulsesThenStops()
    {
        var entity = new Entity();
        entity.StartFlash(nowMs: 1000, color: 0xFF0000, periodMs: 200, repeats: 3);

        Assert.Equal(0f, entity.FlashStrength(1000, out _), 3);
        Assert.Equal(0.5f, entity.FlashStrength(1100, out int color), 3);
        Assert.Equal(0xFF0000, color);

        // Still going in the third cycle.
        Assert.Equal(0.5f, entity.FlashStrength(1500, out _), 3);

        // And done once the three cycles are up.
        Assert.Equal(0f, entity.FlashStrength(1700, out _), 3);
    }

    /// <summary>A flash with a nonsense period is refused rather than dividing by zero.</summary>
    [Fact]
    public void FlashIgnoresDegenerateArguments()
    {
        var entity = new Entity();

        entity.StartFlash(nowMs: 0, color: 0xFFFFFF, periodMs: 0, repeats: 4);
        Assert.Equal(0f, entity.FlashStrength(10, out _));

        entity.StartFlash(nowMs: 0, color: 0xFFFFFF, periodMs: 100, repeats: 0);
        Assert.Equal(0f, entity.FlashStrength(10, out _));
    }

    /// <summary>
    /// A hit spray is thrown away from the shot rather than along it.
    /// </summary>
    [Fact]
    public void HitSprayTravelsAwayFromTheShot()
    {
        var map = NewMap();
        var particles = new ParticleSystem(map, seed: 31);

        // Fired due east, fast enough that the spread cannot flip the direction.
        particles.Hit(20f, 20f, new[] { 0xFF0000 }, size: 100, count: 20, angle: 0f, speed: 1000f);

        Assert.Equal(20, particles.Count);

        foreach (var particle in particles.Particles)
            Assert.True(particle.Dx < 0f, $"dx was {particle.Dx}");
    }

    /// <summary>An object with no colours to throw does not produce colourless particles.</summary>
    [Fact]
    public void DebrisNeedsAPalette()
    {
        var map = NewMap();
        var particles = new ParticleSystem(map, seed: 37);

        particles.Explode(20f, 20f, Array.Empty<int>(), size: 100, count: 20);
        particles.Hit(20f, 20f, null, size: 100, count: 20, angle: 0f, speed: 100f);

        Assert.Equal(0, particles.Count);
    }
}

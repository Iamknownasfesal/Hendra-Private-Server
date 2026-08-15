using System;
using System.Collections.Generic;
using Hendra.Net.Packets;

namespace Hendra.World;

/// <summary>How a particle finds its position each frame.</summary>
public enum ParticleMotion : byte
{
    /// <summary>Straight line at a constant velocity.</summary>
    Drift,

    /// <summary>Held at a fixed angle and distance from an entity, rising as it goes.</summary>
    Orbit,

    /// <summary>Travels from a start point to an end point over its whole lifetime.</summary>
    Path,

    /// <summary>Accelerates towards an entity and dies on arrival.</summary>
    Flow,
}

/// <summary>
/// One live particle.
/// </summary>
/// <remarks>
/// <para>
/// A struct in a flat list, not an object in the entity dictionary. The original made every particle
/// a <c>BasicObject</c>, gave it a fake object id, inserted it into the map's object dictionary and
/// its square's occupancy list, and appended a fill, a path and an end-fill to the display list for
/// it every frame. A nova at full radius is several hundred of those, each also spawning a trail
/// particle per frame.
/// </para>
/// <para>
/// <see cref="Size"/> is in the original's units, where 100 is five screen pixels — the argument
/// values here are lifted straight from the AS3 effects, so keeping the unit avoids rescaling every
/// constant and getting one of them wrong.
/// </para>
/// </remarks>
public struct Particle
{
    public float X;
    public float Y;

    /// <summary>Height above the ground, in tiles.</summary>
    public float Z;

    /// <summary>Packed 0xRRGGBB.</summary>
    public int Color;

    public ParticleMotion Motion;

    public int LifetimeMs;
    public int TimeLeftMs;

    /// <summary>Current size, in the original's units: 100 is five screen pixels.</summary>
    public float Size;

    public float InitialSize;

    /// <summary>Whether the particle shrinks to nothing over its life.</summary>
    public bool Shrinks;

    // Drift velocity, in tiles per second.
    public float Dx;
    public float Dy;
    public float Dz;

    /// <summary>The entity an orbiting or flowing particle is tied to.</summary>
    public int AnchorId;

    /// <summary>Orbit bearing, in radians.</summary>
    public float Angle;

    /// <summary>Orbit radius, or a flow particle's remaining allowance, in tiles.</summary>
    public float Distance;

    /// <summary>How fast an orbiting particle sweeps around its anchor, in radians per second.</summary>
    public float Spin;

    /// <summary>A flow particle's current speed, in tiles per second.</summary>
    public float Speed;

    // Path endpoints and the position along the ideal straight line, before deflection.
    public float PathX;
    public float PathY;
    public float DeflectX;
    public float DeflectY;
    public float Period;

    /// <summary>Peak height of a thrown particle's arc, in tiles. Zero for a flat path.</summary>
    public float ArcHeight;

    /// <summary>Colour of the trail this particle sheds, or -1 for none.</summary>
    public int TrailColor;

    public int TrailLifetimeMs;

    /// <summary>Particle-local countdown to the next trail spark.</summary>
    public int NextTrailMs;
}

/// <summary>
/// Every visual effect in the game: the particles themselves, and the emitters that produce them
/// over time.
/// </summary>
/// <remarks>
/// <para>
/// The original spread this over some thirty classes, one per effect and one per particle kind, each
/// a display object with its own update and draw. Almost all of them differ only in their constants:
/// there are four distinct motions between them, so those are the enum above and everything else is
/// a table of numbers. The numbers themselves are the original's, taken effect by effect.
/// </para>
/// <para>
/// Two things here are deliberately not faithful. Trail particles are shed on a fixed interval of
/// particle time rather than one per rendered frame — the original's density scaled with frame rate,
/// so the same nova was three times denser on a fast machine. And the total is capped: the original
/// had per-class static counters (200 flow particles, 400 explosion particles) which bounded some
/// kinds and not others, so a room full of shocked enemies could bury the frame rate.
/// </para>
/// <para>
/// The random source is private and has nothing to do with <see cref="Core.MinstdRandom"/>. That one
/// is shared with the server and stepped in lockstep with it; drawing from it to jitter a spark
/// would desynchronise damage.
/// </para>
/// </remarks>
public sealed class ParticleSystem
{
    /// <summary>
    /// The ceiling on live particles. Emission is refused past it rather than trimming the oldest,
    /// which keeps whatever is already on screen coherent instead of thinning it out mid-flight.
    /// </summary>
    public const int MaxParticles = 3000;

    /// <summary>How often a trailing particle sheds a spark, in milliseconds of its own life.</summary>
    private const int TrailIntervalMs = 16;

    /// <summary>Converts the original's per-millisecond velocity units into tiles per second.</summary>
    /// <remarks>
    /// Several particle kinds advance by <c>v * deltaMs * 0.008</c>, which is 8·v tiles per second.
    /// Others use <c>v * deltaMs / 1000</c>, which is v tiles per second. Rather than carry the
    /// distinction into the particle, the faster kinds are converted once at emission.
    /// </remarks>
    private const float FastUnitsToTilesPerSecond = 8f;

    private readonly GameMap _map;
    private readonly Random _random;
    private readonly List<Particle> _particles = new(512);
    private readonly List<Emitter> _emitters = new(16);

    /// <summary>Emitters that produce particles over time rather than all at once.</summary>
    private struct Emitter
    {
        public EmitterKind Kind;
        public int AnchorId;
        public int Color;
        public int IntervalMs;
        public int NextMs;
        public int Remaining;
    }

    private enum EmitterKind : byte
    {
        Shocker,
        RisingFury,
        LevelUp,
    }

    public ParticleSystem(GameMap map, int seed = 0)
    {
        _map = map;
        _random = seed == 0 ? new Random() : new Random(seed);
    }

    public IReadOnlyList<Particle> Particles => _particles;

    public int Count => _particles.Count;

    /// <summary>Camera shake, in tiles. Non-zero while a Jitter effect is running down.</summary>
    public float Jitter { get; private set; }

    /// <summary>Drops everything. Called on a map change, where nothing in flight still applies.</summary>
    public void Clear()
    {
        _particles.Clear();
        _emitters.Clear();
        Jitter = 0f;
        _jittering = false;
    }

    // ----------------------------------------------------------------------------------------
    // Simulation
    // ----------------------------------------------------------------------------------------

    public void Update(int nowMs, int deltaMs)
    {
        if (deltaMs <= 0)
            return;

        UpdateJitter(deltaMs);
        UpdateEmitters(nowMs);

        float seconds = deltaMs / 1000f;

        // Backwards, so removing by swapping with the last entry never skips one.
        for (int i = _particles.Count - 1; i >= 0; i--)
        {
            var particle = _particles[i];

            if (Advance(ref particle, deltaMs, seconds))
                _particles[i] = particle;
            else
                RemoveAt(i);
        }
    }

    private void RemoveAt(int index)
    {
        _particles[index] = _particles[^1];
        _particles.RemoveAt(_particles.Count - 1);
    }

    /// <summary>Advances one particle. False when it has expired.</summary>
    private bool Advance(ref Particle particle, int deltaMs, float seconds)
    {
        particle.TimeLeftMs -= deltaMs;
        if (particle.TimeLeftMs <= 0)
            return false;

        switch (particle.Motion)
        {
            case ParticleMotion.Drift:
                particle.X += particle.Dx * seconds;
                particle.Y += particle.Dy * seconds;
                particle.Z += particle.Dz * seconds;
                break;

            case ParticleMotion.Orbit:
            {
                var anchor = _map.GetEntity(particle.AnchorId);
                if (anchor == null)
                    return false;

                particle.Angle += particle.Spin * seconds;
                particle.X = anchor.X + particle.Distance * MathF.Cos(particle.Angle);
                particle.Y = anchor.Y + particle.Distance * MathF.Sin(particle.Angle);
                particle.Z += particle.Dz * seconds;
                break;
            }

            case ParticleMotion.Path:
                AdvancePath(ref particle, deltaMs, seconds);
                break;

            case ParticleMotion.Flow:
                if (!AdvanceFlow(ref particle, seconds))
                    return false;
                break;
        }

        if (particle.Shrinks && particle.LifetimeMs > 0)
            particle.Size = particle.TimeLeftMs / (float)particle.LifetimeMs * particle.InitialSize;

        return true;
    }

    /// <summary>
    /// A particle travelling a fixed line, optionally weaving across it and shedding sparks.
    /// </summary>
    /// <remarks>
    /// The ideal position is tracked separately from the drawn one so that the deflection is a
    /// displacement from the line rather than an accumulating drift along it.
    /// </remarks>
    private void AdvancePath(ref Particle particle, int deltaMs, float seconds)
    {
        particle.PathX += particle.Dx * seconds;
        particle.PathY += particle.Dy * seconds;

        if (particle.Period > 0f)
        {
            // Driven by time remaining, not elapsed, which is what the original used -- the weave
            // therefore ends mid-swing rather than settling back onto the line.
            float offset = MathF.Sin(particle.TimeLeftMs / 1000f / particle.Period);
            particle.X = particle.PathX + particle.DeflectX * offset;
            particle.Y = particle.PathY + particle.DeflectY * offset;
        }
        else
        {
            particle.X = particle.PathX;
            particle.Y = particle.PathY;
        }

        if (particle.ArcHeight > 0f && particle.LifetimeMs > 0)
        {
            float phase = particle.TimeLeftMs / (float)particle.LifetimeMs;
            particle.Z = MathF.Sin(phase * MathF.PI) * particle.ArcHeight;
        }

        if (particle.TrailColor < 0)
            return;

        // A loop rather than a single test, so a long frame sheds the sparks it owes instead of
        // dropping them. That is what makes the density the same at thirty frames a second as at
        // a hundred and forty; the original emitted exactly one per frame and so drew a nova three
        // times denser on a fast machine.
        particle.NextTrailMs -= deltaMs;
        while (particle.NextTrailMs <= 0)
        {
            particle.NextTrailMs += TrailIntervalMs;

            // The trail grows with the height of whatever is shedding it, which is what makes a
            // thrown object read as rising and falling even though the object itself is not drawn.
            float size = 100f * (particle.Z + 1f);

            Emit(new Particle
            {
                X = particle.X,
                Y = particle.Y,
                Z = particle.Z,
                Color = particle.TrailColor,
                Motion = ParticleMotion.Drift,
                LifetimeMs = particle.TrailLifetimeMs,
                TimeLeftMs = particle.TrailLifetimeMs,
                Size = size,
                InitialSize = size,
                Shrinks = true,
                Dx = PlusMinus(1f),
                Dy = PlusMinus(1f),
                TrailColor = -1,
            });
        }
    }

    /// <summary>
    /// A particle being drawn into an entity, accelerating as it closes.
    /// </summary>
    /// <remarks>
    /// The allowance in <see cref="Particle.Distance"/> only ever shrinks, so a particle can never
    /// be pushed back out by its target moving away from it. Without that it would orbit forever
    /// around a fleeing target instead of catching it.
    /// </remarks>
    private bool AdvanceFlow(ref Particle particle, float seconds)
    {
        const float Acceleration = 8f;

        var anchor = _map.GetEntity(particle.AnchorId);
        if (anchor == null)
            return false;

        float dx = anchor.X - particle.X;
        float dy = anchor.Y - particle.Y;
        float distance = MathF.Sqrt(dx * dx + dy * dy);

        if (distance < 0.5f)
            return false;

        particle.Speed += Acceleration * seconds;
        particle.Distance -= particle.Speed * seconds;

        float remaining = MathF.Min(distance - particle.Speed * seconds, particle.Distance);
        float scale = remaining / distance;

        particle.X = anchor.X - dx * scale;
        particle.Y = anchor.Y - dy * scale;
        return true;
    }

    /// <summary>How far the camera may eventually wander, in tiles.</summary>
    /// <remarks><c>Camera.MAX_JITTER</c> (<c>Camera.as:20</c>).</remarks>
    private const float MaxJitter = 0.5f;

    /// <summary>How long the shake takes to reach <see cref="MaxJitter"/>, in milliseconds.</summary>
    /// <remarks><c>Camera.JITTER_BUILDUP_MS</c> (<c>Camera.as:21</c>).</remarks>
    private const int JitterBuildupMs = 10_000;

    /// <summary>Whether an earthquake is still building.</summary>
    private bool _jittering;

    /// <summary>
    /// Builds the camera shake up.
    /// </summary>
    /// <remarks>
    /// A linear ramp rather than a decay, which is what <c>Camera.update</c> (<c>Camera.as:112-119</c>)
    /// does: the shake starts at nothing and grows towards half a tile over ten seconds. A realm
    /// quake reconnects everybody after eight of those, so what a player actually feels is a rumble
    /// that keeps getting worse right up until the castle takes them — which is the point of it.
    /// </remarks>
    private void UpdateJitter(int deltaMs)
    {
        if (!_jittering)
            return;

        Jitter = MathF.Min(MaxJitter, Jitter + deltaMs * MaxJitter / JitterBuildupMs);
    }

    private void UpdateEmitters(int nowMs)
    {
        for (int i = _emitters.Count - 1; i >= 0; i--)
        {
            var emitter = _emitters[i];

            var anchor = _map.GetEntity(emitter.AnchorId);
            if (anchor == null || emitter.Remaining <= 0)
            {
                _emitters[i] = _emitters[^1];
                _emitters.RemoveAt(_emitters.Count - 1);
                continue;
            }

            if (nowMs < emitter.NextMs)
            {
                _emitters[i] = emitter;
                continue;
            }

            emitter.NextMs = nowMs + emitter.IntervalMs;
            emitter.Remaining--;

            switch (emitter.Kind)
            {
                case EmitterKind.Shocker:
                    EmitShockArc(anchor, emitter.Color);
                    break;

                case EmitterKind.RisingFury:
                    EmitFurySpark(anchor, emitter.Color);
                    break;

                case EmitterKind.LevelUp:
                    EmitLevelUpPair(anchor, emitter.Color, emitter.Remaining);
                    break;
            }

            _emitters[i] = emitter;
        }
    }

    // ----------------------------------------------------------------------------------------
    // Effects
    // ----------------------------------------------------------------------------------------

    /// <summary>
    /// Runs whatever a ShowEffect asks for.
    /// </summary>
    /// <remarks>
    /// Both position fields are overloaded per effect type, and not consistently: sometimes a
    /// radius arrives in <c>Pos1.X</c>, sometimes as the distance between the two points, and
    /// sometimes in <c>Pos2.X</c>. The mapping is the original's and is documented on
    /// <see cref="ShowEffectType"/>.
    /// </remarks>
    /// <summary>
    /// The sparks a projectile leaves behind it.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The original spawns three of these per frame for every trailing projectile in flight, which
    /// ties the density of a trail to the frame rate: the same shot leaves a thicker trail on a
    /// faster machine. Here it is spawned on a clock instead, so a trail looks the same everywhere
    /// and a slow machine is not also given a thinner one.
    /// </para>
    /// <para>
    /// Everything else is the original's: half alpha, a lifetime of six hundred milliseconds unless
    /// the projectile names its own, and a scatter of a few tenths of a tile so the trail has width.
    /// </para>
    /// </remarks>
    public void ProjectileTrail(float x, float y, float z, int color, int lifetimeMs)
    {
        Emit(new Particle
        {
            X = x + Scatter(0.15f),
            Y = y + Scatter(0.15f),
            Z = z,
            Color = color,
            Motion = ParticleMotion.Drift,
            LifetimeMs = lifetimeMs,
            TimeLeftMs = lifetimeMs,
            Size = 130f,
            InitialSize = 130f,
            Shrinks = true,
            TrailColor = -1,
        });
    }

    /// <summary>
    /// The burst where a shot lands.
    /// </summary>
    /// <remarks>
    /// Not in the original, which shows nothing at all when a bullet stops -- it simply vanishes.
    /// A few sparks thrown back along the shot's own direction is the cheapest way to make hitting
    /// something feel like contact rather than like the projectile being switched off.
    /// </remarks>
    public void Impact(float x, float y, float z, float angle, int color, int count = 5)
    {
        for (int i = 0; i < count; i++)
        {
            // Thrown back into the quadrant the shot came from, spread across a half turn, so the
            // burst reads as a splash off the surface rather than as a ring.
            float away = angle + MathF.PI + (Next() - 0.5f) * MathF.PI;
            float speed = 0.6f + Next() * 1.4f;

            Emit(new Particle
            {
                X = x,
                Y = y,
                Z = z,
                Dx = MathF.Cos(away) * speed,
                Dy = MathF.Sin(away) * speed,
                Dz = 0.4f + Next(),
                Color = color,
                Motion = ParticleMotion.Drift,
                LifetimeMs = 320,
                TimeLeftMs = 320,
                Size = 150f,
                InitialSize = 150f,
                Shrinks = true,
                TrailColor = -1,
            });
        }
    }

    /// <summary>
    /// A blast: a ring of sparks thrown outward from where something went off.
    /// </summary>
    /// <remarks>
    /// Drawn from the impact spray rather than a new kind of particle, so an explosion looks like a
    /// great many small hits at once -- which is what it is. The count follows the radius, because a
    /// grenade and a boss's detonation should not throw the same handful of sparks.
    /// </remarks>
    public void Blast(float x, float y, float radius, int origType)
    {
        int arms = Math.Clamp((int)(radius * 6f), 8, 32);

        for (int i = 0; i < arms; i++)
        {
            float angle = i / (float)arms * MathF.Tau;

            Impact(
                x + MathF.Cos(angle) * radius * 0.6f,
                y + MathF.Sin(angle) * radius * 0.6f,
                0.4f, angle, 0xFF6A2A, 3);
        }
    }

    private float Scatter(float amount) => (Next() - 0.5f) * 2f * amount;

    public void Show(ShowEffectPacket packet, int nowMs)
    {
        int color = (packet.Color.R << 16) | (packet.Color.G << 8) | packet.Color.B;
        var target = _map.GetEntity(packet.TargetObjectId);

        switch (packet.EffectType)
        {
            case ShowEffectType.Heal when target != null:
                Heal(target, color);
                break;

            case ShowEffectType.Teleport:
                Teleport(packet.Pos1.X, packet.Pos1.Y);
                break;

            case ShowEffectType.Stream:
                Stream(packet.Pos1.X, packet.Pos1.Y, packet.Pos2.X, packet.Pos2.Y, color);
                break;

            case ShowEffectType.Throw when target != null:
                Throw(target.X, target.Y, packet.Pos1.X, packet.Pos1.Y, color);
                break;

            case ShowEffectType.Throw:
                Throw(packet.Pos2.X, packet.Pos2.Y, packet.Pos1.X, packet.Pos1.Y, color);
                break;

            case ShowEffectType.Nova when target != null:
                Ring(target.X, target.Y, packet.Pos1.X, color, particleSize: 200f, lifetimeMs: 200, fromCentre: true);
                break;

            case ShowEffectType.Poison when target != null:
                Poison(target.X, target.Y, color);
                break;

            case ShowEffectType.Line when target != null:
                Line(target.X, target.Y, packet.Pos1.X, packet.Pos1.Y, color);
                break;

            case ShowEffectType.Burst:
                Burst(packet.Pos1.X, packet.Pos1.Y, packet.Pos2.X, packet.Pos2.Y, color);
                break;

            case ShowEffectType.Flow when target != null:
                Flow(packet.Pos1.X, packet.Pos1.Y, target, color);
                break;

            case ShowEffectType.Ring when target != null:
                // The particles themselves are sized zero and only their trails show, which is what
                // makes a ring read as a thin band rather than a circle of dots.
                Ring(target.X, target.Y, packet.Pos1.X, color, particleSize: 0f, lifetimeMs: 200, fromCentre: false);
                break;

            case ShowEffectType.Lightning when target != null:
                Lightning(target.X, target.Y, packet.Pos1.X, packet.Pos1.Y, color, (int)packet.Pos2.X);
                break;

            case ShowEffectType.Collapse:
                Collapse(packet.Pos1.X, packet.Pos1.Y, packet.Pos2.X, packet.Pos2.Y, color);
                break;

            case ShowEffectType.ConeBlast when target != null:
                ConeBlast(target.X, target.Y, packet.Pos1.X, packet.Pos1.Y, packet.Pos2.X, color);
                break;

            case ShowEffectType.Jitter:
                StartJitter();
                break;

            case ShowEffectType.Flash when target != null:
                target.StartFlash(nowMs, color, (int)(packet.Pos1.X * 1000f), (int)packet.Pos1.Y);
                break;

            case ShowEffectType.ThrowProjectile:
                Throw(packet.Pos2.X, packet.Pos2.Y, packet.Pos1.X, packet.Pos1.Y, color);
                break;

            case ShowEffectType.Shocker when target != null:
                AddEmitter(EmitterKind.Shocker, target.ObjectId, color, intervalMs: 200, count: 10, nowMs);
                break;

            case ShowEffectType.Shockee when target != null:
                // Twelve toggles at fifty milliseconds each, so six full flashes.
                target.StartFlash(nowMs, 0xFFFFFF, periodMs: 100, repeats: 6);
                break;

            case ShowEffectType.RisingFury when target != null:
            {
                int durationMs = (int)(packet.Pos1.X * 1000f);
                AddEmitter(EmitterKind.RisingFury, target.ObjectId, color,
                    intervalMs: 40, count: Math.Max(1, durationMs / 40), nowMs);
                break;
            }
        }
    }

    /// <summary>Ten motes circling the target and rising.</summary>
    private void Heal(Entity target, int color)
    {
        const int Count = 10;

        for (int i = 0; i < Count; i++)
        {
            float angle = 2f * MathF.PI * i / Count;
            float distance = 0.3f + 0.4f * Next();

            Emit(new Particle
            {
                X = target.X + distance * MathF.Cos(angle),
                Y = target.Y + distance * MathF.Sin(angle),
                Z = Next() * 0.3f,
                Color = color,
                Motion = ParticleMotion.Orbit,
                LifetimeMs = 1000,
                TimeLeftMs = 1000,
                Size = SmallSpray(),
                AnchorId = target.ObjectId,
                Angle = angle,
                Distance = distance,
                Dz = (0.1f + Next() * 0.1f) * FastUnitsToTilesPerSecond,
                TrailColor = -1,
            });
        }
    }

    /// <summary>A column of blue motes rising where someone arrived or left.</summary>
    private void Teleport(float x, float y)
    {
        const int Count = 20;
        const int Blue = 0x0000FF;

        for (int i = 0; i < Count; i++)
        {
            float angle = 2f * MathF.PI * Next();
            float distance = 0.7f * Next();
            int lifetime = 500 + (int)(1000f * Next());

            Emit(new Particle
            {
                X = x + distance * MathF.Cos(angle),
                Y = y + distance * MathF.Sin(angle),
                Color = Blue,
                Motion = ParticleMotion.Drift,
                LifetimeMs = lifetime,
                TimeLeftMs = lifetime,
                Size = 50f,
                Dz = 0.1f * FastUnitsToTilesPerSecond,
                TrailColor = -1,
            });
        }
    }

    /// <summary>A weaving ribbon of motes between two points.</summary>
    private void Stream(float startX, float startY, float endX, float endY, int color)
    {
        const int Count = 5;
        const float DeflectAmount = 0.25f;

        float dx = endX - startX;
        float dy = endY - startY;
        float distance = MathF.Sqrt(dx * dx + dy * dy);
        if (distance <= 0f)
            return;

        for (int i = 0; i < Count; i++)
        {
            int lifetime = 1500 + (int)(3000f * Next());
            float seconds = lifetime / 1000f;

            Emit(new Particle
            {
                X = startX,
                Y = startY,
                Z = 1.85f,
                PathX = startX,
                PathY = startY,
                Color = color,
                Motion = ParticleMotion.Path,
                LifetimeMs = lifetime,
                TimeLeftMs = lifetime,
                Size = SmallSpray(),
                Dx = dx / seconds,
                Dy = dy / seconds,

                // Perpendicular to the direction of travel, scaled so the weave is the same width
                // whatever the distance.
                DeflectX = dy / distance * DeflectAmount,
                DeflectY = -dx / distance * DeflectAmount,
                Period = 0.25f + Next() * 0.5f,
                TrailColor = -1,
            });
        }
    }

    /// <summary>One invisible mote arcing between two points, seen only by the sparks it sheds.</summary>
    private void Throw(float startX, float startY, float endX, float endY, int color)
    {
        const int LifetimeMs = 1500;
        const float PeakHeight = 2f;

        float seconds = LifetimeMs / 1000f;

        Emit(new Particle
        {
            X = startX,
            Y = startY,
            PathX = startX,
            PathY = startY,
            Color = color,
            Motion = ParticleMotion.Path,
            LifetimeMs = LifetimeMs,
            TimeLeftMs = LifetimeMs,
            Size = 0f,
            Dx = (endX - startX) / seconds,
            Dy = (endY - startY) / seconds,
            ArcHeight = PeakHeight,
            TrailColor = color,
            TrailLifetimeMs = 400,
        });
    }

    /// <summary>
    /// Motes racing outward from a centre, or inward towards it, each leaving a trail.
    /// </summary>
    /// <remarks>
    /// Nova and Ring differ only in where the mote starts and how large it is drawn: a nova's motes
    /// run the whole radius and are visible, a ring's cover the outermost tenth and are invisible,
    /// so only the trail marks the circle out.
    /// </remarks>
    private void Ring(float centreX, float centreY, float radius, int color, float particleSize, int lifetimeMs, bool fromCentre)
    {
        int count = fromCentre ? 4 + (int)(radius * 2f) : 12;
        if (count <= 0 || radius <= 0f)
            return;

        float innerRadius = fromCentre ? 0f : radius * 0.9f;

        for (int i = 0; i < count; i++)
        {
            float angle = 2f * MathF.PI * i / count;
            float cos = MathF.Cos(angle);
            float sin = MathF.Sin(angle);

            EmitTrailingPath(
                centreX + innerRadius * cos,
                centreY + innerRadius * sin,
                centreX + radius * cos,
                centreY + radius * sin,
                color,
                particleSize,
                lifetimeMs);
        }
    }

    /// <summary>A cloud of sparks scattering from a point.</summary>
    private void Poison(float x, float y, int color)
    {
        const int Count = 10;

        for (int i = 0; i < Count; i++)
        {
            Emit(new Particle
            {
                X = x,
                Y = y,
                Z = 0.75f,
                Color = color,
                Motion = ParticleMotion.Drift,
                LifetimeMs = 400,
                TimeLeftMs = 400,
                Size = 100f,
                InitialSize = 100f,
                Shrinks = true,
                Dx = PlusMinus(4f),
                Dy = PlusMinus(4f),
                TrailColor = -1,
            });
        }
    }

    /// <summary>A dotted line drawn between two points.</summary>
    private void Line(float startX, float startY, float endX, float endY, int color)
    {
        const int Count = 30;

        for (int i = 0; i < Count; i++)
        {
            float t = i / (float)Count;

            Emit(new Particle
            {
                X = startX + (endX - startX) * t,
                Y = startY + (endY - startY) * t,
                Z = 0.5f,
                Color = color,
                Motion = ParticleMotion.Drift,
                LifetimeMs = 700,
                TimeLeftMs = 700,
                Size = 100f,
                InitialSize = 100f,
                Shrinks = true,
                Dx = PlusMinus(1f),
                Dy = PlusMinus(1f),
                TrailColor = -1,
            });
        }
    }

    /// <summary>An expanding shell, with the radius given as a point on its edge.</summary>
    private void Burst(float centreX, float centreY, float edgeX, float edgeY, int color)
    {
        const int Count = 24;

        float radius = Distance(centreX, centreY, edgeX, edgeY);
        if (radius <= 0f)
            return;

        for (int i = 0; i < Count; i++)
        {
            float angle = 2f * MathF.PI * i / Count;

            EmitTrailingPath(
                centreX,
                centreY,
                centreX + radius * MathF.Cos(angle),
                centreY + radius * MathF.Sin(angle),
                color,
                particleSize: 100f,
                lifetimeMs: 100 + (int)(200f * Next()));
        }
    }

    /// <summary>A contracting shell: the same as a burst, run inwards.</summary>
    private void Collapse(float centreX, float centreY, float edgeX, float edgeY, int color)
    {
        const int Count = 24;

        float radius = Distance(centreX, centreY, edgeX, edgeY);
        if (radius <= 0f)
            return;

        for (int i = 0; i < Count; i++)
        {
            float angle = 2f * MathF.PI * i / Count;

            EmitTrailingPath(
                centreX + radius * MathF.Cos(angle),
                centreY + radius * MathF.Sin(angle),
                centreX,
                centreY,
                color,
                particleSize: 300f,
                lifetimeMs: 200);
        }
    }

    /// <summary>A sixty-degree fan of motes thrown towards a point.</summary>
    private void ConeBlast(float startX, float startY, float towardsX, float towardsY, float radius, int color)
    {
        const int Count = 7;
        const float Spread = MathF.PI / 3f;

        float bearing = MathF.Atan2(towardsY - startY, towardsX - startX);

        for (int i = 0; i < Count; i++)
        {
            float angle = bearing - Spread / 2f + i * Spread / Count;

            EmitTrailingPath(
                startX,
                startY,
                startX + radius * MathF.Cos(angle),
                startY + radius * MathF.Sin(angle),
                color,
                particleSize: 200f,
                lifetimeMs: 100);
        }
    }

    /// <summary>
    /// A jagged bolt between two points.
    /// </summary>
    /// <remarks>
    /// The scatter is largest in the middle and tapers to nothing at both ends, so the bolt stays
    /// anchored to what it is arcing between however wild the middle gets.
    /// </remarks>
    private void Lightning(float startX, float startY, float endX, float endY, int color, int particleSize)
    {
        float distance = Distance(startX, startY, endX, endY);
        int count = (int)(distance * 3f);
        if (count <= 0)
            return;

        if (particleSize <= 0)
            particleSize = 100;

        for (int i = 0; i < count; i++)
        {
            float t = i / (float)count;
            float taper = Math.Min(i, count - i) * (distance / 200f);
            int lifetime = (int)(1000f - t * 900f);

            Emit(new Particle
            {
                X = startX + (endX - startX) * t + PlusMinus(taper),
                Y = startY + (endY - startY) * t + PlusMinus(taper),
                Z = 0.5f,
                Color = color,
                Motion = ParticleMotion.Drift,
                LifetimeMs = lifetime,
                TimeLeftMs = lifetime,
                Size = particleSize,
                InitialSize = particleSize,
                Shrinks = true,
                TrailColor = -1,
            });
        }
    }

    /// <summary>Motes drawn from a point into an entity, accelerating as they close.</summary>
    private void Flow(float startX, float startY, Entity target, int color)
    {
        const int Count = 5;

        for (int i = 0; i < Count; i++)
        {
            Emit(new Particle
            {
                X = startX,
                Y = startY,
                Z = 0.5f,
                Color = color,
                Motion = ParticleMotion.Flow,
                LifetimeMs = int.MaxValue,
                TimeLeftMs = int.MaxValue,
                Size = SmallSpray(),
                AnchorId = target.ObjectId,
                Distance = Distance(startX, startY, target.X, target.Y),
                Speed = Next() * 5f,
                TrailColor = -1,
            });
        }
    }

    /// <summary>One arc of a shocker's crackle, thrown out from the entity at a random bearing.</summary>
    private void EmitShockArc(Entity anchor, int color)
    {
        const float InnerRadius = 0.7f;
        const float OuterRadius = 2f;

        float angle = 2f * MathF.PI * Next();
        float cos = MathF.Cos(angle);
        float sin = MathF.Sin(angle);

        EmitTrailingPath(
            anchor.X + InnerRadius * sin,
            anchor.Y + InnerRadius * cos,
            anchor.X + OuterRadius * sin,
            anchor.Y + OuterRadius * cos,
            color == 0 ? 0x99CCFF : color,
            particleSize: 120f,
            lifetimeMs: 150);
    }

    /// <summary>
    /// One mote of a charging enemy's aura.
    /// </summary>
    /// <remarks>
    /// The original drew this as a field of particles sampled over the enemy's own sprite, then
    /// strobed the sprite itself once the charge completed. Sampling the sprite would mean reading
    /// the texture back on the CPU, so the aura is emitted around the entity instead; the strobe
    /// arrives separately as a Flash.
    /// </remarks>
    private void EmitFurySpark(Entity anchor, int color)
    {
        float angle = 2f * MathF.PI * Next();
        float distance = 0.2f + Next() * 0.5f;

        Emit(new Particle
        {
            X = anchor.X + distance * MathF.Cos(angle),
            Y = anchor.Y + distance * MathF.Sin(angle),
            Color = color == 0 ? 0xFF3300 : color,
            Motion = ParticleMotion.Drift,
            LifetimeMs = 500,
            TimeLeftMs = 500,
            Size = 80f,
            InitialSize = 80f,
            Shrinks = true,
            Dz = 1.5f,
            TrailColor = -1,
        });
    }

    /// <summary>How many motes each of a level-up's two swirls sheds.</summary>
    private const int LevelUpMotes = 20;

    /// <summary>How long a level-up's swirls take to complete, in milliseconds.</summary>
    private const int LevelUpDurationMs = 2000;

    /// <summary>
    /// The two counter-rotating green swirls that climb a character who has gained a level.
    /// </summary>
    /// <remarks>
    /// The original held forty particles alive for the whole two seconds and drove each one's height
    /// from a shared clock, so a particle was only actually shown while its height lay between zero
    /// and one and was invisible on either side of that window. The visible half is all that is
    /// emitted here: a mote is born at the moment its height would have reached zero and dies as it
    /// reaches one, which puts the same motes in the same places without the thirty idle ones.
    /// </remarks>
    public void LevelUp(Entity target, int color = 0x00FF00)
    {
        AddEmitter(EmitterKind.LevelUp, target.ObjectId, color,
            intervalMs: LevelUpDurationMs / 2 / LevelUpMotes, count: LevelUpMotes, nowMs: 0);
    }

    /// <summary>
    /// One mote from each of the two swirls, half a turn apart.
    /// </summary>
    /// <param name="index">
    /// Which mote of the swirl this is, counting down. It sets the bearing the mote starts at, which
    /// is what spreads the swirl into a helix rather than firing every mote from the same side.
    /// </param>
    private void EmitLevelUpPair(Entity anchor, int color, int index)
    {
        const float Radius = 0.5f;
        const int MoteLifetimeMs = LevelUpDurationMs / 2;

        // A full turn over the effect's whole duration, which is half a turn in a mote's own life.
        float spin = MathF.Tau / (LevelUpDurationMs / 1000f);
        float bearing = MathF.PI + MathF.PI * index / LevelUpMotes;

        EmitLevelUpMote(anchor, color, bearing, Radius, spin, MoteLifetimeMs);
        EmitLevelUpMote(anchor, color, bearing + MathF.PI, Radius, spin, MoteLifetimeMs);
    }

    private void EmitLevelUpMote(Entity anchor, int color, float angle, float radius, float spin, int lifetimeMs)
    {
        Emit(new Particle
        {
            X = anchor.X + radius * MathF.Cos(angle),
            Y = anchor.Y + radius * MathF.Sin(angle),
            Z = 0f,
            Color = color,
            Motion = ParticleMotion.Orbit,
            LifetimeMs = lifetimeMs,
            TimeLeftMs = lifetimeMs,
            Size = 100f,
            AnchorId = anchor.ObjectId,
            Angle = angle,
            Distance = radius,
            Spin = spin,

            // One tile of climb over the mote's life, which is the height the original ran to before
            // it stopped drawing the particle.
            Dz = 1000f / lifetimeMs,
            TrailColor = -1,
        });
    }

    /// <summary>
    /// Starts the camera shaking, from nothing.
    /// </summary>
    /// <remarks>
    /// <c>Camera.startJitter</c> (<c>Camera.as:107-110</c>) sets a flag and zeroes the amplitude,
    /// and nothing anywhere clears the flag again: the only thing that stops an earthquake in the
    /// original is arriving somewhere else, which builds a fresh camera. <see cref="Clear"/> is that
    /// here, and the world controller calls it on every map change.
    /// </remarks>
    public void StartJitter()
    {
        _jittering = true;
        Jitter = 0f;
    }

    // ----------------------------------------------------------------------------------------
    // Damage
    // ----------------------------------------------------------------------------------------

    /// <summary>
    /// The spray of the target's own colours thrown off by a hit.
    /// </summary>
    /// <param name="palette">Colours sampled from the target's sprite; empty means draw nothing.</param>
    /// <param name="angle">The incoming projectile's bearing, so the spray goes the other way.</param>
    /// <param name="speed">The projectile's speed, which sets how far the spray is thrown.</param>
    public void Hit(float x, float y, IReadOnlyList<int> palette, int size, int count, float angle, float speed)
    {
        if (palette == null || palette.Count == 0)
            return;

        count = Detail(count);

        float dx = speed / 600f * MathF.Cos(angle + MathF.PI);
        float dy = speed / 600f * MathF.Sin(angle + MathF.PI);

        for (int i = 0; i < count; i++)
            EmitDebris(x, y, palette, size, dx + (Next() - 0.5f) * 0.4f, dy + (Next() - 0.5f) * 0.4f);
    }

    /// <summary>The same spray, thrown in every direction. Used for deaths and for unattributed damage.</summary>
    public void Explode(float x, float y, IReadOnlyList<int> palette, int size, int count)
    {
        if (palette == null || palette.Count == 0)
            return;

        count = Detail(count);

        for (int i = 0; i < count; i++)
            EmitDebris(x, y, palette, size, Next() - 0.5f, Next() - 0.5f);
    }

    /// <summary>
    /// Thins a burst down to what the quality setting asks for.
    /// </summary>
    /// <remarks>
    /// Fewer pieces rather than shorter-lived ones: a spray that vanishes early reads as a bug,
    /// where a thinner spray just reads as a smaller one. At least one always comes out, so a hit
    /// is never completely silent to look at.
    /// </remarks>
    private int Detail(int count) => Quality switch
    {
        0 => Math.Max(1, count / 4),
        1 => Math.Max(1, count / 2),
        _ => count,
    };

    /// <summary>
    /// How much of a burst to draw: 0 low, 1 medium, 2 high.
    /// </summary>
    /// <remarks>
    /// Pushed in rather than read from the settings, because this class is compiled into the
    /// engine-free tests and must not know that a settings file exists.
    /// </remarks>
    public int Quality { get; set; } = 2;

    private void EmitDebris(float x, float y, IReadOnlyList<int> palette, int size, float dx, float dy)
    {
        int lifetime = 200 + (int)(100f * Next());

        Emit(new Particle
        {
            X = x,
            Y = y,
            Z = 0.5f,
            Color = palette[_random.Next(palette.Count)],
            Motion = ParticleMotion.Drift,
            LifetimeMs = lifetime,
            TimeLeftMs = lifetime,
            Size = size,
            Dx = dx * FastUnitsToTilesPerSecond,
            Dy = dy * FastUnitsToTilesPerSecond,
            TrailColor = -1,
        });
    }

    // ----------------------------------------------------------------------------------------
    // Helpers
    // ----------------------------------------------------------------------------------------

    /// <summary>A mote travelling a line and shedding sparks the whole way.</summary>
    private void EmitTrailingPath(float startX, float startY, float endX, float endY, int color, float particleSize, int lifetimeMs)
    {
        if (lifetimeMs <= 0)
            return;

        float seconds = lifetimeMs / 1000f;

        Emit(new Particle
        {
            X = startX,
            Y = startY,
            PathX = startX,
            PathY = startY,
            Color = color,
            Motion = ParticleMotion.Path,
            LifetimeMs = lifetimeMs,
            TimeLeftMs = lifetimeMs,
            Size = particleSize,
            Dx = (endX - startX) / seconds,
            Dy = (endY - startY) / seconds,
            TrailColor = color,
            TrailLifetimeMs = 600,
        });
    }

    private void Emit(in Particle particle)
    {
        if (_particles.Count >= MaxParticles)
            return;

        _particles.Add(particle);
    }

    private void AddEmitter(EmitterKind kind, int anchorId, int color, int intervalMs, int count, int nowMs)
    {
        // One emitter per entity per kind; a second Shocker on an already-shocked enemy replaces the
        // first rather than doubling the crackle.
        for (int i = 0; i < _emitters.Count; i++)
        {
            if (_emitters[i].Kind != kind || _emitters[i].AnchorId != anchorId)
                continue;

            _emitters.RemoveAt(i);
            break;
        }

        _emitters.Add(new Emitter
        {
            Kind = kind,
            AnchorId = anchorId,
            Color = color,
            IntervalMs = intervalMs,
            NextMs = nowMs,
            Remaining = count,
        });
    }

    /// <summary>The original's spray size: three to seven steps of twenty.</summary>
    private float SmallSpray() => (3 + (int)(Next() * 5f)) * 20f;

    /// <summary>A number in -<paramref name="magnitude"/>..<paramref name="magnitude"/>.</summary>
    private float PlusMinus(float magnitude) => Next() * magnitude * 2f - magnitude;

    private float Next() => (float)_random.NextDouble();

    private static float Distance(float x1, float y1, float x2, float y2)
    {
        float dx = x2 - x1;
        float dy = y2 - y1;
        return MathF.Sqrt(dx * dx + dy * dy);
    }
}

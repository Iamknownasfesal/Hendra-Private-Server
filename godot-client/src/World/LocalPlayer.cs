using System;

namespace Hendra.World;

/// <summary>
/// Our own player: the one entity whose position the client decides rather than receives.
/// </summary>
/// <remarks>
/// The server accepts whatever position we report — its speed check is commented out — so movement
/// here is authoritative in practice. What it does check is that the tile we land on is not
/// occupied, and it disconnects if it is. The speed formulas are still reproduced faithfully
/// because they have to agree with the server's for shot timing, which *is* enforced.
/// </remarks>
public sealed class LocalPlayer : Entity
{
    // Tiles per millisecond.
    public const float MinMoveSpeed = 0.004f;
    public const float MaxMoveSpeed = 0.0096f;

    // Shots per millisecond.
    public const float MinAttackFrequency = 0.0015f;
    public const float MaxAttackFrequency = 0.008f;

    public const float MinAttackMultiplier = 0.5f;
    public const float MaxAttackMultiplier = 2.0f;

    /// <summary>Stat value at which a curve reaches its maximum. Stats above it keep scaling.</summary>
    private const float StatDivisor = 75f;

    /// <summary>The boost applied by Speedy, Berserk and Damaging alike.</summary>
    private const float BoostMultiplier = 1.5f;

    public const int MaxSinkLevel = 18;

    /// <summary>Radians per millisecond of camera rotation while a turn key is held.</summary>
    public const float RotateSpeed = 0.003f;

    /// <summary>Below this speed a sliding player is treated as stopped.</summary>
    private const float SlideRestThreshold = 0.00012f;

    public int Speed;
    public int Dexterity;
    public int Attack;
    public int Vitality;
    public int Wisdom;
    public int Mp;
    public int MaxMp;
    public int Breath = -1;

    /// <summary>Gold, and the fame the character has banked. Both are spent at vendors.</summary>
    public int Credits;

    public int Fame;

    /// <summary>Terrain speed factor, updated whenever the player changes tile.</summary>
    public float MoveMultiplier = 1f;

    /// <summary>Movement input in camera-relative space, each component in -1..1.</summary>
    public float InputX;

    public float InputY;

    /// <summary>Camera rotation input: -1, 0 or 1.</summary>
    public float InputRotate;

    /// <summary>Current velocity in tiles per millisecond. Persists between frames so ice can carry it.</summary>
    public float VelocityX;

    public float VelocityY;

    /// <summary>Tiles per millisecond, after conditions and terrain.</summary>
    public float GetMoveSpeed()
    {
        if (IsSlowed)
            return MinMoveSpeed * MoveMultiplier;

        float speed = MinMoveSpeed + Speed / StatDivisor * (MaxMoveSpeed - MinMoveSpeed);
        if (IsSpeedy)
            speed *= BoostMultiplier;

        return speed * MoveMultiplier;
    }

    /// <summary>Shots per millisecond, before the weapon's own rate of fire.</summary>
    public float GetAttackFrequency()
    {
        if (IsDazed)
            return MinAttackFrequency;

        float frequency = MinAttackFrequency + Dexterity / StatDivisor * (MaxAttackFrequency - MinAttackFrequency);
        if (IsBerserk)
            frequency *= BoostMultiplier;

        return frequency;
    }

    /// <summary>Damage scaling applied to our own weapon shots.</summary>
    public float GetAttackMultiplier()
    {
        if (IsWeak)
            return MinAttackMultiplier;

        float multiplier = MinAttackMultiplier + Attack / StatDivisor * (MaxAttackMultiplier - MinAttackMultiplier);
        if (IsDamaging)
            multiplier *= BoostMultiplier;

        return multiplier;
    }

    /// <summary>
    /// Milliseconds between shots for a weapon with the given rate of fire.
    /// </summary>
    /// <remarks>
    /// Must match the server's <c>1 / GetAttackFrequency() * 1 / item.RateOfFire</c>: it rejects
    /// shots that arrive early, and compensates by burning a draw of the shared random stream, so
    /// firing too fast desynchronises damage prediction as well as losing the shot.
    /// </remarks>
    public float GetAttackPeriodMs(float rateOfFire) =>
        1f / GetAttackFrequency() * (1f / (rateOfFire <= 0f ? 1f : rateOfFire));

    /// <summary>
    /// Applies input and terrain to produce this frame's movement.
    /// </summary>
    /// <param name="map">Used for collision.</param>
    /// <param name="cameraAngle">
    /// Input is camera-relative — pressing "up" moves away from the viewer, not north — so the
    /// camera's rotation is added to the input angle.
    /// </param>
    /// <param name="deltaMs">Frame time.</param>
    public void UpdateMovement(GameMap map, float cameraAngle, int deltaMs)
    {
        if (IsPaused)
            return;

        var square = Square;
        float slide = square?.Desc?.SlideAmount ?? 0f;

        if (InputX != 0f || InputY != 0f)
        {
            float speed = GetMoveSpeed();
            float angle = cameraAngle + MathF.Atan2(InputY, InputX);
            float targetX = speed * MathF.Cos(angle);
            float targetY = speed * MathF.Sin(angle);

            if (slide > 0f)
            {
                // On ice, existing momentum decays and steering only tops it back up — so input
                // changes heading slowly rather than immediately.
                VelocityX *= slide;
                VelocityY *= slide;

                float currentSpeed = MathF.Sqrt(VelocityX * VelocityX + VelocityY * VelocityY);
                float inputSpeed = MathF.Sqrt(targetX * targetX + targetY * targetY);

                if (currentSpeed < inputSpeed)
                {
                    VelocityX += targetX * (1f - slide);
                    VelocityY += targetY * (1f - slide);
                }
            }
            else
            {
                VelocityX = targetX;
                VelocityY = targetY;
            }
        }
        else
        {
            float currentSpeed = MathF.Sqrt(VelocityX * VelocityX + VelocityY * VelocityY);
            if (currentSpeed > SlideRestThreshold && slide > 0f)
            {
                VelocityX *= slide;
                VelocityY *= slide;
            }
            else
            {
                VelocityX = 0f;
                VelocityY = 0f;
            }
        }

        // Conveyor tiles push independently of input, so they can carry a standing player.
        if (square?.Desc is { Push: true })
        {
            VelocityX -= square.Desc.AnimationDx / 1000f;
            VelocityY -= square.Desc.AnimationDy / 1000f;
        }

        if (IsParalyzed || IsPetrified)
            return;

        float destinationX = X + deltaMs * VelocityX;
        float destinationY = Y + deltaMs * VelocityY;

        var result = Movement.Resolve(map, X, Y, destinationX, destinationY);

        // Bouncing off a wall while sliding: reverse and halve, so ice feels slippery rather than
        // sticky when you hit something.
        if (slide > 0f)
        {
            if (result.BlockedX)
            {
                VelocityX *= -0.5f;
                VelocityY *= 0.5f;
            }

            if (result.BlockedY)
            {
                VelocityY *= -0.5f;
                VelocityX *= 0.5f;
            }
        }

        if (result.X != X || result.Y != Y)
        {
            // Facing follows movement, which is what the animation samples.
            Facing = MathF.Atan2(result.Y - Y, result.X - X);
            MoveVecX = (result.X - X) / MathF.Max(deltaMs, 1);
            MoveVecY = (result.Y - Y) / MathF.Max(deltaMs, 1);
        }
        else
        {
            MoveVecX = 0f;
            MoveVecY = 0f;
        }

        map.MoveEntity(this, result.X, result.Y);
        OnMoved();
    }

    /// <summary>
    /// Recomputes terrain-derived movement state after a move.
    /// </summary>
    /// <remarks>
    /// Sinking is cumulative rather than a fixed penalty: each step deeper slows the player further,
    /// approaching a floor of a tenth of normal speed, which is what makes deep water feel like a
    /// trap rather than merely slow going.
    /// </remarks>
    public void OnMoved()
    {
        var desc = Square?.Desc;
        if (desc == null)
        {
            MoveMultiplier = 1f;
            return;
        }

        if (desc.Sinking)
        {
            SinkLevel = Math.Min(SinkLevel + 1, MaxSinkLevel);
            MoveMultiplier = 0.1f + (1f - (float)SinkLevel / MaxSinkLevel) * (desc.Speed - 0.1f);
        }
        else
        {
            SinkLevel = 0;
            MoveMultiplier = desc.Speed;
        }
    }

    /// <summary>
    /// Applies movement input, accounting for confusion.
    /// </summary>
    /// <remarks>
    /// Confusion swaps and negates the axes, so the controls become consistently wrong rather than
    /// randomly wrong — you can still navigate, badly, once you work out the mapping.
    /// </remarks>
    public void SetInput(float x, float y, float rotate)
    {
        if (IsConfused)
        {
            InputX = -y;
            InputY = -x;
            InputRotate = -rotate;
        }
        else
        {
            InputX = x;
            InputY = y;
            InputRotate = rotate;
        }
    }

    /// <summary>Our own position is never overwritten by a tick; only Goto repositions us.</summary>
    public override void OnTickPosition(float x, float y, int tickDurationMs)
    {
    }

    public override void Update(int nowMs, int deltaMs)
    {
        // Movement is driven from UpdateMovement, which needs the camera angle, so the base class's
        // interpolation towards a server position is deliberately not run here.
    }
}

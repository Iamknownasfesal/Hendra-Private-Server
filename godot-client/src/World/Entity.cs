using System;
using Hendra.Data;
using Hendra.Resources;

namespace Hendra.World;

/// <summary>
/// Anything the server tracks by object id: monsters, players, portals, containers, decorations.
/// </summary>
/// <remarks>
/// The original had a deep inheritance chain — BasicObject, GameObject, Character, Player — with
/// behaviour spread across all four levels and a great deal of it switched on at runtime by testing
/// flags. Here the flags live in the shared <see cref="Resources.ObjectDesc"/> and only genuinely
/// different behaviour gets a subclass.
/// </remarks>
public class Entity
{
    /// <summary>How long a non-player attack pose is held, in milliseconds.</summary>
    public const int AttackPeriodMs = 300;

    /// <summary>
    /// Client-side objects — projectiles, particles, effects — take ids from this range so they can
    /// never collide with a server-assigned one.
    /// </summary>
    private const int FakeObjectIdBase = 0x7F000000;

    private static int _nextFakeObjectId;

    public int ObjectId;
    public ushort ObjectType;
    public ObjectDesc Desc;

    /// <summary>Position in tiles.</summary>
    public float X;

    public float Y;

    /// <summary>Height above the ground, in tiles.</summary>
    public float Z;

    public string Name;
    public ConditionEffects Conditions;

    public int Hp = 200;
    public int MaxHp = 200;
    public int Size = 100;
    public int Level = -1;
    public int Defense;
    public int Texture1;
    public int Texture2;
    public int AltTextureIndex;
    public int SinkLevel;
    public bool Dead;

    /// <summary>Clock reading at which this thing appeared in the world, or zero if it was here.</summary>
    /// <remarks>
    /// Drives the arrival: a thing that walked into view, stepped out of a portal or was spawned
    /// settles down out of the air rather than being there abruptly. A reading rather than a
    /// countdown, so nothing has to tick it.
    /// </remarks>
    public int ArrivedAtMs;

    /// <summary>Equipment, inventory and backpack in one array: 0-7 worn, 8-15 carried, 16-23 backpack.</summary>
    public int[] Equipment;

    // ---- Vendor state. Only meaningful for merchants and other sellable objects. ----

    /// <summary>What this vendor sells, as an object type. -1 when it sells nothing.</summary>
    public int MerchandiseType = -1;

    public int MerchandisePrice;

    /// <summary>0 gold, 1 fame, and so on. Matches the server's CurrencyType.</summary>
    public int MerchandiseCurrency;

    /// <summary>Stock remaining. -1 means unlimited.</summary>
    public int MerchandiseCount = -1;

    /// <summary>Account rank needed to buy, if any.</summary>
    public int MerchandiseRankRequired;

    /// <summary>Heading in radians, used to pick the animation facing.</summary>
    public float Facing;

    /// <summary>The guild this entity belongs to, or empty. Arrives as a stat.</summary>
    public string Guild = string.Empty;

    public float AttackAngle;
    public int AttackStartMs = int.MinValue;

    /// <summary>The tile this entity currently stands on. Null while off-map.</summary>
    public Square Square;

    // ---- Flash. A colour pulsed over the sprite for a fixed number of cycles. ----

    private int _flashStartMs;
    private int _flashColor;
    private int _flashPeriodMs;
    private int _flashRepeats;

    /// <summary>
    /// Starts a colour pulse over this entity's sprite.
    /// </summary>
    /// <param name="periodMs">One full pulse, in milliseconds.</param>
    /// <param name="repeats">How many pulses before it stops.</param>
    public void StartFlash(int nowMs, int color, int periodMs, int repeats)
    {
        if (periodMs <= 0 || repeats <= 0)
            return;

        _flashStartMs = nowMs;
        _flashColor = color;
        _flashPeriodMs = periodMs;
        _flashRepeats = repeats;
    }

    /// <summary>
    /// How strongly the flash colour covers the sprite right now, from zero to a half.
    /// </summary>
    /// <remarks>
    /// The original applied this as a colour transform — <c>rgb·(1-s) + target·s</c> — which meant
    /// cloning the finished bitmap and transforming it on every frame of the flash. Drawing the
    /// same sprite a second time at alpha <c>s</c> in the flash colour produces exactly that
    /// expression through ordinary alpha blending, and costs one more quad.
    /// </remarks>
    public float FlashStrength(int nowMs, out int color)
    {
        color = _flashColor;

        if (_flashPeriodMs <= 0)
            return 0f;

        if (nowMs > _flashStartMs + _flashPeriodMs * _flashRepeats)
        {
            _flashPeriodMs = 0;
            return 0f;
        }

        int phase = (nowMs - _flashStartMs) % _flashPeriodMs;
        return MathF.Sin(phase / (float)_flashPeriodMs * MathF.PI) * 0.5f;
    }

    // Where the server last said this entity is, and how fast it appeared to be going. The
    // velocity is only used to drive the walk animation; motion itself comes from interpolation.
    protected float TickX;
    protected float TickY;
    public float MoveVecX;
    public float MoveVecY;

    public bool IsMoving => MoveVecX != 0f || MoveVecY != 0f;

    public static int NextFakeObjectId() => FakeObjectIdBase | _nextFakeObjectId++;

    public bool Has(ConditionEffects flag) => (Conditions & flag) != 0;

    public bool IsPaused => Has(ConditionEffects.Paused);
    public bool IsInvisible => Has(ConditionEffects.Invisible);
    public bool IsHidden => Has(ConditionEffects.Hidden);
    public bool IsParalyzed => Has(ConditionEffects.Paralyzed);
    public bool IsPetrified => Has(ConditionEffects.Petrify);
    public bool IsStasis => Has(ConditionEffects.Stasis);
    public bool IsInvincible => Has(ConditionEffects.Invincible);
    public bool IsInvulnerable => Has(ConditionEffects.Invulnerable);
    public bool IsArmorBroken => Has(ConditionEffects.ArmorBroken);
    public bool IsSlowed => Has(ConditionEffects.Slowed);
    public bool IsDazed => Has(ConditionEffects.Dazed);
    public bool IsSpeedy => Has(ConditionEffects.Speedy) || Has(ConditionEffects.NinjaSpeedy);
    public bool IsBerserk => Has(ConditionEffects.Berserk);
    public bool IsWeak => Has(ConditionEffects.Weak);
    public bool IsDamaging => Has(ConditionEffects.Damaging);
    public bool IsConfused => Has(ConditionEffects.Confused);

    /// <summary>Whether a projectile can currently hit this entity.</summary>
    public bool CanBeHit => !Dead && (Conditions & ConditionEffects.ProjectileNoHitMask) == 0;

    /// <summary>Half-width of the hit box, in tiles. The original's radius_, fixed at half a tile.</summary>
    public virtual float HitRadius => 0.5f;

    public void Place(float x, float y)
    {
        X = x;
        Y = y;
        TickX = x;
        TickY = y;
    }

    /// <summary>
    /// Applies a position from a NewTick.
    /// </summary>
    /// <remarks>
    /// Only ever called for other entities. Our own player is never repositioned by a tick — the
    /// client owns its position between Move packets, and the server corrects it with Goto instead.
    /// </remarks>
    public virtual void OnTickPosition(float x, float y, int tickDurationMs)
    {
        TickX = x;
        TickY = y;

        if (tickDurationMs > 0)
        {
            MoveVecX = (x - X) / tickDurationMs;
            MoveVecY = (y - Y) / tickDurationMs;
        }
    }

    /// <summary>Applies an authoritative correction, snapping rather than interpolating.</summary>
    public virtual void OnGoto(float x, float y)
    {
        X = x;
        Y = y;
        TickX = x;
        TickY = y;
        MoveVecX = 0f;
        MoveVecY = 0f;
    }

    /// <summary>
    /// Advances the entity towards where the server last said it was.
    /// </summary>
    /// <remarks>
    /// This is an exponential approach, not a linear tween between ticks: each frame closes a fixed
    /// fraction of the remaining gap. The constant is the original's, and its odd value
    /// (approximately one part in 484 per millisecond) is worth keeping — it is what gives remote
    /// players their characteristic slightly-lagging glide, and a linear interpolation looks
    /// noticeably different.
    /// </remarks>
    public virtual void Update(int nowMs, int deltaMs)
    {
        float dx = TickX - X;
        float dy = TickY - Y;

        if (dx * dx + dy * dy > 0.01f)
        {
            float fraction = deltaMs * 0.0020666f;
            X = fraction * TickX + (1f - fraction) * X;
            Y = fraction * TickY + (1f - fraction) * Y;
        }
        else
        {
            MoveVecX = 0f;
            MoveVecY = 0f;
        }

        // A few objects sit at a different height, or start flying, only while in motion.
        if (Desc?.WhileMoving != null)
        {
            bool moving = IsMoving;
            Z = moving ? Desc.WhileMoving.Z : Desc.Z;
        }
    }

    /// <summary>Records an attack for the animation, from either a local shot or an AllyShoot.</summary>
    public void SetAttack(float angle, int nowMs)
    {
        AttackAngle = angle;
        AttackStartMs = nowMs;
    }

    /// <summary>Whether the attack pose is still showing.</summary>
    public bool IsAttacking(int nowMs, int periodMs = AttackPeriodMs) =>
        nowMs < AttackStartMs + periodMs;

    public float DistanceTo(float x, float y)
    {
        float dx = x - X;
        float dy = y - Y;
        return MathF.Sqrt(dx * dx + dy * dy);
    }

    /// <summary>
    /// The damage an attack of <paramref name="damage"/> actually does after defence.
    /// </summary>
    /// <remarks>
    /// Mirrors the server's <c>GetDefenseDamage</c>, which is the authoritative version. Note that
    /// the AS3 client floors damage at 15% (<c>damage * 3 / 20</c>) while the server floors it at
    /// 25% — a genuine pre-existing disagreement between the two. The server wins, so this is only
    /// ever a prediction used to draw a number; the real figure arrives in a Damage packet.
    /// </remarks>
    public static int ApplyDefense(int damage, int defense, bool armorPiercing, ConditionEffects conditions)
    {
        int effectiveDefense = defense;

        if (armorPiercing || (conditions & ConditionEffects.ArmorBroken) != 0)
            effectiveDefense = 0;
        else if ((conditions & ConditionEffects.Armored) != 0)
            effectiveDefense *= 2;

        int floor = (int)(damage * 0.25f);
        int result = Math.Max(floor, damage - effectiveDefense);

        if ((conditions & ConditionEffects.Invulnerable) != 0 ||
            (conditions & ConditionEffects.Invincible) != 0)
            return 0;

        if ((conditions & ConditionEffects.Petrify) != 0)
            result = (int)(result * 0.9f);

        if ((conditions & ConditionEffects.Curse) != 0)
            result = (int)(result * 1.2f);

        return result;
    }
}

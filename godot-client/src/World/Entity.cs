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
    /// <summary>
    /// How long a character with no rate of fire of its own holds the attack pose, in milliseconds.
    /// </summary>
    /// <remarks>
    /// <c>GameObject.ATTACK_PERIOD</c>, which is what every monster and every non-player animates
    /// its attack over. A player overrides it with the gap between its own shots — see
    /// <see cref="AttackPeriodMs"/>.
    /// </remarks>
    public const int DefaultAttackPeriodMs = 300;

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

    /// <summary>
    /// The skin this player is wearing, as the skin object's own type. Zero for the class's own
    /// sprite.
    /// </summary>
    /// <remarks>
    /// A whole different animated sheet rather than a frame within the one the object type names,
    /// which is what <see cref="AltTextureIndex"/> picks. <c>ReskinHandler.as:21-32</c> resolves it
    /// the same way: the skin object's own texture replaces the class's.
    /// </remarks>
    public int Skin;
    public int SinkLevel;
    public bool Dead;

    /// <summary>
    /// The bullet artwork a completed equipment set substitutes, and the artwork it stands in for.
    /// Both empty when this player is wearing no set.
    /// </summary>
    /// <remarks>
    /// <para>
    /// A purely visual swap, and a narrow one: only a shot whose own object id matches
    /// <see cref="ProjectileOverrideOld"/> is redrawn, so switching to a different weapon while the
    /// set is on leaves that weapon's bullets alone. Damage, speed, lifetime and size all still come
    /// from the weapon — <c>Projectile.as:96-97</c> replaces <c>props_</c> only, never
    /// <c>projProps_</c>.
    /// </para>
    /// <para>
    /// The old id is captured once, when the skin arrives, off whatever is in the weapon slot at
    /// that moment (<c>GameServerConnectionConcrete.as:1665-1668</c>).
    /// </para>
    /// </remarks>
    public string ProjectileOverrideNew = string.Empty;

    public string ProjectileOverrideOld = string.Empty;

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

    /// <summary>
    /// Rank within that guild: 0 initiate, 10 member, 20 officer, 30 leader, 40 founder.
    /// </summary>
    /// <remarks>
    /// What the guild line over a head is coloured by. Arrives beside the name as its own stat, as
    /// the server exports it.
    /// </remarks>
    public int GuildRank;

    /// <summary>
    /// The account's star rating, which is drawn beside the name.
    /// </summary>
    /// <remarks>
    /// One star per fame threshold each of the account's classes has passed, summed across them, so
    /// it measures how widely somebody has played rather than how far one character got. The
    /// original composites it into the name plate rather than writing it out
    /// (<c>Player.as:750-756</c>).
    /// </remarks>
    public int Stars;

    /// <summary>Whether the account is an administrator, which gives the star its own colour.</summary>
    public bool Admin;

    /// <summary>
    /// The halo colour, as a packed <c>0xRRGGBB</c>. Zero is no halo, which is nearly everybody.
    /// </summary>
    /// <remarks>
    /// <c>StatsType.Glow</c>, which the original hands to <c>GameObject.setGlow</c> and paints as a
    /// ring around the sprite (<c>GlowRedrawer.as:19-46</c>). It is what <c>/glow</c> sets.
    /// </remarks>
    public int GlowColor;

    /// <summary>Whether this account picked its own name, which is what colours it.</summary>
    /// <remarks>
    /// <c>Player.getNameColor</c> answers the chosen-name colour for a player who has one and plain
    /// white for one still wearing a name off the generated list (<c>Player.as:757-764</c>).
    /// </remarks>
    public bool NameChosen;

    /// <summary>
    /// Which neighbours this piece of scenery joins onto, as <c>ConnectionInfo.Bits</c>, or zero.
    /// </summary>
    /// <remarks>
    /// Four bytes, one per side, each 1 or 2. The original turns them into one of six connector
    /// shapes and a rotation (<c>ConnectedObject.as:98</c>), which is what makes a run of cave wall
    /// read as a wall rather than as a row of posts.
    /// </remarks>
    public int ConnectType;

    /// <summary>Whether this portal will let anybody through. Meaningless for anything else.</summary>
    /// <remarks>
    /// <c>PortalPanel</c> hides its Enter button while this is off (<c>PortalPanel.as:92-97</c>).
    /// True by default, as the original's <c>Portal.active_</c> is (<c>Portal.as:20</c>).
    /// </remarks>
    public bool PortalActive = true;

    public float AttackAngle;
    public int AttackStartMs = int.MinValue;

    /// <summary>
    /// How long one cycle of this character's attack animation lasts, in milliseconds.
    /// </summary>
    /// <remarks>
    /// For a player this is the gap between its shots — <c>1 / attackFrequency / rateOfFire</c>,
    /// which <c>Player.setAttack</c> and <c>Player.shoot</c> keep in <c>attackPeriod_</c>. The
    /// attack animation is two frames, the second being the one where the weapon is extended, so
    /// tying the cycle to the rate of fire is what makes the extended frame appear once per shot.
    /// Animating over the fixed 300ms instead stretches the cycle across several shots of a fast
    /// weapon, and each shot restarts it from the first frame before the second is ever reached.
    /// </remarks>
    public int AttackPeriodMs = DefaultAttackPeriodMs;

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

    // What a hit cannot land on this body. Three of them can be a property of the kind of thing it
    // is as well as a condition it is carrying -- `GameObject.as:200-208` reads StunImmune,
    // ParalyzeImmune and DazedImmune off the object's own XML.
    public bool IsStunImmune => Has(ConditionEffects.StunImmune) || Desc is { StunImmune: true };
    public bool IsParalyzeImmune => Has(ConditionEffects.ParalyzeImmune) || Desc is { ParalyzeImmune: true };
    public bool IsDazedImmune => Has(ConditionEffects.DazedImmune) || Desc is { DazedImmune: true };
    public bool IsSlowedImmune => Has(ConditionEffects.SlowedImmune);
    public bool IsArmorBreakImmune => Has(ConditionEffects.ArmorBreakImmune);
    public bool IsPetrifyImmune => Has(ConditionEffects.PetrifyImmune);
    public bool IsCurseImmune => Has(ConditionEffects.CurseImmune);

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
    /// <param name="periodMs">
    /// How long the pose lasts. The shooter's own gap between shots where that is known, and
    /// <see cref="DefaultAttackPeriodMs"/> for everything that fires without a rate of fire.
    /// </param>
    public void SetAttack(float angle, int nowMs, int periodMs = DefaultAttackPeriodMs)
    {
        AttackAngle = angle;
        AttackStartMs = nowMs;
        AttackPeriodMs = Math.Max(1, periodMs);
    }

    /// <summary>Whether the attack pose is still showing.</summary>
    public bool IsAttacking(int nowMs) =>
        AttackStartMs != int.MinValue && nowMs < AttackStartMs + AttackPeriodMs;

    /// <summary>How far through the attack animation this character is, in [0, 1).</summary>
    public float AttackPhase(int nowMs) =>
        (nowMs - AttackStartMs) % AttackPeriodMs / (float)AttackPeriodMs;

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

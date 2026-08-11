namespace Hendra.Core;

/// <summary>
/// The synced pseudo-random generator shared with the server.
///
/// Park-Miller MINSTD (multiplier 16807, modulus 2^31-1) in the split-multiply form used by the
/// AS3 client's com/company/util/Random.as. The server seeds its own generator with the same value
/// and hands it to the client in MapInfo.Seed, so both sides walk the same sequence and the client
/// can predict damage rolls locally.
///
/// Two rules keep the streams aligned, and breaking either desyncs every subsequent roll:
///   - Reseed on every MapInfo (i.e. every map change).
///   - Draw in exactly the same order as the server. In the AS3 client there are only two call
///     sites: once per projectile in Player.doShoot (iterating NumProjectiles), and once per
///     damaging-tile tick for ground damage. The server compensates for a shot it rejects by
///     calling DropNextRandom(), which is a bare NextInt().
///
/// All arithmetic is unsigned 32-bit and deliberately wrapping.
/// </summary>
public sealed class MinstdRandom
{
    private uint _seed;

    public MinstdRandom(uint seed)
    {
        _seed = seed;
    }

    /// <summary>The current internal state. Exposed for parity tests against the server.</summary>
    public uint Seed => _seed;

    /// <summary>Advances the generator and returns the raw state.</summary>
    /// <remarks>
    /// The two right shifts are *arithmetic*, not logical. Both wServer/wRandom.cs and the AS3
    /// original shift the signed reinterpretation of the value, and the distinction is load
    /// bearing on the very first draw: the seed arrives straight off the wire as an arbitrary
    /// uint32 (the server derives it from a tick count times a hash), so its high bit is set about
    /// half the time. A logical shift there produces a different first value and every draw after
    /// it — see WRandomParityTests. Once running, the conditional subtract does keep the state
    /// below 2^31-1, so from the second draw on the two forms happen to agree; that is exactly what
    /// makes the bug easy to miss.
    /// </remarks>
    public uint NextInt()
    {
        unchecked
        {
            uint lo = 16807u * (_seed & 0xFFFFu);
            uint hi = 16807u * (uint)((int)_seed >> 16);
            lo += (hi & 32767u) << 16;
            lo += (uint)((int)hi >> 15);
            if (lo > 2147483647u)
                lo -= 2147483647u;
            return _seed = lo;
        }
    }

    /// <summary>
    /// A value in [min, max). Matches the AS3 nextIntRange, including the top-exclusive bound and
    /// the short-circuit when the range is empty (which does *not* advance the generator).
    /// </summary>
    public uint NextIntRange(uint min, uint max)
    {
        if (min == max)
            return min;
        return min + NextInt() % (max - min);
    }

    /// <summary>
    /// Burns one draw without using it. The mirror of the server's DropNextRandom, for the cases
    /// where the server is known to have stepped its generator without the client producing a roll.
    /// </summary>
    public void Drop() => NextInt();
}

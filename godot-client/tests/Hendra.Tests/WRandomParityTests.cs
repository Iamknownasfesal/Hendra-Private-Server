using Hendra.Core;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// The client predicts its own damage locally by stepping a generator seeded from MapInfo with the
/// same value the server uses. If the two sequences ever diverge, damage numbers drift out of
/// agreement and — because the server also uses the stream for ground damage — so does health.
///
/// These tests compare against <c>wServer.wRandom</c> compiled straight out of the server tree, so
/// they check agreement with the real implementation rather than with a second copy of my own
/// reading of it.
/// </summary>
public sealed class WRandomParityTests
{
    /// <summary>
    /// Seeds worth covering: zero, one, the modulus boundary, values with the high bit set (which
    /// is where an arithmetic-versus-logical shift mistake shows up), and the seed shape the server
    /// actually produces — <c>(uint)(TickCount * guid.GetHashCode()) % uint.MaxValue</c>, which is
    /// spread across the whole 32-bit range.
    /// </summary>
    public static TheoryData<uint> Seeds => new()
    {
        0u,
        1u,
        16807u,
        2147483646u,
        2147483647u,
        2147483648u,
        3000000000u,
        4294967294u,
        uint.MaxValue,
        1234567890u,
        0x80000001u,
        0xDEADBEEFu,
    };

    [Theory]
    [MemberData(nameof(Seeds))]
    public void NextInt_MatchesServer_OverLongSequence(uint seed)
    {
        var mine = new MinstdRandom(seed);
        var theirs = new wServer.wRandom(seed);

        for (int i = 0; i < 10_000; i++)
        {
            uint expected = theirs.NextInt();
            uint actual = mine.NextInt();
            Assert.True(expected == actual,
                $"Diverged at draw {i} from seed {seed}: server {expected}, client {actual}.");
        }
    }

    [Theory]
    [MemberData(nameof(Seeds))]
    public void NextIntRange_MatchesServer(uint seed)
    {
        var mine = new MinstdRandom(seed);
        var theirs = new wServer.wRandom(seed);

        // Ranges shaped like real weapon damage, plus the degenerate equal-bounds case, which must
        // short-circuit without advancing the generator on either side.
        (uint Min, uint Max)[] ranges =
        {
            (0u, 1u), (1u, 2u), (10u, 20u), (45u, 60u), (100u, 140u), (7u, 7u), (0u, 1000u),
        };

        for (int i = 0; i < 2_000; i++)
        {
            var (min, max) = ranges[i % ranges.Length];
            uint expected = theirs.NextIntRange(min, max);
            uint actual = mine.NextIntRange(min, max);
            Assert.True(expected == actual,
                $"Diverged at draw {i} from seed {seed} over [{min},{max}): server {expected}, client {actual}.");
        }
    }

    [Fact]
    public void EqualBounds_DoesNotAdvanceTheGenerator()
    {
        // Relied on implicitly: a weapon with min == max damage must not consume a draw, or every
        // subsequent roll would be one step ahead of the server's.
        var mine = new MinstdRandom(12345u);
        uint before = mine.Seed;
        Assert.Equal(7u, mine.NextIntRange(7u, 7u));
        Assert.Equal(before, mine.Seed);
    }

    [Fact]
    public void Drop_AdvancesExactlyOneStep()
    {
        // The server calls DropNextRandom() when it refuses a shot we already fired, to stay in
        // step with us. Our compensation has to cost exactly one draw.
        var dropped = new MinstdRandom(999u);
        var reference = new MinstdRandom(999u);

        dropped.Drop();
        reference.NextInt();

        Assert.Equal(reference.Seed, dropped.Seed);
    }

    /// <summary>
    /// The same generator but with logical shifts instead of arithmetic ones — the mistake this
    /// test exists to catch.
    /// </summary>
    private static uint LogicalShiftVariant(uint seed)
    {
        unchecked
        {
            uint lo = 16807u * (seed & 0xFFFFu);
            uint hi = 16807u * (seed >> 16);
            lo += (hi & 32767u) << 16;
            lo += hi >> 15;
            if (lo > 2147483647u)
                lo -= 2147483647u;
            return lo;
        }
    }

    [Theory]
    [InlineData(2147483648u)]
    [InlineData(3000000000u)]
    [InlineData(0x80000001u)]
    [InlineData(0xDEADBEEFu)]
    [InlineData(uint.MaxValue)]
    public void ArithmeticShiftIsRequired_ForSeedsWithTheHighBitSet(uint seed)
    {
        // MapInfo.Seed is an arbitrary uint32 — the server builds it from a tick count multiplied
        // by a hash — so roughly half of all sessions start with the high bit set. On those, a
        // logical shift produces a different first draw and never recovers.
        //
        // This is the whole reason NextInt casts through int. Once the generator is running the
        // conditional subtract keeps the state under 2^31-1, so the two forms agree from the second
        // draw onward and the bug hides everywhere except the first roll of each map.
        uint fromServer = new wServer.wRandom(seed).NextInt();

        Assert.Equal(fromServer, new MinstdRandom(seed).NextInt());
        Assert.NotEqual(fromServer, LogicalShiftVariant(seed));
    }

    [Theory]
    [InlineData(1u)]
    [InlineData(16807u)]
    [InlineData(1234567890u)]
    public void BothShiftForms_AgreeWhenTheSeedFitsInThirtyOneBits(uint seed)
    {
        // The counterpart to the test above: with the high bit clear there is nothing to sign
        // extend, so the shift choice is invisible. Recorded so the asymmetry is documented rather
        // than rediscovered.
        uint fromServer = new wServer.wRandom(seed).NextInt();

        Assert.Equal(fromServer, new MinstdRandom(seed).NextInt());
        Assert.Equal(fromServer, LogicalShiftVariant(seed));
    }
}

using System;

namespace wServer.realm
{
    /// <summary>
    /// A player's own account of where it has been, kept in the player's clock.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Every Move packet carries a short trail of timestamped positions: the client samples itself
    /// about ten times a second and sends the lot. This server has always parsed that trail and
    /// thrown it away. Kept, it is what lets the server ask "where did you say you were when that
    /// bullet passed through you" instead of the much weaker "where are you now" -- which, on a
    /// two-hundred millisecond tick and a hundred milliseconds of wire, is a tile or more out of
    /// date and would have the server killing people who dodged.
    /// </para>
    /// <para>
    /// The clock is the client's own throughout. Nothing here converts to server time; the caller
    /// has <c>Player.S2CTime</c> for that, and doing the conversion once at the edge keeps the
    /// drift in one place.
    /// </para>
    /// </remarks>
    public sealed class PositionTimeline
    {
        /// <summary>Samples held. At ten a second, a little over twelve seconds of trail.</summary>
        private const int Capacity = 128;

        private readonly int[] _time = new int[Capacity];
        private readonly float[] _x = new float[Capacity];
        private readonly float[] _y = new float[Capacity];

        private int _count;
        private int _next;

        private readonly object _lock = new object();

        public void Clear()
        {
            lock (_lock)
            {
                _count = 0;
                _next = 0;
            }
        }

        /// <summary>
        /// Adds a sample, ignoring anything not newer than what is already held.
        /// </summary>
        /// <remarks>
        /// The trail overlaps between packets -- the client re-sends samples it has already sent --
        /// and it must stay sorted for <see cref="At"/> to binary search it, so out-of-order and
        /// duplicate readings are dropped rather than appended.
        /// </remarks>
        public void Add(int clientTime, float x, float y)
        {
            lock (_lock)
            {
                if (_count > 0 && clientTime <= _time[Index(_count - 1)])
                    return;

                _time[_next] = clientTime;
                _x[_next] = x;
                _y[_next] = y;

                _next = (_next + 1) % Capacity;
                if (_count < Capacity)
                    _count++;
            }
        }

        /// <summary>
        /// Where the player said it was at that reading of its own clock, or null when the trail
        /// does not reach.
        /// </summary>
        /// <remarks>
        /// Nothing is extrapolated off either end. A reading past the newest sample would be the
        /// player's last known position wearing a timestamp it never had, and answering with it
        /// would put a dodging player where they used to be -- which is exactly the mistake this
        /// class exists to avoid. Callers treat not knowing as innocence and ask again later.
        /// </remarks>
        public Position? At(int clientTime)
        {
            lock (_lock)
            {
                if (_count == 0)
                    return null;

                var oldest = _time[Index(0)];
                var newest = _time[Index(_count - 1)];

                if (clientTime < oldest || clientTime > newest)
                    return null;

                if (clientTime == newest)
                    return Sample(_count - 1);

                // The largest sample at or before the reading. Everything between two samples is
                // walked in a straight line, which is what the client was doing between them.
                var lo = 0;
                var hi = _count - 1;
                while (lo < hi)
                {
                    var mid = (lo + hi + 1) / 2;
                    if (_time[Index(mid)] <= clientTime)
                        lo = mid;
                    else
                        hi = mid - 1;
                }

                var a = Index(lo);
                var b = Index(lo + 1);

                var span = _time[b] - _time[a];
                if (span <= 0)
                    return Sample(lo);

                var t = (float)(clientTime - _time[a]) / span;
                return new Position
                {
                    X = _x[a] + (_x[b] - _x[a]) * t,
                    Y = _y[a] + (_y[b] - _y[a]) * t
                };
            }
        }

        /// <summary>
        /// The most recent sample, for questions about a moment the trail has not reached yet.
        /// </summary>
        /// <remarks>
        /// A shot is sent between two Move packets, so the trail almost never covers the instant it
        /// was fired. The caller gets the last thing the player did say, and the reading it was said
        /// at, so it can allow for the travelling done since rather than guess at it.
        /// </remarks>
        public bool TryNewest(out int clientTime, out Position at)
        {
            lock (_lock)
            {
                if (_count == 0)
                {
                    clientTime = 0;
                    at = new Position();
                    return false;
                }

                clientTime = _time[Index(_count - 1)];
                at = Sample(_count - 1);
                return true;
            }
        }

        /// <summary>Ring slot of the i-th oldest sample held.</summary>
        private int Index(int i)
        {
            return (_next - _count + i + Capacity * 2) % Capacity;
        }

        private Position Sample(int i)
        {
            var at = Index(i);
            return new Position { X = _x[at], Y = _y[at] };
        }
    }
}

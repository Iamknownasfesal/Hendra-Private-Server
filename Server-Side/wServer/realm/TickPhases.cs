using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Text;
using log4net;

namespace wServer.realm
{
    /// <summary>
    /// Where the server's tick goes, measured rather than guessed at.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The logic loop reports that it lagged and by how much, which says a tick took too long but
    /// nothing at all about what it was doing. This times the named parts of it and writes a line
    /// every few seconds while it is behind, so the answer to "what is the server spending its
    /// time on" is a log line rather than an argument.
    /// </para>
    /// <para>
    /// Deliberately as cheap as the thing it is measuring can afford: a stopwatch read per phase
    /// per tick, and the summing done when the line is written rather than as it goes.
    /// </para>
    /// </remarks>
    public static class TickPhases
    {
        private static readonly ILog Log = LogManager.GetLogger("TickPhases");

        /// <summary>How often a summary is written, in milliseconds.</summary>
        private const int ReportEvery = 5000;

        private class Phase
        {
            public double TotalMs;
        }

        private static readonly Dictionary<string, Phase> Phases = new Dictionary<string, Phase>();
        private static readonly List<string> Order = new List<string>();
        private static readonly Stopwatch Watch = Stopwatch.StartNew();
        private static readonly object Gate = new object();

        private static long _reportedAt;

        /// <summary>Times a section for as long as the returned value is alive.</summary>
        public static Scope Measure(string name) => new Scope(name);

        private static void Record(string name, double milliseconds)
        {
            lock (Gate)
            {
                Phase phase;
                if (!Phases.TryGetValue(name, out phase))
                {
                    phase = new Phase();
                    Phases[name] = phase;
                    Order.Add(name);
                }

                phase.TotalMs += milliseconds;
            }
        }

        /// <summary>
        /// Writes a summary, at most every few seconds, of what each phase has been costing.
        /// </summary>
        /// <param name="lagging">
        /// Whether the loop is behind. A server keeping up does not need telling what it spent its
        /// spare time on.
        /// </param>
        public static void Report(bool lagging)
        {
            if (!lagging)
                return;

            long now = Watch.ElapsedMilliseconds;
            if (now - _reportedAt < ReportEvery)
                return;

            long windowStart = _reportedAt;
            _reportedAt = now;

            // Totals over the window divided by the window, not by the number of times each phase
            // was entered: a phase that runs once per world and one that runs once per tick are
            // otherwise reported on different scales, which had the enemy tick looking six times
            // cheaper than it was because five of the six worlds it was averaged over were empty.
            // Both sides in milliseconds. Dividing a total in milliseconds by a window in seconds
            // and calling it a percentage overstates everything by a thousand, which is how this
            // came to report an enemy tick at ninety-nine thousand per cent of a second.
            double windowMs = now - windowStart;
            var line = new StringBuilder($"share of the last {windowMs / 1000d:0.0}s:");

            lock (Gate)
            {
                foreach (var name in Order)
                {
                    var phase = Phases[name];
                    line.Append($" {name} {phase.TotalMs / windowMs * 100d:0.0}%");
                    phase.TotalMs = 0d;
                }
            }

            Log.Info(line.ToString());
        }

        public struct Scope : IDisposable
        {
            private readonly string _name;
            private readonly long _start;

            public Scope(string name)
            {
                _name = name;
                _start = Watch.ElapsedTicks;
            }

            public void Dispose() =>
                Record(_name, (Watch.ElapsedTicks - _start) * 1000d / Stopwatch.Frequency);
        }
    }
}

using System.Collections.Generic;
using System.Diagnostics;

namespace Hendra.World;

/// <summary>
/// Where the frame goes, measured rather than guessed at.
/// </summary>
/// <remarks>
/// <para>
/// A frame that takes seventy milliseconds has one or two things in it that take sixty, and the
/// only reliable way to find out which is to time them. Reasoning from what looks expensive is how
/// you end up optimising the draw calls on a frame that was spending its time in collision tests.
/// </para>
/// <para>
/// Each phase is averaged over a short window, because a single frame's timing is dominated by
/// whatever the garbage collector and the operating system happened to be doing. The measurement
/// itself is a stopwatch read per phase per frame -- a handful of nanoseconds against the
/// milliseconds being measured.
/// </para>
/// </remarks>
public sealed class FramePhases
{
    /// <summary>Frames averaged. About half a second at sixty, several seconds when struggling.</summary>
    private const int Window = 30;

    private sealed class Phase
    {
        public readonly double[] Samples = new double[Window];
        public int At;
        public int Count;

        public double AverageMs
        {
            get
            {
                if (Count == 0)
                    return 0d;

                double total = 0d;
                for (int i = 0; i < Count; i++)
                    total += Samples[i];

                return total / Count;
            }
        }
    }

    private readonly Dictionary<string, Phase> _phases = new();
    private readonly List<string> _order = new();
    private readonly Stopwatch _watch = Stopwatch.StartNew();

    /// <summary>Whether anything is being timed. Off, the scopes cost a branch.</summary>
    public bool Enabled { get; set; }

    /// <summary>Times a section for as long as the returned value is alive.</summary>
    public Scope Measure(string name) => new(this, name);

    private void Record(string name, double milliseconds)
    {
        if (!_phases.TryGetValue(name, out var phase))
        {
            phase = new Phase();
            _phases[name] = phase;
            _order.Add(name);
        }

        phase.Samples[phase.At] = milliseconds;
        phase.At = (phase.At + 1) % Window;
        if (phase.Count < Window)
            phase.Count++;
    }

    /// <summary>Every phase and its average, in the order they were first seen.</summary>
    public void Collect(List<(string Name, double Ms)> into)
    {
        into.Clear();
        foreach (string name in _order)
            into.Add((name, _phases[name].AverageMs));
    }

    public readonly struct Scope : System.IDisposable
    {
        private readonly FramePhases _owner;
        private readonly string _name;
        private readonly long _start;

        public Scope(FramePhases owner, string name)
        {
            _owner = owner;
            _name = name;
            _start = owner.Enabled ? owner._watch.ElapsedTicks : 0L;
        }

        public void Dispose()
        {
            if (!_owner.Enabled)
                return;

            long ticks = _owner._watch.ElapsedTicks - _start;
            _owner.Record(_name, ticks * 1000d / Stopwatch.Frequency);
        }
    }
}

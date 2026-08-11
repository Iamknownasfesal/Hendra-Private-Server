using System.Diagnostics;

namespace Hendra.Core;

/// <summary>
/// The single monotonic millisecond clock for the whole client.
///
/// The AS3 client juggled three different notions of "now" and the protocol depends on the
/// distinction, so it is worth being explicit about all three:
///
///   getTimer()   - ms since boot. Went into Pong.time, QueuePong.time, UseItem.time.
///   lastUpdate_  - getTimer() sampled at the *start of the last rendered frame*. Went into
///                  Move.time, InvSwap.time and every ack.
///   currentTime  - the value handed to update(currentTime, deltaMs). Went into the hit packets
///                  and drove projectile lifetimes.
///
/// In practice the last two are the same number: AS3 set lastUpdate_ at the end of the frame and
/// passed it as currentTime at the start of the next one. So we keep one per-frame snapshot
/// (<see cref="FrameMs"/>) and use it everywhere those two appear, and expose the live reading
/// (<see cref="NowMs"/>) for the handful of places that sampled getTimer() directly.
///
/// Backed by <see cref="Stopwatch"/> rather than the engine's frame timer, for two reasons. It has
/// to track wall time: the server's TimeCop compares our reported clock deltas against its own
/// elapsed-time deltas over a 20-sample window and silently voids shots once the ratio leaves
/// 0.92..1.08, so a clock that drifts with frame pacing would quietly stop us shooting. And keeping
/// it off the engine leaves the whole networking layer testable without booting Godot.
/// </summary>
public sealed class GameClock
{
    private readonly Stopwatch _stopwatch = new();

    public GameClock()
    {
        Reset();
    }

    /// <summary>Live reading, in ms since the clock was started.</summary>
    public int NowMs => (int)_stopwatch.ElapsedMilliseconds;

    /// <summary>
    /// <see cref="NowMs"/> sampled once at the top of the current frame. This is the AS3
    /// lastUpdate_ / currentTime value; use it for anything that goes on the wire.
    /// </summary>
    public int FrameMs { get; private set; }

    /// <summary>Milliseconds elapsed between the previous frame's snapshot and this one.</summary>
    public int DeltaMs { get; private set; }

    /// <summary>Restarts the clock at zero. Called once at boot.</summary>
    public void Reset()
    {
        _stopwatch.Restart();
        FrameMs = 0;
        DeltaMs = 0;
    }

    /// <summary>Takes this frame's snapshot. Must be called exactly once, before anything else ticks.</summary>
    public void BeginFrame()
    {
        int now = NowMs;
        DeltaMs = now - FrameMs;
        FrameMs = now;
    }
}

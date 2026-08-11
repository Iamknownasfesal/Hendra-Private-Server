using System;
using System.Collections.Generic;
using Hendra.Data;

namespace Hendra.Net;

/// <summary>
/// A short history of the local player's position, sent alongside each Move so the server can audit
/// sub-tick movement.
/// </summary>
/// <remarks>
/// One sample is recorded per rendered frame, but only one per 100 ms bucket survives: whichever
/// sample sits closest to that bucket's centre. Ten buckets are kept, so the buffer spans about a
/// second — which is also why the whole thing is cleared every time a Move goes out.
///
/// This server build parses the array and then ignores it, but the bucketing is cheap and keeping
/// it faithful costs nothing if a future build starts checking.
/// </remarks>
public sealed class MoveRecords
{
    private const int BucketMs = 100;
    private const int MaxBuckets = 10;

    /// <summary>Samples closer than this to the Move's own timestamp are held back for next time.</summary>
    private const int FreshnessMarginMs = 25;

    /// <summary>Below this gap since the last clear, a Move carries no history at all.</summary>
    private const int MinGapForRecordsMs = 125;

    private readonly List<MoveRecord> _records = new(MaxBuckets + 1);

    /// <summary>
    /// The clock reading at the last <see cref="Clear"/>. Negative until the first one, which is
    /// how recording stays disabled before we are in a world.
    /// </summary>
    public int LastClearTime { get; private set; } = -1;

    public IReadOnlyList<MoveRecord> Records => _records;

    /// <summary>Records this frame's position, subject to the bucketing rules.</summary>
    public void AddRecord(int time, float x, float y)
    {
        if (LastClearTime < 0)
            return;

        int bucket = BucketOf(time);
        if (bucket < 1 || bucket > MaxBuckets)
            return;

        if (_records.Count == 0)
        {
            _records.Add(new MoveRecord(time, x, y));
            return;
        }

        var last = _records[^1];
        if (bucket != BucketOf(last.Time))
        {
            _records.Add(new MoveRecord(time, x, y));
            return;
        }

        // Same bucket as the previous sample: keep whichever is nearer its centre.
        if (DistanceFromBucketCentre(bucket, time) < DistanceFromBucketCentre(bucket, last.Time))
            _records[^1] = new MoveRecord(time, x, y);
    }

    /// <summary>
    /// Fills <paramref name="destination"/> with the records that should accompany a Move stamped
    /// <paramref name="moveTime"/>, then resets the buffer.
    /// </summary>
    public void TakeForMove(int moveTime, List<MoveRecord> destination)
    {
        destination.Clear();

        if (LastClearTime >= 0 && moveTime - LastClearTime > MinGapForRecordsMs)
        {
            int count = Math.Min(MaxBuckets, _records.Count);
            for (int i = 0; i < count; i++)
            {
                // Stop at the first sample too close to the Move's own timestamp; it belongs to
                // the next batch. The original breaks rather than skipping, so ordering is kept.
                if (_records[i].Time >= moveTime - FreshnessMarginMs)
                    break;
                destination.Add(_records[i]);
            }
        }

        Clear(moveTime);
    }

    /// <summary>Empties the buffer and rebases the bucket grid on <paramref name="time"/>.</summary>
    public void Clear(int time)
    {
        _records.Clear();
        LastClearTime = time;
    }

    /// <summary>Disables recording until the next <see cref="Clear"/>, e.g. on disconnect.</summary>
    public void Reset()
    {
        _records.Clear();
        LastClearTime = -1;
    }

    private int BucketOf(int time) => (time - LastClearTime + BucketMs / 2) / BucketMs;

    private int DistanceFromBucketCentre(int bucket, int time) =>
        Math.Abs(time - LastClearTime - bucket * BucketMs);
}

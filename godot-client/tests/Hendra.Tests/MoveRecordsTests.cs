using System.Collections.Generic;
using Hendra.Data;
using Hendra.Net;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// The position history that rides along with each Move packet.
///
/// This server build parses the array and ignores it, so none of this is currently load bearing —
/// but it is the piece a future anti-speedhack check would read, and the bucketing rules are
/// fiddly enough to be worth pinning down while the original is still available to compare against.
/// </summary>
public sealed class MoveRecordsTests
{
    private static List<MoveRecord> Take(MoveRecords records, int moveTime)
    {
        var destination = new List<MoveRecord>();
        records.TakeForMove(moveTime, destination);
        return destination;
    }

    [Fact]
    public void RecordingIsDisabledUntilTheFirstClear()
    {
        // LastClearTime starts negative, which is how the original suppresses recording before the
        // player is in a world.
        var records = new MoveRecords();
        records.AddRecord(1000, 1f, 1f);
        Assert.Empty(records.Records);
    }

    [Fact]
    public void OneSampleSurvivesPerHundredMillisecondBucket()
    {
        var records = new MoveRecords();
        records.Clear(0);

        // Four frames inside the first two buckets.
        records.AddRecord(90, 1f, 1f);
        records.AddRecord(110, 2f, 2f);
        records.AddRecord(190, 3f, 3f);
        records.AddRecord(210, 4f, 4f);

        Assert.Equal(2, records.Records.Count);
    }

    [Fact]
    public void TheSampleNearestTheBucketCentreWins()
    {
        var records = new MoveRecords();
        records.Clear(0);

        records.AddRecord(80, 1f, 1f);   // 20 ms from the 100 ms centre
        records.AddRecord(103, 2f, 2f);  // 3 ms from it — should replace the first
        records.AddRecord(140, 3f, 3f);  // 40 ms away — should be ignored

        Assert.Single(records.Records);
        Assert.Equal(103, records.Records[0].Time);
        Assert.Equal(2f, records.Records[0].X);
    }

    [Fact]
    public void SamplesOutsideTheTenBucketWindowAreDropped()
    {
        var records = new MoveRecords();
        records.Clear(0);

        // Bucket 0 (anything under 50 ms) and bucket 11+ are both rejected, so the buffer spans
        // about a second.
        records.AddRecord(10, 1f, 1f);
        records.AddRecord(1100, 2f, 2f);
        Assert.Empty(records.Records);

        records.AddRecord(500, 3f, 3f);
        Assert.Single(records.Records);
    }

    [Fact]
    public void NoHistoryIsSentWhenTheGapSinceTheLastMoveIsShort()
    {
        // Under 125 ms since the last clear, the Move goes out with an empty array.
        var records = new MoveRecords();
        records.Clear(0);
        records.AddRecord(100, 1f, 1f);

        Assert.Empty(Take(records, 120));
    }

    [Fact]
    public void SamplesTooCloseToTheMoveTimestampAreHeldBack()
    {
        var records = new MoveRecords();
        records.Clear(0);
        records.AddRecord(100, 1f, 1f);
        records.AddRecord(200, 2f, 2f);
        records.AddRecord(300, 3f, 3f);

        // Anything within 25 ms of the Move's own timestamp belongs to the next batch. The original
        // stops at the first such sample rather than skipping it, which keeps the array ordered.
        var sent = Take(records, 310);
        Assert.Equal(2, sent.Count);
        Assert.Equal(100, sent[0].Time);
        Assert.Equal(200, sent[1].Time);
    }

    [Fact]
    public void TakingForAMoveResetsTheBuffer()
    {
        var records = new MoveRecords();
        records.Clear(0);
        records.AddRecord(100, 1f, 1f);
        records.AddRecord(200, 2f, 2f);

        Assert.NotEmpty(Take(records, 400));
        Assert.Empty(records.Records);
        Assert.Equal(400, records.LastClearTime);

        // The bucket grid is rebased on the new clear time, so a sample just after it lands in
        // bucket 1 rather than being rejected as stale.
        records.AddRecord(500, 3f, 3f);
        Assert.Single(records.Records);
    }

    [Fact]
    public void AtMostTenRecordsAreSent()
    {
        var records = new MoveRecords();
        records.Clear(0);
        for (int i = 1; i <= 10; i++)
            records.AddRecord(i * 100, i, i);

        Assert.Equal(10, Take(records, 2000).Count);
    }

    [Fact]
    public void ResetDisablesRecordingAgain()
    {
        // Used on disconnect, so a stale buffer cannot leak into the next session.
        var records = new MoveRecords();
        records.Clear(0);
        records.AddRecord(100, 1f, 1f);

        records.Reset();
        Assert.Empty(records.Records);

        records.AddRecord(200, 2f, 2f);
        Assert.Empty(records.Records);
    }
}

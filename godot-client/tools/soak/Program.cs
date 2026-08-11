using System;
using System.Diagnostics;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Hendra.Core;
using Hendra.Data;
using Hendra.Net;
using Hendra.Net.Packets;

namespace Hendra.Soak;

/// <summary>
/// Drives the real GameSession against a running world server and reports whether the connection
/// survives.
///
/// This is the test the unit suite cannot do: acknowledgement conservation, the handshake, the
/// continuous RC4 streams and the RSA credentials only prove themselves against the actual server.
/// Getting any of them wrong shows up as a connection that simply stops, with nothing logged
/// client-side, so the measurement here is elapsed time and packet counts rather than assertions.
/// </summary>
internal static class Program
{
    private static async Task<int> Main(string[] args)
    {
        string host = Arg(args, "--host", "127.0.0.1");
        int port = int.Parse(Arg(args, "--port", "2050"));
        string guid = Arg(args, "--guid", "test@test.com");
        string password = Arg(args, "--password", "test");
        int seconds = int.Parse(Arg(args, "--seconds", "300"));
        bool create = args.Contains("--create");
        int charId = int.Parse(Arg(args, "--char", "0"));
        ushort classType = ushort.Parse(Arg(args, "--class", "782"));

        var clock = new GameClock();
        var session = new GameSession(clock);

        int updates = 0, ticks = 0, gotos = 0;
        string ended = null;
        var mapName = "(none)";

        session.Disconnected += reason =>
        {
            ended ??= reason;
            Console.WriteLine($"[{clock.NowMs,7}ms] DISCONNECTED: {reason}");
        };
        session.Failed += failure =>
        {
            ended ??= $"Failure {failure.ErrorId}: {failure.ErrorDescription}";
            Console.WriteLine($"[{clock.NowMs,7}ms] FAILURE {failure.ErrorId}: {failure.ErrorDescription}");
        };
        session.MapLoaded += map =>
        {
            mapName = map.Name;
            Console.WriteLine(
                $"[{clock.NowMs,7}ms] MapInfo: {map.Name} {map.Width}x{map.Height}, seed {map.Seed}, " +
                $"{map.ClientXml.Length} client xml");
        };
        session.Entered += ok =>
            Console.WriteLine($"[{clock.NowMs,7}ms] CreateSuccess: object {ok.ObjectId}, char {ok.CharId}");
        session.WorldUpdated += update =>
        {
            updates++;

            // Our spawn position arrives here, as a stat block for our own object id. Without it we
            // would report (0, 0) in every Move, and the server's no-clip check disconnects on the
            // first tick -- silently, with nothing written to its log.
            foreach (var definition in update.NewObjects)
            {
                if (definition.Stats.ObjectId != session.PlayerObjectId)
                    continue;
                session.PlayerX = definition.Stats.Position.X;
                session.PlayerY = definition.Stats.Position.Y;
                Console.WriteLine(
                    $"[{clock.NowMs,7}ms] spawned at ({session.PlayerX:F2}, {session.PlayerY:F2})");
            }

            if (updates <= 3)
            {
                Console.WriteLine(
                    $"[{clock.NowMs,7}ms] Update #{updates}: {update.Tiles.Length} tiles, " +
                    $"{update.NewObjects.Length} objects, {update.Drops.Length} drops");
            }
        };
        session.Ticked += _ => ticks++;
        session.Repositioned += got0 =>
        {
            gotos++;
            if (got0.ObjectId == session.PlayerObjectId)
            {
                session.PlayerX = got0.Position.X;
                session.PlayerY = got0.Position.Y;
                Console.WriteLine(
                    $"[{clock.NowMs,7}ms] Goto: moved to ({got0.Position.X:F2}, {got0.Position.Y:F2})");
            }
        };
        session.QueueUpdated += queue =>
            Console.WriteLine($"[{clock.NowMs,7}ms] Queued at {queue.Position}/{queue.Count}");

        // Our own starting position arrives in the first Update, as a stat block for our object id.
        session.PacketReceived += packet =>
        {
            if (packet is TextPacket text && !string.IsNullOrEmpty(text.Text))
                Console.WriteLine($"[{clock.NowMs,7}ms] Text <{text.Name}> {text.Text}");
        };

        Console.WriteLine($"Connecting to {host}:{port} as {guid} ({(create ? "new character" : $"char {charId}")})...");

        try
        {
            if (create)
                await session.ConnectToCreateAsync(host, port, guid, password, -2, charId, classType, 0);
            else
                await session.ConnectToLoadAsync(host, port, guid, password, -2, charId);
        }
        catch (Exception ex)
        {
            Console.WriteLine($"Could not connect: {ex.Message}");
            return 2;
        }

        var stopwatch = Stopwatch.StartNew();
        var report = TimeSpan.FromSeconds(30);
        var nextReport = report;

        // Roughly sixty frames a second, matching what the client would do. The clock has to keep
        // pace with wall time or the server's shot-timing window rejects us.
        while (stopwatch.Elapsed.TotalSeconds < seconds && ended == null)
        {
            clock.BeginFrame();
            session.RecordPosition();
            session.Poll();

            if (stopwatch.Elapsed > nextReport)
            {
                Console.WriteLine(
                    $"[{clock.NowMs,7}ms] alive {stopwatch.Elapsed.TotalSeconds:F0}s — " +
                    $"{ticks} ticks, {updates} updates, {gotos} gotos, state {session.State}");
                nextReport += report;
            }

            await Task.Delay(16);
        }

        stopwatch.Stop();
        Console.WriteLine();
        Console.WriteLine($"map          : {mapName}");
        Console.WriteLine($"survived     : {stopwatch.Elapsed.TotalSeconds:F1}s of {seconds}s");
        Console.WriteLine($"ticks        : {ticks}  (one Move sent per tick)");
        Console.WriteLine($"updates      : {updates}  (one UpdateAck sent per update)");
        Console.WriteLine($"gotos        : {gotos}  (one GotoAck sent per goto)");
        Console.WriteLine($"final state  : {session.State}");
        Console.WriteLine($"ended        : {ended ?? "no — still connected"}");

        session.Close("Soak finished.");
        return ended == null ? 0 : 1;
    }

    private static string Arg(string[] args, string name, string fallback)
    {
        int i = Array.IndexOf(args, name);
        return i >= 0 && i + 1 < args.Length ? args[i + 1] : fallback;
    }
}

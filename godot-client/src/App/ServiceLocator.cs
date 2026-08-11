using Godot;
using Hendra.Core;
using Hendra.Net;

namespace Hendra.App;

/// <summary>
/// The one global the client has: it owns the process-wide clock and the world-server session, and
/// drives them once per frame.
/// </summary>
/// <remarks>
/// Registered as the <c>Svc</c> autoload. This deliberately replaces the AS3 client's Robotlegs
/// container and its <c>StaticInjectorContext</c> escape hatch — roughly thirty <c>*Config</c>
/// classes and a service locator reachable from anywhere, used to wire objects that could simply
/// have been constructed. Everything else in the client is built and passed explicitly; only the
/// two things that genuinely are process-global live here.
///
/// Ordering within a frame matters and is fixed here rather than left to node order:
/// the clock snapshot has to be taken before anything reads it, and the session must be polled
/// after the world has published the player's position, since that is what the outgoing Move
/// packet reads.
/// </remarks>
public partial class ServiceLocator : Node
{
    private static ServiceLocator _instance;

    /// <summary>The monotonic millisecond clock. Everything that goes on the wire is stamped from it.</summary>
    public static GameClock Clock => _instance._clock;

    /// <summary>The world-server session. Null until a game is entered.</summary>
    public static GameSession Session => _instance._session;

    private readonly GameClock _clock = new();
    private GameSession _session;

    public override void _EnterTree()
    {
        _instance = this;
        _clock.Reset();

        // Keep ticking while the window is unfocused. The server's keepalive and ack deadlines run
        // on wall time, so a client that stops processing for twelve seconds gets disconnected.
        ProcessMode = ProcessModeEnum.Always;
    }

    public override void _ExitTree()
    {
        _session?.Dispose();
        _session = null;
        if (_instance == this)
            _instance = null;
    }

    /// <summary>Creates a fresh session, discarding any previous one.</summary>
    public static GameSession BeginSession()
    {
        _instance._session?.Dispose();
        _instance._session = new GameSession(_instance._clock);
        return _instance._session;
    }

    /// <summary>Tears down the current session, if any.</summary>
    public static void EndSession(string reason = "Left the game.")
    {
        if (_instance._session == null)
            return;
        _instance._session.Close(reason);
        _instance._session.Dispose();
        _instance._session = null;
    }

    public override void _Process(double delta)
    {
        _clock.BeginFrame();

        // The world publishes the player's position during its own _Process, which Godot runs
        // after this node because the autoload sits at the top of the tree. Polling here would
        // therefore send a Move built from last frame's position, so draining the socket is left
        // to the game scene, which calls Session.Poll() at the right point in its own update.
    }
}

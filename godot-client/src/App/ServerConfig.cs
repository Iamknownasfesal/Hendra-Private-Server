namespace Hendra.App;

/// <summary>
/// Where the game lives. Built into the client rather than typed in.
/// </summary>
/// <remarks>
/// <para>
/// The original hard-codes its app server and never shows an address to the player: the title
/// screen's Servers option picks between named worlds the app server hands back, not between
/// machines. Asking a player for an IP is a development affordance that had no business being on
/// the front page.
/// </para>
/// <para>
/// The world server's address is not here at all. It comes from <c>/char/list</c>, along with the
/// characters and the account — which is the one genuinely surprising thing about that endpoint.
/// </para>
/// </remarks>
public static class ServerConfig
{
    /// <summary>The host the app server runs on.</summary>
    public const string Host = "127.0.0.1";

    /// <summary>The app server's port. Fixed in this fork; the world server is always 2050.</summary>
    public const int AppPort = 8888;

    /// <summary>The app server's base URL.</summary>
    public static string AppServer => $"http://{Host}:{AppPort}";
}

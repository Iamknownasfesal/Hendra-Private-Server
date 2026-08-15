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

    /// <summary>
    /// The app server's port.
    /// </summary>
    /// <remarks>
    /// The original hard-codes this. It is settable here so a development run can be pointed at one
    /// app server while another keeps answering on the usual port, which is the only way to tell
    /// which of two servers actually answered a request. <c>--app-port</c> sets it at boot.
    /// </remarks>
    /// <remarks>
    /// 8080 is where <c>hendra-app</c> listens. The original C# app server answers on 8888, and
    /// pointing here at 8888 sends the character list, the language strings, the interface audio and
    /// the death screen to that server instead — which reads as a client that cannot find its own
    /// characters, because the two servers keep separate accounts.
    /// </remarks>
    public static int AppPort { get; set; } = 8080;

    /// <summary>The app server's base URL.</summary>
    public static string AppServer => $"http://{Host}:{AppPort}";
}

using System;

namespace Hendra.UI;

/// <summary>
/// What this client calls itself, in the two lines every screen that prints them uses.
/// </summary>
/// <remarks>
/// The title screen and the Escape menu both carry the pair, and the reference sets them in the
/// same words in both places. Composed once here so the two cannot drift apart: a version bumped
/// on one screen and not the other is a number a player quotes back wrong.
/// </remarks>
public static class ClientBuild
{
    /// <summary>
    /// The five-part figure, the way the original prints its own.
    /// </summary>
    /// <remarks>
    /// The protocol carries a separate build string that the server checks — this one is for the
    /// player, and the two are allowed to differ.
    /// </remarks>
    public const string Version = "0.1.0.0.0";

    /// <summary>Whose client this is, for the copyright line.</summary>
    public const string Owner = "Hendra";

    /// <summary>
    /// A short identifier for exactly this build.
    /// </summary>
    /// <remarks>
    /// Taken from the assembly's module identity, which the compiler regenerates on every build. A
    /// hand-written constant here would be a number that stops moving the first time somebody
    /// forgets to bump it, which makes the line worse than useless when a player quotes it.
    /// </remarks>
    public static string BuildId =>
        System.Reflection.Assembly.GetExecutingAssembly().ManifestModule.ModuleVersionId
            .ToString("N")[..9];

    /// <summary>The upper of the two lines: what this build is.</summary>
    public static string VersionLine => $"v {Version}, build id: {BuildId}";

    /// <summary>The lower of the two: whose it is.</summary>
    public static string CopyrightLine =>
        $"Copyright © {DateTime.Now.Year} {Owner}. All Rights reserved.";
}

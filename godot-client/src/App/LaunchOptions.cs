using System;
using Godot;

namespace Hendra.App;

/// <summary>
/// Command-line options, for skipping straight past the login screen during development.
/// </summary>
/// <remarks>
/// Passed after a bare <c>--</c>, which is where Godot puts arguments meant for the game rather
/// than the engine:
/// <code>
/// godot-mono -- --host 127.0.0.1 --guid me@example.com --password pw --char 1
/// </code>
/// The screenshot option exists so the renderer can be checked against a real world without a
/// person sitting in front of it.
/// </remarks>
public sealed class LaunchOptions
{
    public string Host { get; private set; }
    public int Port { get; private set; } = 2050;
    public string Guid { get; private set; }
    public string Password { get; private set; }
    public int CharacterId { get; private set; } = -1;

    /// <summary>Where to write a screenshot, or null to take none.</summary>
    public string ScreenshotPath { get; private set; }

    /// <summary>How long to wait before the screenshot, letting the world stream in first.</summary>
    public float ScreenshotDelaySeconds { get; private set; } = 6f;

    /// <summary>Quit after taking the screenshot. Useful for scripted checks.</summary>
    public bool QuitAfterScreenshot { get; private set; }

    /// <summary>Hold fire from the moment the world loads, for unattended checks of the combat path.</summary>
    public bool Autofire { get; private set; }

    /// <summary>Whether enough was supplied to connect without the login screen.</summary>
    public bool CanAutoConnect =>
        !string.IsNullOrEmpty(Host) && !string.IsNullOrEmpty(Guid) && CharacterId >= 0;

    public static LaunchOptions Parse()
    {
        var options = new LaunchOptions();
        string[] args = OS.GetCmdlineUserArgs();

        for (int i = 0; i < args.Length; i++)
        {
            string Next() => i + 1 < args.Length ? args[++i] : null;

            switch (args[i])
            {
                case "--host": options.Host = Next(); break;
                case "--port": options.Port = ParseInt(Next(), options.Port); break;
                case "--guid": options.Guid = Next(); break;
                case "--password": options.Password = Next(); break;
                case "--char": options.CharacterId = ParseInt(Next(), -1); break;
                case "--screenshot": options.ScreenshotPath = Next(); break;
                case "--screenshot-after":
                    options.ScreenshotDelaySeconds = ParseFloat(Next(), options.ScreenshotDelaySeconds);
                    break;
                case "--quit-after-screenshot": options.QuitAfterScreenshot = true; break;
                case "--autofire": options.Autofire = true; break;
            }
        }

        return options;
    }

    private static int ParseInt(string text, int fallback) =>
        int.TryParse(text, out int value) ? value : fallback;

    private static float ParseFloat(string text, float fallback) =>
        float.TryParse(text, System.Globalization.NumberStyles.Float,
            System.Globalization.CultureInfo.InvariantCulture, out float value)
            ? value
            : fallback;

    /// <summary>A server entry describing the host these options point at.</summary>
    public Account.ServerInfo ToServer() => new()
    {
        Name = "Command line",
        Address = Host,
        Port = Port,
    };
}

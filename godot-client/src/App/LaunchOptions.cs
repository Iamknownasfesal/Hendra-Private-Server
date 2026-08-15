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

    /// <summary>
    /// The object type of a class to create a character for, instead of playing an existing one.
    /// </summary>
    /// <remarks>
    /// An object type -- Archer is 775 -- not an index into the class list. The two are easy to
    /// confuse and the server answers an index with a disconnect rather than an explanation.
    /// </remarks>
    public int CreateClassType { get; private set; } = -1;

    /// <summary>
    /// Stop on the title screen, whatever else was passed.
    /// </summary>
    /// <remarks>
    /// The title screen is what the client shows when it has been given nothing to connect with, so
    /// it can normally be reached by leaving the connection flags off. This says so explicitly
    /// instead, which is what lets an unattended capture of it use the same command line as every
    /// other capture rather than a shorter one that also has to remember not to sign in.
    /// </remarks>
    public bool StopOnTitle { get; private set; }

    public bool CanAutoCreate =>
        !StopOnTitle && !string.IsNullOrEmpty(Host) && !string.IsNullOrEmpty(Guid) && CreateClassType >= 0;

    /// <summary>Where to write a screenshot, or null to take none.</summary>
    public string ScreenshotPath { get; private set; }

    /// <summary>How long to wait before the screenshot, letting the world stream in first.</summary>
    public float ScreenshotDelaySeconds { get; private set; } = 6f;

    /// <summary>Quit after taking the screenshot. Useful for scripted checks.</summary>
    public bool QuitAfterScreenshot { get; private set; }

    /// <summary>Hold fire from the moment the world loads, for unattended checks of the combat path.</summary>
    public bool Autofire { get; private set; }

    /// <summary>Use the equipped ability on a loop, for unattended checks of the ability path.</summary>
    public bool AutoAbility { get; private set; }

    /// <summary>
    /// Walk in a slow circle instead of standing still.
    /// </summary>
    /// <remarks>
    /// For checking that other people's characters move: on a server whose monsters have no
    /// behaviours, another player is the only thing in the world that ever changes position.
    /// </remarks>
    public bool AutoWalk { get; private set; }

    /// <summary>Opens the character sheet on arrival, so an unattended run can screenshot it.</summary>
    public bool OpenCharacterPanel { get; private set; }

    /// <summary>The same, for the account panel.</summary>
    public bool OpenAccountPanel { get; private set; }

    /// <summary>Opens the vault panel once in the world, for unattended screenshots of it.</summary>
    public bool OpenVault { get; private set; }

    /// <summary>Opens the characters panel, in the world or on the sign-in page if there is no world.</summary>
    public bool OpenCharacters { get; private set; }

    /// <summary>Which of its two tabs to open on, or null for the living.</summary>
    public string CharactersTab { get; private set; }

    /// <summary>Opens the create-a-character page, by the same two routes.</summary>
    public bool OpenNewCharacter { get; private set; }
    /// <summary>Opens the potion rack once in the world, for unattended screenshots of it.</summary>
    public bool OpenPotionRack { get; private set; }

    /// <summary>Opens the options page once in the world. Same purpose as the three above.</summary>
    public bool OpenOptions { get; private set; }

    /// <summary>Opens the Escape menu once in the world, so it can be screenshotted unattended.</summary>
    public bool OpenMenu { get; private set; }

    /// <summary>Which of its tabs to open on, or null for the first.</summary>
    public string OptionsTab { get; private set; }

    /// <summary>
    /// Starting camera heading in degrees, or null for the usual one.
    /// </summary>
    /// <remarks>
    /// The projection folds height into the ground plane along the camera's up vector, so a great
    /// deal of the renderer only looks right at one heading by accident. Being able to start at
    /// another is how that gets checked without a person holding a key down.
    /// </remarks>
    public float? CameraAngleDegrees { get; private set; }

    /// <summary>
    /// Lines to send once the world is up, in order, a couple of seconds apart.
    /// </summary>
    /// <remarks>
    /// Repeat <c>--say</c> for each. Slash commands go down the same pipe as chat, so this is how an
    /// unattended run reaches anything the server only does on request — and more than one line is
    /// needed for anything that has to leave the Nexus first, since you cannot die there.
    /// </remarks>
    public System.Collections.Generic.List<string> Say { get; } = new();

    /// <summary>Whether enough was supplied to connect without the login screen.</summary>
    public bool CanAutoConnect =>
        !StopOnTitle && !string.IsNullOrEmpty(Host) && !string.IsNullOrEmpty(Guid) && CharacterId >= 0;

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
                case "--create": options.CreateClassType = ParseInt(Next(), -1); break;
                case "--screenshot": options.ScreenshotPath = Next(); break;
                case "--screenshot-after":
                    options.ScreenshotDelaySeconds = ParseFloat(Next(), options.ScreenshotDelaySeconds);
                    break;
                case "--quit-after-screenshot": options.QuitAfterScreenshot = true; break;
                case "--autofire": options.Autofire = true; break;
                case "--use-ability": options.AutoAbility = true; break;
                case "--walk": options.AutoWalk = true; break;
                case "--character": options.OpenCharacterPanel = true; break;
                case "--account": options.OpenAccountPanel = true; break;
                case "--vault": options.OpenVault = true; break;
                case "--title": options.StopOnTitle = true; break;
                case "--character-select":
                {
                    options.OpenCharacters = true;

                    // An optional tab name after it, but only if what follows is not another flag.
                    if (i + 1 < args.Length && !args[i + 1].StartsWith("--"))
                        options.CharactersTab = args[++i];

                    break;
                }
                case "--new-character": options.OpenNewCharacter = true; break;
                case "--rack": options.OpenPotionRack = true; break;
                case "--menu": options.OpenMenu = true; break;
                case "--options":
                {
                    options.OpenOptions = true;

                    // An optional tab name after it, but only if what follows is not another flag.
                    if (i + 1 < args.Length && !args[i + 1].StartsWith("--"))
                        options.OptionsTab = args[++i];

                    break;
                }
                case "--camera-angle": options.CameraAngleDegrees = ParseFloat(Next(), 0f); break;
                case "--say":
                {
                    string line = Next();
                    if (!string.IsNullOrEmpty(line))
                        options.Say.Add(line);
                    break;
                }
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

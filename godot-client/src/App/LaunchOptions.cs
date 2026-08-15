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

    /// <summary>
    /// The port the account, character-list and guild requests go to.
    /// </summary>
    /// <remarks>
    /// Separate from <see cref="Port"/>, which is the world server. Defaults to the built-in 8888 so
    /// an ordinary run is unchanged; pointing it elsewhere is how a run is aimed at one app server
    /// while another is still listening on the usual port.
    /// </remarks>
    public int AppPort { get; private set; } = ServerConfig.AppPort;
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

    public bool CanAutoCreate =>
        !string.IsNullOrEmpty(Host) && !string.IsNullOrEmpty(Guid) && CreateClassType >= 0;

    /// <summary>
    /// Go in through the login screen rather than around it.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The difference between this and <see cref="CanAutoConnect"/> is the whole point of it. Those
    /// build a session straight from the command line, which is fine for checking a world but
    /// proves nothing about how anybody actually gets into one: they never touch the login screen,
    /// the character list or the account server behind them. This fills in the form and presses the
    /// same buttons a player presses, so an unattended run exercises the path a player takes.
    /// </para>
    /// <para>
    /// The credentials come from <c>--guid</c> and <c>--password</c>, which are only ever a name and
    /// a password; it is <c>--char</c> and <c>--create</c> that skip the interface.
    /// </para>
    /// </remarks>
    public bool SignInThroughInterface { get; private set; }

    /// <summary>Registers the account first, for a run that is testing a brand-new player.</summary>
    public bool RegisterFirst { get; private set; }

    /// <summary>The name to claim, when signing in reaches an account that has not chosen one.</summary>
    public string ClaimName { get; private set; }

    /// <summary>
    /// The class to create from the character list, by object type. Wizard is 782.
    /// </summary>
    public int PickClassType { get; private set; } = -1;

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

    /// <summary>Stops the client testing the ground before it reports where it is.</summary>
    /// <remarks>
    /// A wall-hacked client in one flag. For watching the server refuse a claim it has to refuse on
    /// its own, since a client that checks its own collision never sends one.
    /// </remarks>
    public bool NoClip { get; private set; }

    /// <summary>How far a <c>--noclip</c> client jumps in one frame, in tiles.</summary>
    /// <remarks>
    /// The distance is the point: a claim shorter than a tile has no wall between its ends, and a
    /// server could get away with checking only where it lands. This is the claim that has one.
    /// </remarks>
    public float Lurch { get; private set; }

    /// <summary>Opens the character sheet on arrival, so an unattended run can screenshot it.</summary>
    public bool OpenCharacterPanel { get; private set; }

    /// <summary>The same, for the account panel.</summary>
    public bool OpenAccountPanel { get; private set; }

    /// <summary>Opens the vault panel once in the world, for unattended screenshots of it.</summary>
    public bool OpenVault { get; private set; }

    /// <summary>Opens the options page once in the world. Same purpose as the three above.</summary>
    public bool OpenOptions { get; private set; }

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

    /// <summary>
    /// Lines to send once every scripted gesture has been made, in order.
    /// </summary>
    /// <remarks>
    /// Repeat <c>--then</c> for each. <see cref="Say"/> runs before the gestures and drains in the
    /// world the run starts in, which is the wrong side of a gesture that changes worlds: opening a
    /// dungeon with a key and stepping through the door leaves the player somewhere no line has
    /// been said yet. These are said there.
    /// </remarks>
    public System.Collections.Generic.List<string> Afterwards { get; } = new();

    /// <summary>How many gifts to claim from the vault panel, once it knows what is waiting.</summary>
    /// <remarks>
    /// The panel answers to clicks, which an unattended run cannot make. Same reason
    /// <see cref="OpenVault"/> exists.
    /// </remarks>
    public int ClaimGifts { get; set; }

    /// <summary>Whether to buy one more vault chest once the panel knows what the vault is.</summary>
    public bool BuyVaultChest { get; set; }

    /// <summary>How many times to press Buy at the vendor in reach, for unattended checks of a shop.</summary>
    public int BuyFromMerchant { get; private set; }

    /// <summary>Vault squares to take out into the pack, by flat index, in order.</summary>
    public System.Collections.Generic.Queue<int> TakeFromVault { get; } = new();

    /// <summary>
    /// Drags to make in the player's own inventory, each a pair of flat slot numbers.
    /// </summary>
    /// <remarks>
    /// A drag is a press, a movement and a release, and an unattended run can make none of them.
    /// This hands the same pair of slots to the same handler the release does, which is everything
    /// a drag is once the mouse has finished with it.
    /// </remarks>
    public System.Collections.Generic.Queue<(int From, int To)> Drags { get; } = new();

    /// <summary>Slots to click, by flat number, in order. What clicking one does depends on it.</summary>
    public System.Collections.Generic.Queue<int> Activations { get; } = new();

    /// <summary>Slots to drop on the ground, by flat number, in order.</summary>
    public System.Collections.Generic.Queue<int> Discards { get; } = new();

    /// <summary>Squares of the bag underfoot to take from, in order.</summary>
    public System.Collections.Generic.Queue<int> TakeFromBag { get; } = new();

    /// <summary>One entry per press of the interact key, on whatever the player is standing next to.</summary>
    /// <remarks>
    /// A queue rather than a count for the same reason the scripted lines are one: it is handed to
    /// each controller in turn and so survives a change of world, and stepping through a portal is
    /// exactly a change of world. Interacting is the only way into one, and a portal a key opened
    /// stands where the player already is, so a run that can use the key but not step through it
    /// can never reach the world behind the door.
    /// </remarks>
    public System.Collections.Generic.Queue<int> Interactions { get; } = new();

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
                case "--app-port": options.AppPort = ParseInt(Next(), options.AppPort); break;
                case "--guid": options.Guid = Next(); break;
                case "--password": options.Password = Next(); break;
                case "--char": options.CharacterId = ParseInt(Next(), -1); break;
                case "--create": options.CreateClassType = ParseInt(Next(), -1); break;
                case "--through-ui": options.SignInThroughInterface = true; break;
                case "--register": options.RegisterFirst = true; break;
                case "--claim-name": options.ClaimName = Next(); break;
                case "--pick-class": options.PickClassType = ParseInt(Next(), -1); break;
                case "--screenshot": options.ScreenshotPath = Next(); break;
                case "--screenshot-after":
                    options.ScreenshotDelaySeconds = ParseFloat(Next(), options.ScreenshotDelaySeconds);
                    break;
                case "--quit-after-screenshot": options.QuitAfterScreenshot = true; break;
                case "--autofire": options.Autofire = true; break;
                case "--use-ability": options.AutoAbility = true; break;
                case "--walk": options.AutoWalk = true; break;
                case "--noclip": options.NoClip = true; break;
                case "--lurch": options.Lurch = ParseFloat(Next(), 0f); break;
                case "--character": options.OpenCharacterPanel = true; break;
                case "--account": options.OpenAccountPanel = true; break;
                case "--vault": options.OpenVault = true; break;
                case "--vault-claim":
                    options.ClaimGifts = ParseInt(Next(), 1);
                    break;
                case "--vault-buy": options.BuyVaultChest = true; break;
                case "--buy": options.BuyFromMerchant = ParseInt(Next(), 1); break;
                case "--vault-take":
                    options.TakeFromVault.Enqueue(ParseInt(Next(), 0));
                    break;
                case "--take":
                    options.TakeFromBag.Enqueue(ParseInt(Next(), -1));
                    break;
                case "--drop":
                    options.Discards.Enqueue(ParseInt(Next(), -1));
                    break;
                case "--click":
                    options.Activations.Enqueue(ParseInt(Next(), -1));
                    break;
                case "--interact":
                    options.Interactions.Enqueue(0);
                    break;
                case "--drag":
                {
                    int from = ParseInt(Next(), -1);
                    int to = ParseInt(Next(), -1);
                    if (from >= 0 && to >= 0)
                        options.Drags.Enqueue((from, to));
                    break;
                }
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
                case "--then":
                {
                    string line = Next();
                    if (!string.IsNullOrEmpty(line))
                        options.Afterwards.Add(line);
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

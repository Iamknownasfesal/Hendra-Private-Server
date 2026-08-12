using System;
using Godot;
using Hendra.Assets;
using Hendra.Core;
using Hendra.Net;
using Hendra.Resources;
using Hendra.Text;

namespace Hendra.App;

/// <summary>
/// The one global the client has: the process-wide clock, the loaded game data, and the current
/// world-server session.
/// </summary>
/// <remarks>
/// Registered as the <c>Svc</c> autoload. This deliberately replaces the original's Robotlegs
/// container and its <c>StaticInjectorContext</c> escape hatch — some thirty config classes and a
/// service locator reachable from anywhere, used to wire objects that could simply have been
/// constructed. Everything else in the client is built and passed explicitly; only the things that
/// genuinely are process-global live here.
/// </remarks>
public partial class ServiceLocator : Node
{
    private static ServiceLocator _instance;

    private readonly GameClock _clock = new();
    private GameSession _session;
    private Audio.AudioLibrary _audio;
    private Settings _settings;

    /// <summary>The monotonic millisecond clock. Everything that goes on the wire is stamped from it.</summary>
    public static GameClock Clock => _instance._clock;

    /// <summary>Sprites and animations. Null until <see cref="LoadContent"/> has run.</summary>
    public static AssetLibrary Assets { get; private set; }

    /// <summary>Object and terrain definitions. Null until <see cref="LoadContent"/> has run.</summary>
    public static GameData Data { get; private set; }

    /// <summary>
    /// Localised text. Empty until <see cref="LoadLanguageAsync"/> completes, which is fine —
    /// an unresolved key renders as itself rather than as nothing.
    /// </summary>
    public static StringMap Strings { get; } = new();

    /// <summary>The current session, or null when not in a game.</summary>
    public static GameSession Session => _instance?._session;

    /// <summary>
    /// Sound effects and music.
    /// </summary>
    /// <remarks>
    /// Process-global like the clock, and for the same reason: it holds fetched audio that should
    /// outlive any one session, so a trip to the Nexus does not silence the game while it downloads
    /// the same files again.
    /// </remarks>
    public static Audio.AudioLibrary Audio => _instance?._audio;

    /// <summary>The player's saved preferences. Loaded once, at startup.</summary>
    public static Settings Settings => _instance?._settings;

    /// <summary>Pushes the current settings to whatever they govern, and writes them out.</summary>
    public static void ApplySettings()
    {
        if (_instance?._settings == null)
            return;

        var options = _instance._settings;

        if (_instance._audio != null)
        {
            // Master multiplies the other two rather than replacing them, so turning everything
            // down and back up leaves the balance between music and effects where it was.
            _instance._audio.EffectVolume = options.EffectVolume * options.MasterVolume;
            _instance._audio.MusicVolume = options.MusicVolume * options.MasterVolume;
        }

        // Zero means no ceiling, which is what Godot's own "unlimited" is.
        Engine.MaxFps = Mathf.Max(0, options.MaxFps);

        DisplayServer.WindowSetVsyncMode(options.VSync switch
        {
            0 => DisplayServer.VSyncMode.Disabled,
            2 => DisplayServer.VSyncMode.Adaptive,
            _ => DisplayServer.VSyncMode.Enabled,
        });

        // Only when it is actually changing. Setting the mode unconditionally churns the window on
        // every saved setting, and every panel in the game relays itself when the window resizes.
        var wanted = options.Windowed
            ? DisplayServer.WindowMode.Windowed
            : DisplayServer.WindowMode.Fullscreen;

        if (DisplayServer.WindowGetMode() != wanted)
            DisplayServer.WindowSetMode(wanted);

        _instance._settings.Save();
    }

    public static bool ContentLoaded => Assets != null && Data != null;

    public override void _EnterTree()
    {
        _instance = this;
        _clock.Reset();

        _settings = Settings.Load();

        // Before anything reads the input map, and exactly once: the defaults are taken out of the
        // map itself, so applying overrides any later would record an override as the default.
        KeyBindings.Apply(_settings);

        _audio = new Audio.AudioLibrary
        {
            EffectVolume = _settings.EffectVolume,
            MusicVolume = _settings.MusicVolume,
        };
        AddChild(_audio);

        // Keep ticking while the window is unfocused: the server's keepalive and acknowledgement
        // deadlines run on wall time, so a client that stops processing for twelve seconds is
        // disconnected.
        ProcessMode = ProcessModeEnum.Always;
    }

    public override void _ExitTree()
    {
        EndSession("Client shutting down.");
        if (_instance == this)
            _instance = null;
    }

    /// <summary>
    /// Loads the extracted sprites and game data. Slow enough to be worth doing once, at boot.
    /// </summary>
    public static void LoadContent()
    {
        if (ContentLoaded)
            return;

        Assets = AssetLibrary.Load();

        var data = new GameData();
        var manifest = Assets.Manifest;

        foreach (string file in manifest.Xml.Ground)
            LoadXml(file, data.AddGround);

        // Order matters: the first entries are base definitions and everything after them is a
        // per-dungeon overlay that replaces the types it mentions.
        foreach (string file in manifest.Xml.Objects)
            LoadXml(file, data.AddObjects);

        Data = data;
        GD.Print($"[content] {Assets.Sheets.Count} sheets, {Data.Objects.Count} objects, {Data.Ground.Count} terrain types.");
    }

    private static void LoadXml(string fileName, Action<string> merge)
    {
        string path = AssetManifest.XmlDirectory + fileName;
        using var file = FileAccess.Open(path, FileAccess.ModeFlags.Read);
        if (file == null)
        {
            GD.PushError($"[content] missing {path}; run tools/extract_assets.py.");
            return;
        }

        try
        {
            merge(file.GetAsText());
        }
        catch (Exception ex)
        {
            // One unreadable file should cost only the content it declares.
            GD.PushError($"[content] could not parse {fileName}: {ex.Message}");
        }
    }

    /// <summary>
    /// Fetches the language table from the app server.
    /// </summary>
    /// <remarks>
    /// Deliberately not fatal. The client is perfectly playable with keys showing through instead
    /// of prose, and refusing to start because a translation file is missing would be worse than
    /// the missing translations.
    /// </remarks>
    public static async System.Threading.Tasks.Task LoadLanguageAsync(string baseUrl, string language = "en")
    {
        try
        {
            using var client = new Account.AppEngineClient(baseUrl);
            string json = await client.PostAsync("/app/getLanguageStrings",
                new System.Collections.Generic.Dictionary<string, string> { ["languageType"] = language });

            Strings.LoadFrom(json);
            GD.Print($"[content] {Strings.Count} localised strings.");
        }
        catch (Exception ex)
        {
            GD.PushWarning($"[content] could not load language strings: {ex.Message}");
        }
    }

    /// <summary>Creates a fresh session, discarding any previous one.</summary>
    public static GameSession BeginSession()
    {
        EndSession("Starting a new session.");
        _instance._session = new GameSession(_instance._clock);
        return _instance._session;
    }

    /// <summary>Tears down the current session, if any.</summary>
    public static void EndSession(string reason = "Left the game.")
    {
        if (_instance?._session == null)
            return;

        _instance._session.Close(reason);
        _instance._session.Dispose();
        _instance._session = null;
    }

    public override void _Process(double delta)
    {
        // Taken before anything else reads it. The session is drained by the game scene instead, at
        // the point in its own update where the player's position is already current.
        _clock.BeginFrame();
    }
}

using Godot;

namespace Hendra.App;

/// <summary>
/// The handful of things a player can change, kept between sessions.
/// </summary>
/// <remarks>
/// <para>
/// A Godot <c>ConfigFile</c> under <c>user://</c>, which is a per-user directory the engine picks
/// for the platform. The original kept the same kind of thing in a Flash shared object and reached
/// it through a global <c>Parameters.data_</c> that a hundred files wrote to directly; here the
/// values are read once at startup and pushed to whatever needs them.
/// </para>
/// <para>
/// Saving is immediate rather than deferred. There are a few values, they change rarely, and a
/// setting that quietly fails to survive a crash is worse than a write.
/// </para>
/// </remarks>
public sealed class Settings
{
    private const string Path = "user://settings.cfg";
    private const string Section = "options";

    /// <summary>Sound effect volume, 0 to 1.</summary>
    public float EffectVolume { get; set; } = 0.6f;

    /// <summary>Music volume, 0 to 1.</summary>
    public float MusicVolume { get; set; } = 0.35f;

    /// <summary>
    /// Whether the camera keeps the player centred.
    /// </summary>
    /// <remarks>
    /// The original offered the alternative — the player sitting low on the screen, so more of the
    /// world ahead is visible — and players who use it feel strongly about it.
    /// </remarks>
    public bool CenterOnPlayer { get; set; } = true;

    public static Settings Load()
    {
        var settings = new Settings();
        var file = new ConfigFile();

        // A missing file is the ordinary first-run case, not a fault.
        if (file.Load(Path) != Error.Ok)
            return settings;

        settings.EffectVolume = (float)file.GetValue(Section, "effect_volume", settings.EffectVolume);
        settings.MusicVolume = (float)file.GetValue(Section, "music_volume", settings.MusicVolume);
        settings.CenterOnPlayer = (bool)file.GetValue(Section, "center_on_player", settings.CenterOnPlayer);
        return settings;
    }

    public void Save()
    {
        var file = new ConfigFile();
        file.SetValue(Section, "effect_volume", EffectVolume);
        file.SetValue(Section, "music_volume", MusicVolume);
        file.SetValue(Section, "center_on_player", CenterOnPlayer);

        var error = file.Save(Path);
        if (error != Error.Ok)
            GD.PushWarning($"[settings] could not save {Path}: {error}");
    }
}

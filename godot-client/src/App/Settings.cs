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

    /// <summary>
    /// The account last signed in with, and its password.
    /// </summary>
    /// <remarks>
    /// The original keeps the same three things -- account, password and token -- in a Flash
    /// shared object, with remembering switched on by default, which is why it never asks twice.
    /// The password is stored in the clear there and here; this is a game client's saved login, not
    /// a credential store, and hiding it behind an encoding that the same machine can trivially
    /// undo would only look safer than it is. Turning remembering off clears both.
    /// </remarks>
    public string Account { get; set; } = string.Empty;

    public string Password { get; set; } = string.Empty;

    /// <summary>Whether to keep the login. On by default, as in the original.</summary>
    public bool RememberMe { get; set; } = true;

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

    /// <summary>
    /// Which of the minimap's three zoom steps is showing, nearest first.
    /// </summary>
    /// <remarks>
    /// Kept because it is a preference rather than a state: a player who runs the map zoomed out
    /// wants it zoomed out in the next world too, not just for the rest of this one.
    /// </remarks>
    public int MinimapZoom { get; set; } = 1;

    /// <summary>Master volume, multiplied into both of the others.</summary>
    public float MasterVolume { get; set; } = 1f;

    /// <summary>Whether a weapon makes a noise when it is fired.</summary>
    public bool WeaponSounds { get; set; } = true;

    /// <summary>Whether Q and E turn the camera at all.</summary>
    public bool AllowCameraRotation { get; set; } = true;

    /// <summary>How fast the camera turns: 0 slow, 1 normal, 2 fast.</summary>
    public int CameraRotationSpeed { get; set; } = 1;

    /// <summary>The angle the camera resets to, in degrees. Zero or forty-five.</summary>
    public int DefaultCameraAngle { get; set; }

    /// <summary>Whose health bars are drawn. See <see cref="HealthBarMode"/>.</summary>
    public int HealthBars { get; set; } = (int)Hendra.App.HealthBarMode.All;

    /// <summary>Whether speech balloons are drawn over the people saying things.</summary>
    public bool TextBubbles { get; set; } = true;

    /// <summary>Whether damage numbers are thrown off the things taking it.</summary>
    public bool EnemyDamageText { get; set; } = true;

    public bool AllyDamageText { get; set; } = true;

    /// <summary>
    /// Whether experience is still announced at level 20, where it buys nothing.
    /// </summary>
    /// <remarks>
    /// The original's <c>forceEXP</c> (<c>Options.as:450</c>, defaulted to 0 at
    /// <c>Parameters.as:238</c>): 0 off, 1 for everyone, 2 for yourself only. Off, a level-20
    /// character says nothing about the experience it keeps earning — <c>Player.handleExpUp</c>
    /// returns before drawing a thing.
    /// </remarks>
    public int AlwaysShowExp { get; set; }

    /// <summary>The switch that turns off every particle at once.</summary>
    public bool Particles { get; set; } = true;

    /// <summary>Hit and death sprays off enemies, and off players, separately.</summary>
    public bool EnemyParticles { get; set; } = true;

    public bool PlayerHitParticles { get; set; } = true;

    /// <summary>Blast effects from grenades and other area attacks.</summary>
    public bool AoeParticles { get; set; } = true;

    /// <summary>Ground shadows: 0 off, 1 low, 2 high.</summary>
    public int Shadows { get; set; } = 2;

    /// <summary>Whether the tier of a piece of gear is written on its slot.</summary>
    public bool ShowTierLevel { get; set; } = true;

    /// <summary>Chat text size in pixels.</summary>
    public int ChatFontSize { get; set; } = 12;

    /// <summary>Whether the chat window is hidden outright.</summary>
    public bool HideChat { get; set; }

    /// <summary>
    /// The frame rate ceiling, or zero for none.
    /// </summary>
    /// <remarks>
    /// Worth having even on a machine that never struggles: an uncapped client renders as fast as
    /// it can whether or not anyone benefits, which on a laptop is fan noise and a flat battery for
    /// frames nobody sees. Zero leaves it to the display's own timing.
    /// </remarks>
    public int MaxFps { get; set; }

    /// <summary>0 off, 1 on, 2 adaptive.</summary>
    /// <remarks>
    /// Adaptive holds the refresh rate while the machine can keep up and lets it tear rather than
    /// halving the rate when it cannot, which is the better failure for a game that is sometimes
    /// showing four thousand projectiles.
    /// </remarks>
    public int VSync { get; set; } = 1;

    /// <summary>Windowed rather than fullscreen.</summary>
    public bool Windowed { get; set; }

    /// <summary>
    /// What fraction of native the world is rendered at, as a percentage.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Above a hundred is supersampling: the world is drawn larger than the window and scaled back
    /// down, so every sprite edge is averaged from several samples instead of landing on one pixel
    /// or the next. It is the one setting that meaningfully changes how this game looks. The art is
    /// eight pixels square drawn forty pixels tall, so every diagonal in it is a staircase, and no
    /// amount of filtering fixes a staircase -- more samples do.
    /// </para>
    /// <para>
    /// Only the world. The interface is drawn on its own canvas and is never scaled by this, so a
    /// hundred and fifty per cent costs nothing in text sharpness.
    /// </para>
    /// </remarks>
    public int RenderScale { get; set; } = 150;

    /// <summary>
    /// How large the interface is drawn, as a percentage, or zero to fit it to the window.
    /// </summary>
    /// <remarks>
    /// Fitting is the default and it is right on most screens, but it is a calculation about
    /// proportions and not about eyesight: on a large panel at a normal viewing distance it lands
    /// on something correct and small. This is the override.
    /// </remarks>
    public int HudScale { get; set; }

    /// <summary>0 off, 1 FXAA, 2 MSAA 2x, 3 MSAA 4x, 4 MSAA 8x.</summary>
    /// <remarks>
    /// Two different tools under one control. FXAA smooths the finished image and so reaches the
    /// alpha-cut edges of sprites, which is most of what is jagged here; MSAA works on geometry
    /// edges and so mainly helps the ground and the walls. Both, at once, is what the top setting
    /// is for.
    /// </remarks>
    public int AntiAliasing { get; set; } = 1;

    /// <summary>Keys the player has moved, as action name to keycode. Empty means all default.</summary>
    public System.Collections.Generic.Dictionary<string, int> KeyOverrides { get; }
        = new System.Collections.Generic.Dictionary<string, int>();

    // ---- Gameplay ----

    /// <summary>Whether the minimap turns with the camera. Off is the original's default.</summary>
    public bool MinimapRotation { get; set; }

    /// <summary>Hold the modifier and click to move an item between inventory and backpack.</summary>
    public bool SwapWithBackpack { get; set; } = true;

    // ---- Opacity. Applied to other people so your own character stays readable in a crowd. ----

    public float Opacity { get; set; } = 1f;
    public bool PlayerOnTop { get; set; } = true;
    public bool FadeGuildMembers { get; set; }
    public bool FadePlayers { get; set; } = true;
    public bool FadeProjectiles { get; set; } = true;

    // ---- Accessibility ----

    /// <summary>Health bars that run green to orange to red as they empty.</summary>
    public bool DynamicHpGui { get; set; } = true;

    public bool DynamicHpPlayer { get; set; } = true;
    public bool DynamicHpBoss { get; set; } = true;

    /// <summary>Condition icons at half size.</summary>
    public bool SmallConditionIcons { get; set; }

    // ---- Social ----

    /// <summary>Chat kinds, each shown or hidden on its own.</summary>
    public bool PlayerChat { get; set; } = true;

    public bool WhisperChat { get; set; } = true;
    public bool GuildChatShown { get; set; } = true;
    public bool ShowPlayerTitles { get; set; } = true;

    // ---- Interface ----

    public bool ShowAllyBuffIcons { get; set; } = true;
    public bool ShowBossHpBars { get; set; } = true;
    public bool ExpandLog { get; set; } = true;
    public bool ShowFameGain { get; set; }

    /// <summary>Always write the numbers on the vitals bars: 0 off, 1 fame, 2 HP/MP, 3 both.</summary>
    public int BarText { get; set; }

    // ---- Quality ----

    /// <summary>How many particles a burst throws: 0 low, 1 medium, 2 high.</summary>
    public int ParticleDetail { get; set; } = 2;

    /// <summary>Notifications floating over allies.</summary>
    public bool AllyNotifications { get; set; } = true;

    /// <summary>Cursed enemies drawn with a red wash, so they can be picked out of a fight.</summary>
    public bool CurseIndication { get; set; }

    /// <summary>Other people's shots: 0 all, 1 hide their projectiles, 2 hide those and the flash.</summary>
    public int AllyShoot { get; set; }

    /// <summary>
    /// How close the camera sits, as a multiplier. Larger shows less ground, magnified.
    /// </summary>
    /// <remarks>
    /// The original has no such control -- its view is a fixed number of tiles -- so the range here
    /// is chosen rather than copied: half again out, to twice in. Far enough out to see a boss
    /// arena, not so far that the world turns to soup.
    /// </remarks>
    public float CameraZoom { get; set; } = 1f;

    /// <summary>How large loot bags are drawn, as a multiplier on their own size.</summary>
    /// <remarks>
    /// A bag in a heap of other bags after a fight is the one thing on screen a player has to hit
    /// with a mouse, and the artwork is eight pixels across.
    /// </remarks>
    public float BagSize { get; set; } = 1f;

    /// <summary>Which of the two carried pages the hotbar is showing.</summary>
    public int HotbarPage { get; set; }

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
        settings.MinimapZoom = (int)file.GetValue(Section, "minimap_zoom", settings.MinimapZoom);
        settings.HotbarPage = (int)file.GetValue(Section, "hotbar_page", settings.HotbarPage);
        settings.MasterVolume = (float)file.GetValue(Section, "master_volume", settings.MasterVolume);
        settings.WeaponSounds = (bool)file.GetValue(Section, "weapon_sounds", settings.WeaponSounds);
        settings.AllowCameraRotation = (bool)file.GetValue(Section, "camera_rotation", settings.AllowCameraRotation);
        settings.CameraRotationSpeed = (int)file.GetValue(Section, "camera_rotation_speed", settings.CameraRotationSpeed);
        settings.DefaultCameraAngle = (int)file.GetValue(Section, "camera_angle", settings.DefaultCameraAngle);
        settings.HealthBars = (int)file.GetValue(Section, "health_bars", settings.HealthBars);
        settings.TextBubbles = (bool)file.GetValue(Section, "text_bubbles", settings.TextBubbles);
        settings.EnemyDamageText = (bool)file.GetValue(Section, "enemy_damage_text", settings.EnemyDamageText);
        settings.AllyDamageText = (bool)file.GetValue(Section, "ally_damage_text", settings.AllyDamageText);
        settings.AlwaysShowExp = (int)file.GetValue(Section, "always_show_exp", settings.AlwaysShowExp);
        settings.Particles = (bool)file.GetValue(Section, "particles", settings.Particles);
        settings.EnemyParticles = (bool)file.GetValue(Section, "enemy_particles", settings.EnemyParticles);
        settings.PlayerHitParticles = (bool)file.GetValue(Section, "player_hit_particles", settings.PlayerHitParticles);
        settings.AoeParticles = (bool)file.GetValue(Section, "aoe_particles", settings.AoeParticles);
        settings.Shadows = (int)file.GetValue(Section, "shadows", settings.Shadows);
        settings.ShowTierLevel = (bool)file.GetValue(Section, "show_tier_level", settings.ShowTierLevel);
        settings.ChatFontSize = (int)file.GetValue(Section, "chat_font_size", settings.ChatFontSize);
        settings.HideChat = (bool)file.GetValue(Section, "hide_chat", settings.HideChat);
        settings.Windowed = (bool)file.GetValue(Section, "windowed", settings.Windowed);
        settings.MaxFps = (int)file.GetValue(Section, "max_fps", settings.MaxFps);
        settings.VSync = (int)file.GetValue(Section, "vsync", settings.VSync);
        settings.RenderScale = (int)file.GetValue(Section, "render_scale", settings.RenderScale);
        settings.HudScale = (int)file.GetValue(Section, "hud_scale", settings.HudScale);
        settings.AntiAliasing = (int)file.GetValue(Section, "anti_aliasing", settings.AntiAliasing);
        settings.CameraZoom = (float)file.GetValue(Section, "camera_zoom", settings.CameraZoom);
        settings.BagSize = (float)file.GetValue(Section, "bag_size", settings.BagSize);
        settings.MinimapRotation = (bool)file.GetValue(Section, "minimap_rotation", settings.MinimapRotation);
        settings.SwapWithBackpack = (bool)file.GetValue(Section, "swap_with_backpack", settings.SwapWithBackpack);
        settings.Opacity = (float)file.GetValue(Section, "opacity", settings.Opacity);
        settings.PlayerOnTop = (bool)file.GetValue(Section, "player_on_top", settings.PlayerOnTop);
        settings.FadeGuildMembers = (bool)file.GetValue(Section, "fade_guild", settings.FadeGuildMembers);
        settings.FadePlayers = (bool)file.GetValue(Section, "fade_players", settings.FadePlayers);
        settings.FadeProjectiles = (bool)file.GetValue(Section, "fade_projectiles", settings.FadeProjectiles);
        settings.DynamicHpGui = (bool)file.GetValue(Section, "dynamic_hp_gui", settings.DynamicHpGui);
        settings.DynamicHpPlayer = (bool)file.GetValue(Section, "dynamic_hp_player", settings.DynamicHpPlayer);
        settings.DynamicHpBoss = (bool)file.GetValue(Section, "dynamic_hp_boss", settings.DynamicHpBoss);
        settings.SmallConditionIcons = (bool)file.GetValue(Section, "small_condition_icons", settings.SmallConditionIcons);
        settings.PlayerChat = (bool)file.GetValue(Section, "player_chat", settings.PlayerChat);
        settings.WhisperChat = (bool)file.GetValue(Section, "whisper_chat", settings.WhisperChat);
        settings.GuildChatShown = (bool)file.GetValue(Section, "guild_chat_shown", settings.GuildChatShown);
        settings.ShowPlayerTitles = (bool)file.GetValue(Section, "show_player_titles", settings.ShowPlayerTitles);
        settings.ShowAllyBuffIcons = (bool)file.GetValue(Section, "ally_buff_icons", settings.ShowAllyBuffIcons);
        settings.ShowBossHpBars = (bool)file.GetValue(Section, "boss_hp_bars", settings.ShowBossHpBars);
        settings.ExpandLog = (bool)file.GetValue(Section, "expand_log", settings.ExpandLog);
        settings.ShowFameGain = (bool)file.GetValue(Section, "show_fame_gain", settings.ShowFameGain);
        settings.BarText = (int)file.GetValue(Section, "bar_text", settings.BarText);
        settings.ParticleDetail = (int)file.GetValue(Section, "particle_detail", settings.ParticleDetail);
        settings.AllyNotifications = (bool)file.GetValue(Section, "ally_notifications", settings.AllyNotifications);
        settings.CurseIndication = (bool)file.GetValue(Section, "curse_indication", settings.CurseIndication);
        settings.AllyShoot = (int)file.GetValue(Section, "ally_shoot", settings.AllyShoot);

        foreach (string action in (string[])file.GetValue(Section, "key_actions", System.Array.Empty<string>()))
        {
            int key = (int)file.GetValue(Section, "key_" + action, 0);
            settings.KeyOverrides[action] = key;
        }

        settings.RememberMe = (bool)file.GetValue(Section, "remember_me", settings.RememberMe);
        settings.Account = (string)file.GetValue(Section, "account", settings.Account);
        settings.Password = (string)file.GetValue(Section, "password", settings.Password);
        return settings;
    }

    public void Save()
    {
        var file = new ConfigFile();
        file.SetValue(Section, "effect_volume", EffectVolume);
        file.SetValue(Section, "music_volume", MusicVolume);
        file.SetValue(Section, "center_on_player", CenterOnPlayer);
        file.SetValue(Section, "minimap_zoom", MinimapZoom);
        file.SetValue(Section, "hotbar_page", HotbarPage);
        file.SetValue(Section, "master_volume", MasterVolume);
        file.SetValue(Section, "weapon_sounds", WeaponSounds);
        file.SetValue(Section, "camera_rotation", AllowCameraRotation);
        file.SetValue(Section, "camera_rotation_speed", CameraRotationSpeed);
        file.SetValue(Section, "camera_angle", DefaultCameraAngle);
        file.SetValue(Section, "health_bars", HealthBars);
        file.SetValue(Section, "text_bubbles", TextBubbles);
        file.SetValue(Section, "enemy_damage_text", EnemyDamageText);
        file.SetValue(Section, "ally_damage_text", AllyDamageText);
        file.SetValue(Section, "always_show_exp", AlwaysShowExp);
        file.SetValue(Section, "particles", Particles);
        file.SetValue(Section, "enemy_particles", EnemyParticles);
        file.SetValue(Section, "player_hit_particles", PlayerHitParticles);
        file.SetValue(Section, "aoe_particles", AoeParticles);
        file.SetValue(Section, "shadows", Shadows);
        file.SetValue(Section, "show_tier_level", ShowTierLevel);
        file.SetValue(Section, "chat_font_size", ChatFontSize);
        file.SetValue(Section, "hide_chat", HideChat);
        file.SetValue(Section, "windowed", Windowed);
        file.SetValue(Section, "max_fps", MaxFps);
        file.SetValue(Section, "vsync", VSync);
        file.SetValue(Section, "render_scale", RenderScale);
        file.SetValue(Section, "hud_scale", HudScale);
        file.SetValue(Section, "anti_aliasing", AntiAliasing);
        file.SetValue(Section, "camera_zoom", CameraZoom);
        file.SetValue(Section, "bag_size", BagSize);
        file.SetValue(Section, "minimap_rotation", MinimapRotation);
        file.SetValue(Section, "swap_with_backpack", SwapWithBackpack);
        file.SetValue(Section, "opacity", Opacity);
        file.SetValue(Section, "player_on_top", PlayerOnTop);
        file.SetValue(Section, "fade_guild", FadeGuildMembers);
        file.SetValue(Section, "fade_players", FadePlayers);
        file.SetValue(Section, "fade_projectiles", FadeProjectiles);
        file.SetValue(Section, "dynamic_hp_gui", DynamicHpGui);
        file.SetValue(Section, "dynamic_hp_player", DynamicHpPlayer);
        file.SetValue(Section, "dynamic_hp_boss", DynamicHpBoss);
        file.SetValue(Section, "small_condition_icons", SmallConditionIcons);
        file.SetValue(Section, "player_chat", PlayerChat);
        file.SetValue(Section, "whisper_chat", WhisperChat);
        file.SetValue(Section, "guild_chat_shown", GuildChatShown);
        file.SetValue(Section, "show_player_titles", ShowPlayerTitles);
        file.SetValue(Section, "ally_buff_icons", ShowAllyBuffIcons);
        file.SetValue(Section, "boss_hp_bars", ShowBossHpBars);
        file.SetValue(Section, "expand_log", ExpandLog);
        file.SetValue(Section, "show_fame_gain", ShowFameGain);
        file.SetValue(Section, "bar_text", BarText);
        file.SetValue(Section, "particle_detail", ParticleDetail);
        file.SetValue(Section, "ally_notifications", AllyNotifications);
        file.SetValue(Section, "curse_indication", CurseIndication);
        file.SetValue(Section, "ally_shoot", AllyShoot);

        // The overrides are written as a list of action names plus one entry each, because a
        // ConfigFile value is a Variant and a dictionary of them does not survive the round trip
        // as anything a later version could still read.
        var actions = new string[KeyOverrides.Count];
        KeyOverrides.Keys.CopyTo(actions, 0);
        file.SetValue(Section, "key_actions", actions);

        foreach (var (action, key) in KeyOverrides)
            file.SetValue(Section, "key_" + action, key);

        file.SetValue(Section, "remember_me", RememberMe);
        file.SetValue(Section, "account", RememberMe ? Account : string.Empty);
        file.SetValue(Section, "password", RememberMe ? Password : string.Empty);

        var error = file.Save(Path);
        if (error != Error.Ok)
            GD.PushWarning($"[settings] could not save {Path}: {error}");
    }
}

/// <summary>Whose health bars are drawn over the world.</summary>
public enum HealthBarMode
{
    Off = 0,
    Enemies = 1,
    Allies = 2,
    All = 3,
}

using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Godot;

namespace Hendra.Audio;

/// <summary>
/// Sound effects and music, fetched from the app server as they are first needed.
/// </summary>
/// <remarks>
/// <para>
/// The sounds are not shipped with the client. They live on the app server under <c>/sfx</c> and
/// <c>/music</c> and are served as ordinary static files, which is where the original fetched them
/// from too — it built a URL per name and handed it to Flash's <c>Sound.load</c>.
/// </para>
/// <para>
/// Fetching is asynchronous and the first play of a given sound is therefore silent. That is the
/// original's behaviour as well, and the alternative — blocking a frame on an HTTP round trip the
/// first time anything is shot — is worse. Everything the interface uses is warmed at startup, so
/// in practice only the first monster of a new kind loses its hit sound.
/// </para>
/// <para>
/// A name that does not exist on the server is remembered as missing and never asked for again.
/// The game data names sounds for objects that have none, so without that this would re-request the
/// same missing file on every hit.
/// </para>
/// </remarks>
public partial class AudioLibrary : Node
{
    /// <summary>How many effects can overlap. Beyond this the oldest is cut off.</summary>
    private const int Voices = 16;

    /// <summary>Sounds the interface uses, fetched up front so they are ready the first time.</summary>
    private static readonly string[] Common =
    {
        "button_click", "error", "no_mana", "use_potion", "level_up",
        "loot_appears", "inventory_move_item", "death_screen", "use_key",
    };

    private readonly Dictionary<string, AudioStream> _cache = new();
    private readonly HashSet<string> _missing = new();
    private readonly HashSet<string> _inFlight = new();

    private readonly List<AudioStreamPlayer> _effectPlayers = new();
    private AudioStreamPlayer _musicPlayer;
    private int _nextVoice;

    private System.Net.Http.HttpClient _http;
    private string _baseUrl = string.Empty;
    private string _currentMusic = string.Empty;

    /// <summary>Effect volume, 0 to 1.</summary>
    public float EffectVolume { get; set; } = 0.6f;

    /// <summary>Music volume, 0 to 1.</summary>
    public float MusicVolume { get; set; } = 0.35f;

    public override void _Ready()
    {
        // Sound should keep playing while the window is unfocused, like the rest of the client.
        ProcessMode = ProcessModeEnum.Always;

        for (int i = 0; i < Voices; i++)
        {
            var player = new AudioStreamPlayer();
            AddChild(player);
            _effectPlayers.Add(player);
        }

        _musicPlayer = new AudioStreamPlayer();
        _musicPlayer.Finished += RestartMusic;
        AddChild(_musicPlayer);
    }

    public override void _ExitTree() => _http?.Dispose();

    /// <summary>
    /// Points the library at an app server. Safe to call again when the server changes.
    /// </summary>
    /// <param name="baseUrl">e.g. <c>http://host:8888</c>.</param>
    public void Configure(string baseUrl)
    {
        if (_baseUrl == baseUrl)
            return;

        _baseUrl = baseUrl ?? string.Empty;

        // Cached streams belong to the server they came from, and a different server may well have
        // different sounds under the same names.
        _cache.Clear();
        _missing.Clear();

        _http ??= new System.Net.Http.HttpClient { Timeout = TimeSpan.FromSeconds(15) };

        foreach (string name in Common)
            _ = FetchAsync("sfx", name);
    }

    /// <summary>
    /// Plays a sound effect by name, e.g. <c>level_up</c> or <c>monster/undead_hit</c>.
    /// </summary>
    /// <param name="volume">Multiplied into the effect volume, matching the original's argument.</param>
    public void PlayEffect(string name, float volume = 1f)
    {
        if (string.IsNullOrEmpty(name) || EffectVolume <= 0f)
            return;

        var stream = Get("sfx", name);
        if (stream == null)
            return;

        // Round robin rather than looking for an idle player: a sound that has to interrupt another
        // is better than one that does not play at all, and cutting off the oldest is least
        // noticeable.
        var player = _effectPlayers[_nextVoice];
        _nextVoice = (_nextVoice + 1) % _effectPlayers.Count;

        player.Stream = stream;
        player.VolumeDb = ToDecibels(EffectVolume * volume);
        player.Play();
    }

    /// <summary>
    /// Switches the background music. Re-asking for the track already playing does nothing.
    /// </summary>
    public void PlayMusic(string name)
    {
        if (_currentMusic == name)
            return;

        _currentMusic = name ?? string.Empty;

        if (string.IsNullOrEmpty(_currentMusic) || MusicVolume <= 0f)
        {
            _musicPlayer.Stop();
            return;
        }

        var stream = Get("music", _currentMusic);
        if (stream == null)
        {
            // Not here yet. The fetch is under way; RestartMusic picks it up once it lands.
            _musicPlayer.Stop();
            return;
        }

        StartMusic(stream);
    }

    private void StartMusic(AudioStream stream)
    {
        _musicPlayer.Stream = stream;
        _musicPlayer.VolumeDb = ToDecibels(MusicVolume);
        _musicPlayer.Play();
    }

    /// <summary>
    /// Loops the current track, and starts it if it only just finished downloading.
    /// </summary>
    /// <remarks>
    /// Looping by restarting on the finished signal rather than by setting the stream's loop flag,
    /// because that flag lives on the concrete stream type and would have to be set per format.
    /// </remarks>
    private void RestartMusic()
    {
        if (string.IsNullOrEmpty(_currentMusic))
            return;

        var stream = Get("music", _currentMusic);
        if (stream != null)
            StartMusic(stream);
    }

    /// <summary>
    /// The stream for a name, or null while it is still on its way.
    /// </summary>
    private AudioStream Get(string folder, string name)
    {
        if (_cache.TryGetValue(name, out var cached))
            return cached;

        if (!_missing.Contains(name))
            _ = FetchAsync(folder, name);

        return null;
    }

    private async Task FetchAsync(string folder, string name)
    {
        if (_http == null || string.IsNullOrEmpty(_baseUrl))
            return;

        // One request per name in flight. Sounds are asked for many times a second while the fetch
        // is outstanding -- every shot, every hit.
        lock (_inFlight)
        {
            if (!_inFlight.Add(name))
                return;
        }

        try
        {
            var bytes = await _http.GetByteArrayAsync($"{_baseUrl}/{folder}/{name}.mp3");
            CallDeferred(nameof(Store), name, bytes);
        }
        catch (Exception)
        {
            // Missing sounds are ordinary: the game data names them for objects the server has no
            // audio for. Recorded so it is only asked for once.
            CallDeferred(nameof(MarkMissing), name);
        }
        finally
        {
            lock (_inFlight)
                _inFlight.Remove(name);
        }
    }

    private void Store(string name, byte[] bytes)
    {
        _cache[name] = new AudioStreamMP3 { Data = bytes };
        ReportWarmup(name);

        // The music may have been asked for before it arrived.
        if (name == _currentMusic && !_musicPlayer.Playing)
            RestartMusic();
    }

    private void MarkMissing(string name)
    {
        _missing.Add(name);
        ReportWarmup(name);
    }

    /// <summary>
    /// Says once how much of the common set arrived.
    /// </summary>
    /// <remarks>
    /// Worth a line, in the same spirit as the content counts at startup: silence is otherwise
    /// indistinguishable from a client that never reached the app server.
    /// </remarks>
    private void ReportWarmup(string name)
    {
        if (_reportedWarmup || Array.IndexOf(Common, name) < 0)
            return;

        int loaded = 0, resolved = 0;
        foreach (string common in Common)
        {
            if (_cache.ContainsKey(common)) { loaded++; resolved++; }
            else if (_missing.Contains(common)) resolved++;
        }

        if (resolved < Common.Length)
            return;

        _reportedWarmup = true;
        GD.Print($"[audio] {loaded} of {Common.Length} interface sounds loaded from {_baseUrl}.");
    }

    private bool _reportedWarmup;

    /// <summary>
    /// A linear volume as decibels, which is what the mixer wants.
    /// </summary>
    /// <remarks>
    /// Silence is negative infinity in decibels, so anything at or below zero is clamped to a level
    /// far enough down to be inaudible rather than passed through the logarithm.
    /// </remarks>
    private static float ToDecibels(float linear) =>
        linear <= 0.0005f ? -80f : Mathf.LinearToDb(Mathf.Clamp(linear, 0f, 1f));
}

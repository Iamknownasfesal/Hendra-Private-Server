using System;
using System.Collections.Generic;
using System.Globalization;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The readout behind the debug key: what the machine and the connection are doing.
/// </summary>
/// <remarks>
/// <para>
/// Off by default, and drawn under the player card rather than over it. It exists to answer one
/// question -- "is it me, the machine, or the line?" -- so it puts the connection at the top and the
/// hardware under it, rather than burying the one number a player cares about in a wall of counters.
/// </para>
/// <para>
/// Every figure here is measured rather than estimated, with one exception that says so on the
/// line: see <c>GameSession.PingDelayMs</c> for why this protocol cannot show an absolute ping.
/// </para>
/// </remarks>
public partial class DebugOverlay : Control
{
    private const float Width = 300f;
    private const float LineHeight = 16f;
    private const float Pad = 10f;

    /// <summary>Frames sampled for the average, which is about a second at sixty.</summary>
    private const int Window = 60;

    private readonly float[] _frames = new float[Window];
    private readonly List<(string Label, string Value)> _lines = new(20);

    private int _frameAt;
    private int _frameCount;

    private string _gpu;
    private string _cpu;

    /// <summary>Where the numbers come from. Set by the world once a session exists.</summary>
    public Func<Net.GameSession> Session { get; set; }

    public Func<int> EntityCount { get; set; }

    public Func<int> ProjectileCount { get; set; }

    public Func<int> SpriteSurfaces { get; set; }

    public Func<string> WorldName { get; set; }

    public Func<(float X, float Y)> PlayerAt { get; set; }

    /// <summary>Where the frame goes, filled in by the world while this is showing.</summary>
    public World.FramePhases Phases { get; set; }

    private readonly List<(string Name, double Ms)> _costs = new();

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        Visible = false;
        ZIndex = 100;

        // Asked for once. Both of these query the driver, which is not something to do per frame.
        _gpu = RenderingServer.GetVideoAdapterName();
        _cpu = OS.GetProcessorName();
    }

    public void Toggle()
    {
        Visible = !Visible;

        // Timing is only collected while it is being looked at, so the scopes cost a branch the
        // rest of the time.
        if (Phases != null)
            Phases.Enabled = Visible;

        QueueRedraw();
    }

    public override void _Process(double delta)
    {
        if (!Visible)
            return;

        _frames[_frameAt] = (float)delta * 1000f;
        _frameAt = (_frameAt + 1) % Window;
        _frameCount = Math.Min(_frameCount + 1, Window);

        QueueRedraw();
    }

    /// <summary>The worst frame in the sample, which is what a stutter actually feels like.</summary>
    private float WorstFrameMs()
    {
        float worst = 0f;
        for (int i = 0; i < _frameCount; i++)
            worst = Mathf.Max(worst, _frames[i]);
        return worst;
    }

    private float AverageFrameMs()
    {
        if (_frameCount == 0)
            return 0f;

        float total = 0f;
        for (int i = 0; i < _frameCount; i++)
            total += _frames[i];

        return total / _frameCount;
    }

    private static string Megabytes(ulong bytes) =>
        (bytes / 1024f / 1024f).ToString("0.0", CultureInfo.InvariantCulture) + " MB";

    private void Collect()
    {
        _lines.Clear();

        var session = Session?.Invoke();

        _lines.Add(("Connection", null));

        if (session == null)
        {
            _lines.Add(("state", "not connected"));
        }
        else
        {
            // Named for what it is. This protocol gives a client nothing to time a round trip
            // against, so what is shown is delay above the best this session has seen.
            _lines.Add(("delay over best", $"{session.PingDelayMs} ms"));
            _lines.Add(("last packet", $"{session.SinceLastPacketMs} ms ago"));
            _lines.Add(("state", session.State.ToString()));
        }

        if (WorldName?.Invoke() is { Length: > 0 } world)
            _lines.Add(("world", world));

        if (PlayerAt != null)
        {
            var (x, y) = PlayerAt.Invoke();
            _lines.Add(("position", $"{x:0.0}, {y:0.0}"));
        }

        _lines.Add(("Frame", null));
        _lines.Add(("fps", Engine.GetFramesPerSecond().ToString("0", CultureInfo.InvariantCulture)));
        _lines.Add(("frame time", $"{AverageFrameMs():0.0} ms avg, {WorstFrameMs():0.0} worst"));
        _lines.Add(("physics", $"{Engine.PhysicsTicksPerSecond} Hz"));

        _lines.Add(("Scene", null));

        if (EntityCount != null)
            _lines.Add(("entities", EntityCount.Invoke().ToString(CultureInfo.InvariantCulture)));

        if (ProjectileCount != null)
            _lines.Add(("projectiles", ProjectileCount.Invoke().ToString(CultureInfo.InvariantCulture)));

        if (SpriteSurfaces != null)
            _lines.Add(("sprite surfaces", SpriteSurfaces.Invoke().ToString(CultureInfo.InvariantCulture)));

        _lines.Add(("draw calls", RenderingServer
            .GetRenderingInfo(RenderingServer.RenderingInfo.TotalDrawCallsInFrame)
            .ToString(CultureInfo.InvariantCulture)));

        if (Phases != null)
        {
            Phases.Collect(_costs);
            if (_costs.Count > 0)
            {
                _lines.Add(("Frame cost", null));

                // Heaviest first: the question this answers is "what do I fix", and that is
                // whatever is at the top.
                _costs.Sort(static (a, b) => b.Ms.CompareTo(a.Ms));

                foreach (var (name, ms) in _costs)
                    _lines.Add((name, $"{ms:0.00} ms"));
            }
        }

        _lines.Add(("Machine", null));
        _lines.Add(("memory", Megabytes(OS.GetStaticMemoryUsage())));
        _lines.Add(("video memory", Megabytes(
            RenderingServer.GetRenderingInfo(RenderingServer.RenderingInfo.VideoMemUsed))));
        _lines.Add(("gpu", _gpu));
        _lines.Add(("cpu", _cpu));
    }

    public override void _Draw()
    {
        if (!Visible)
            return;

        Collect();

        float height = Pad * 2f;
        foreach (var (_, value) in _lines)
            height += value == null ? LineHeight + 4f : LineHeight;

        // Under the player card rather than over it: the card is the one thing on screen a player
        // is always reading, and a readout that covers it answers one question by hiding another.
        var layout = new HudLayout(Size.X > 0f && Size.Y > 0f
            ? Size
            : new Vector2(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight));

        var card = layout.PlayerCard;
        var box = new Rect2(card.Position.X, card.End.Y + 8f, Width, height);

        DrawRect(box, new Color(0f, 0f, 0f, 0.78f));
        DrawRect(box, Style.PanelEdge, filled: false, width: 1f);

        float y = box.Position.Y + Pad + LineHeight - 4f;

        foreach (var (label, value) in _lines)
        {
            if (value == null)
            {
                // A heading, set apart by the gap above it rather than by a rule.
                y += 4f;
                this.DrawOutlined(new Vector2(box.Position.X + Pad, y), label, Style.FontSmall, Style.FameFill);
                y += LineHeight;
                continue;
            }

            this.DrawOutlined(new Vector2(box.Position.X + Pad, y), label, Style.FontSmall, Style.TextDim);

            // Values right-aligned, so a changing number does not shuffle the column.
            this.DrawOutlined(
                new Vector2(box.End.X - Pad - Style.Measure(value, Style.FontSmall), y),
                value, Style.FontSmall, Style.Text);

            y += LineHeight;
        }
    }
}

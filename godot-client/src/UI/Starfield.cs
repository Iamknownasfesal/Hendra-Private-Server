using Godot;

namespace Hendra.UI;

/// <summary>
/// The drifting backdrop behind the menus.
/// </summary>
/// <remarks>
/// <para>
/// The original's title screen has a live map running behind it — a real world, streamed from a
/// real server, so the game is already moving before you have signed in. That is a whole second
/// world simulation for a screen you look at for four seconds, which is why the port does not do
/// it. But the instinct behind it is right: a still menu reads as a screenshot, and the first thing
/// a player sees should be alive.
/// </para>
/// <para>
/// So: motes drifting on a slow current, in three layers at different speeds, with the far ones
/// dimmer and slower. Parallax on a plane nobody is looking at directly does most of the work of
/// depth. It costs one polygon per mote and no allocations after the first frame.
/// </para>
/// </remarks>
public partial class Starfield : Control
{
    private const int Motes = 150;

    /// <summary>Seeded rather than random, so the field is the same one every time.</summary>
    private const ulong Seed = 0x5EED_1234;

    private struct Mote
    {
        public Vector2 At;
        public float Depth;
        public float Size;
        public float Twinkle;
        public float Phase;
    }

    private readonly Mote[] _motes = new Mote[Motes];
    private double _elapsed;
    private Vector2 _field;

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        SetAnchorsPreset(LayoutPreset.FullRect);
        Scatter();
    }

    public override void _Notification(int what)
    {
        if (what == NotificationResized)
            Scatter();
    }

    /// <summary>Lays the motes out across the current size.</summary>
    private void Scatter()
    {
        var size = Size;
        if (size.X <= 0f || size.Y <= 0f)
            return;

        // Only rescatter when the field has actually changed shape, so a resize does not reshuffle
        // the sky on every frame of a drag.
        if (size.IsEqualApprox(_field))
            return;

        _field = size;

        var random = new RandomNumberGenerator { Seed = Seed };
        for (int i = 0; i < _motes.Length; i++)
        {
            _motes[i] = new Mote
            {
                At = new Vector2(random.Randf() * size.X, random.Randf() * size.Y),

                // Cubed, so most motes are far away and a few are close. An even spread reads as
                // noise rather than as distance.
                Depth = Mathf.Pow(random.Randf(), 3f),
                Size = 0.8f + random.Randf() * 2.2f,
                Twinkle = 0.5f + random.Randf() * 1.5f,
                Phase = random.Randf() * Mathf.Tau,
            };
        }
    }

    public override void _Process(double delta)
    {
        _elapsed += delta;
        QueueRedraw();
    }

    public override void _Draw()
    {
        var size = Size;
        if (size.X <= 0f || size.Y <= 0f)
            return;

        float time = (float)_elapsed;

        foreach (var mote in _motes)
        {
            // Near motes move several times faster than far ones, which is the whole of the effect.
            float speed = 4f + mote.Depth * 26f;
            float x = Mathf.PosMod(mote.At.X - time * speed, size.X);
            float y = mote.At.Y + Mathf.Sin(time * 0.15f + mote.Phase) * 6f;

            float brightness = (0.10f + mote.Depth * 0.5f) *
                               (0.7f + 0.3f * Mathf.Sin(time * mote.Twinkle + mote.Phase));

            var colour = Style.Steel.Lerp(Colors.White, mote.Depth) with { A = brightness };
            DrawCircle(new Vector2(x, y), mote.Size * (0.5f + mote.Depth), colour);
        }
    }
}

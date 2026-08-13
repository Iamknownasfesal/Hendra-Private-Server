using System;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The layer the interface is drawn on, scaled so its children can be written in reference pixels.
/// </summary>
/// <remarks>
/// <para>
/// The HUD is measured at 1920 by 1080 and every number in <see cref="HudLayout"/> is taken at that
/// size. The rest of the client is not: the title screen, the login form and the panels were laid
/// out against the project's own 1280 by 720 base and are scaled by Godot's stretch, which is the
/// behaviour they should keep. Rather than move the whole client onto a new base and shrink every
/// screen that is not part of this brief, the HUD gets its own canvas with its own scale.
/// </para>
/// <para>
/// The arithmetic is one step: the layer's scale is the factor the HUD wants divided by the factor
/// the root already applies, and its children are sized so that what they cover, once scaled,
/// is exactly the window. So a child asked for its own rectangle is answered in reference pixels
/// and can be positioned straight out of the layout.
/// </para>
/// </remarks>
public partial class HudLayer : CanvasLayer
{
    /// <summary>The rectangle the interface has to lay itself out in, in reference pixels.</summary>
    public Vector2 Space { get; private set; } = new(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight);

    /// <summary>What one reference pixel is worth on the screen. The brief's <c>--hud-scale</c>.</summary>
    public float HudScale { get; private set; } = 1f;

    /// <summary>Raised after <see cref="Space"/> changes, so clusters can re-pin themselves.</summary>
    public event Action Reflowed;

    public override void _Ready()
    {
        GetViewport().SizeChanged += Refit;

        // A child added after this point is sized when it arrives rather than staying at whatever
        // size it was constructed with -- which, for a Control, is zero.
        ChildEnteredTree += _ => CallDeferred(nameof(Refit));

        Refit();
    }

    /// <summary>Recomputes the scale and re-sizes every child to the space it leaves.</summary>
    public void Refit()
    {
        var window = (Vector2)GetWindow().Size;
        var logical = GetViewport().GetVisibleRect().Size;
        if (window.X <= 0f || window.Y <= 0f || logical.X <= 0f || logical.Y <= 0f)
            return;

        // The player's own figure wins over the fitted one, but never past what the window can
        // actually hold: an interface asked to be twice as large as the screen is not an interface.
        int chosen = App.ServiceLocator.Settings?.HudScale ?? 0;

        HudScale = chosen > 0
            ? Mathf.Min(chosen / 100f, HudLayout.LargestFor(window))
            : HudLayout.ScaleFor(window);

        Space = window / HudScale;

        // What the root stretch is already doing, measured rather than assumed, so this keeps
        // working if the project's base resolution or stretch mode is ever changed.
        float root = window.X / logical.X;
        Scale = Vector2.One * (HudScale / root);

        // What one reference pixel is worth on the glass, which is what the type has to be
        // rasterised against. The canvas scale and the root stretch cancel: the net is HudScale.
        Style.Sharpness = HudScale;

        foreach (var child in GetChildren())
        {
            if (child is not Control control)
                continue;

            control.SetAnchorsPreset(Control.LayoutPreset.TopLeft);
            control.Position = Vector2.Zero;
            control.Size = Space;
        }

        Reflowed?.Invoke();
    }
}

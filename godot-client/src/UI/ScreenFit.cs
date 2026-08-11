using Godot;

namespace Hendra.UI;

/// <summary>
/// Makes a full-screen <see cref="Control"/> actually fill the screen.
/// </summary>
/// <remarks>
/// Anchors alone are not enough here. A Control resolves its rectangle against its parent's, and
/// only a direct child of a Viewport is handed the viewport's rectangle to start from — a Control
/// under a CanvasLayer, or under a plain Node, is given nothing and quietly stays zero-sized. Every
/// anchor inside it then resolves against zero, so the whole subtree lays out on top of itself in
/// the corner and nothing appears, with no warning and no error.
///
/// Setting the size explicitly and following the viewport thereafter is the reliable fix.
/// </remarks>
public static class ScreenFit
{
    /// <summary>Sizes <paramref name="control"/> to the viewport and keeps it there.</summary>
    public static void FillScreen(this Control control)
    {
        // Anchored to the top-left corner rather than stretched to fill. Fill anchors would derive
        // the size from the parent's rectangle, which is exactly the thing that is empty here — and
        // Godot rejects an explicit size on a control whose anchors already determine one.
        control.SetAnchorsPreset(Control.LayoutPreset.TopLeft);
        Apply(control);

        // These controls live for the whole session, so there is nothing to disconnect.
        control.GetViewport().SizeChanged += () => Apply(control);
    }

    private static void Apply(Control control)
    {
        if (!GodotObject.IsInstanceValid(control))
            return;

        control.Position = Vector2.Zero;
        control.Size = control.GetViewportRect().Size;
    }
}

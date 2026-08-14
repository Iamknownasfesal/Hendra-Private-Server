using System;
using Godot;

namespace Hendra.UI;

/// <summary>
/// The menu behind Escape: leave, change something, or stop playing.
/// </summary>
/// <remarks>
/// <para>
/// The client had no way out of itself. Quitting meant closing the window, going back to the
/// character list meant the same and signing in again, and the options page was reachable only by
/// finding a gear on the player card — the <c>options</c> action exists in the input map with no
/// key bound to it at all. Escape is where every game of this shape puts that, and it was doing
/// nothing.
/// </para>
/// <para>
/// It does not pause. Nothing can: the world is on a server that keeps ticking, and a menu that
/// implied otherwise would get people killed while they read it. What it does instead is hold back
/// input — see <c>GameScene</c>, which folds this into the same predicate the options page uses —
/// so the character stands still rather than walking on under the panel.
/// </para>
/// </remarks>
public partial class SystemMenu : Control
{
    private const float PanelWidth = 340f;
    private const float Padding = 16f;
    private const float ButtonHeight = 36f;
    private const float ButtonGap = 8f;

    private ModalPanel _shell;
    private HudMenuButton _resume;
    private HudMenuButton _options;
    private HudMenuButton _nexus;
    private HudMenuButton _characters;
    private HudMenuButton _quit;

    /// <summary>Raised for the options page, which this menu does not own.</summary>
    public event Action OptionsRequested;

    /// <summary>Raised to go back to the Nexus, which is a reconnect rather than a teleport.</summary>
    public event Action NexusRequested;

    /// <summary>Raised to end the session and return to the character list.</summary>
    public event Action CharactersRequested;

    /// <summary>Raised to close the game.</summary>
    public event Action QuitRequested;

    public bool IsOpen => _shell is { Visible: true };

    /// <summary>
    /// Whether leaving for the Nexus would achieve anything.
    /// </summary>
    /// <remarks>
    /// Dimmed rather than hidden while the player is already there. A button that disappears makes
    /// the menu change shape between openings and leaves you hunting for the row that moved; a
    /// dimmed one says "this is here, and there is nothing for it to do right now".
    /// </remarks>
    public bool CanReturnToNexus
    {
        set
        {
            if (_nexus != null)
                _nexus.Disabled = !value;
        }
    }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;

        _shell = new ModalPanel("Menu");
        AddChild(_shell);

        _resume = Add("Resume", Close);
        _options = Add("Options", () => { Close(); OptionsRequested?.Invoke(); });
        _nexus = Add("Return to Nexus", () => { Close(); NexusRequested?.Invoke(); });
        _characters = Add("Character Select", () => { Close(); CharactersRequested?.Invoke(); });

        // The one that ends the session gets the warning colour, and sits apart from the rest at
        // the bottom, because it is the one press here that cannot be undone.
        _quit = Add("Quit Game", () => QuitRequested?.Invoke());
        _quit.Face = Style.HpFill.Darkened(0.35f);

        Resized += Reflow;
        if (GetParent() is HudLayer layer)
            layer.Reflowed += Reflow;

        Reflow();
    }

    private HudMenuButton Add(string label, Action pressed)
    {
        var button = new HudMenuButton(label);
        button.Pressed += pressed;
        _shell.Body.AddChild(button);
        return button;
    }

    /// <summary>Centres the panel and stacks the buttons down it.</summary>
    private void Reflow()
    {
        if (_shell == null)
            return;

        var buttons = new[] { _resume, _options, _nexus, _characters, _quit };

        // The gap before Quit is doubled, which is the whole of the separation it needs.
        float body = Padding * 2f + buttons.Length * ButtonHeight
                     + (buttons.Length - 1) * ButtonGap + ButtonGap;

        float height = body + ModalPanel.HeaderHeight + (ModalPanel.FrameWidth + 1f) * 2f;

        _shell.Size = new Vector2(PanelWidth, height);
        _shell.Position = new Vector2(
            Mathf.Round((Size.X - PanelWidth) / 2f), Mathf.Round((Size.Y - height) / 2f));

        float width = _shell.Body.Size.X - Padding * 2f;
        float y = Padding;

        foreach (var button in buttons)
        {
            if (button == _quit)
                y += ButtonGap;

            button.Position = new Vector2(Padding, y);
            button.Size = new Vector2(width, ButtonHeight);
            y += ButtonHeight + ButtonGap;
        }
    }

    public void Toggle()
    {
        if (_shell == null)
            return;

        if (_shell.Visible)
        {
            _shell.Close();
            return;
        }

        Reflow();
        _shell.Open();
    }

    public void Close() => _shell?.Close();
}

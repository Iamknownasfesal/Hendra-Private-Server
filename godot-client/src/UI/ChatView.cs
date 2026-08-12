using System;
using Godot;
using Hendra.Net.Packets;

namespace Hendra.UI;

/// <summary>
/// The chat log and its input line.
/// </summary>
/// <remarks>
/// <para>
/// The input is hidden until summoned with Enter and gives the keyboard back on submit or Escape,
/// because the movement keys are letters — leaving a focused text field around would silently eat
/// them.
/// </para>
/// <para>
/// Server-sent text is not always literal. Several packets carry either a plain string or a JSON
/// blob naming a localisation key with substitutions, and the two are told apart by whether the
/// string starts with a brace, which <c>Hendra.Text.LineBuilder</c> handles.
/// </para>
/// </remarks>
public partial class ChatView : Control
{
    private const int Width = 420;
    private const int Height = 200;
    private const int MaxLines = 150;

    private RichTextLabel _log;
    private LineEdit _input;

    /// <summary>Raised when the player submits a line. Empty lines never reach here.</summary>
    public event Action<string> Submitted;

    /// <summary>Whether the input has focus, and movement keys should be ignored.</summary>
    public bool IsTyping => _input is { Visible: true } && _input.HasFocus();

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        this.FillScreen();

        var column = new VBoxContainer();
        column.SetAnchorsPreset(LayoutPreset.BottomLeft);
        column.OffsetLeft = 8;
        column.OffsetTop = -(Height + 40);
        column.OffsetRight = 8 + Width;
        column.OffsetBottom = -8;
        column.MouseFilter = MouseFilterEnum.Ignore;
        AddChild(column);

        _log = new RichTextLabel
        {
            BbcodeEnabled = true,
            // Following is done by hand below, because the built-in kind follows unconditionally
            // and drags you off whatever you had scrolled back to read.
            ScrollFollowing = false,
            CustomMinimumSize = new Vector2(Width, Height),
            MouseFilter = MouseFilterEnum.Ignore,
        };
        column.AddChild(_log);

        _input = new LineEdit { Visible = false, PlaceholderText = "Say something" };
        _input.TextSubmitted += OnSubmitted;
        column.AddChild(_input);
    }

    /// <summary>
    /// Shows the input and takes the keyboard.
    /// </summary>
    /// <param name="prefix">
    /// Text to start with, and to leave the caret after. The original opens the box already
    /// carrying a slash, a whisper or a guild prefix depending on which key was pressed.
    /// </param>
    public void BeginTyping(string prefix = null)
    {
        _input.Visible = true;
        _input.Text = prefix ?? string.Empty;
        _input.GrabFocus();
        _input.CaretColumn = _input.Text.Length;
    }

    /// <summary>Hides the input and gives the keyboard back to the game.</summary>
    public void EndTyping()
    {
        _input.Text = string.Empty;
        _input.Visible = false;
        _input.ReleaseFocus();
    }

    private void OnSubmitted(string text)
    {
        string trimmed = text?.Trim();
        EndTyping();

        // An empty line would be worse than useless: the server tests the first character for a
        // leading slash before checking the length, so it throws on an empty string and drops the
        // connection.
        if (string.IsNullOrEmpty(trimmed))
            return;

        Submitted?.Invoke(trimmed);
    }

    /// <summary>Appends a line from the server.</summary>
    public void Add(TextPacket text, string displayText)
    {
        string name = text.Name;
        var nameColour = ColourOf(text.NameColor, new Color(0.88f, 0.88f, 0.86f));
        var bodyColour = ColourOf(text.TextColor, Colors.White);

        Append(string.IsNullOrEmpty(name)
            ? $"[color=#{bodyColour.ToHtml(false)}]{Escape(displayText)}[/color]"
            : $"[color=#{nameColour.ToHtml(false)}]{Escape(name)}:[/color] " +
              $"[color=#{bodyColour.ToHtml(false)}]{Escape(displayText)}[/color]");
    }

    /// <summary>Appends a line from the client itself.</summary>
    public void AddSystem(string message) =>
        Append($"[color=#8899aa]{Escape(message)}[/color]");

    /// <summary>
    /// Appends a line, following the log only if the reader was already at the bottom.
    /// </summary>
    /// <remarks>
    /// Someone who has scrolled back to read something is reading it. Following unconditionally --
    /// which is what the built-in setting does -- pulls them off it the moment anyone speaks, and
    /// in a busy world that is every second. So the position is only restored to the bottom when it
    /// was at the bottom to begin with.
    /// </remarks>
    private void Append(string bbcode)
    {
        var bar = _log.GetVScrollBar();
        bool wasAtBottom = bar == null || bar.Value >= bar.MaxValue - bar.Page - 1.0;

        _log.AppendText(bbcode + "\n");

        // The original kept 150 lines; without a bound this grows for the life of the session.
        while (_log.GetLineCount() > MaxLines)
            _log.RemoveParagraph(0);

        if (!wasAtBottom || bar == null)
            return;

        // Deferred because the scrollbar's range is not updated until the label has laid the new
        // line out, and scrolling to a stale maximum lands short of the bottom.
        CallDeferred(nameof(ScrollToBottom));
    }

    private void ScrollToBottom()
    {
        var bar = _log.GetVScrollBar();
        if (bar != null)
            bar.Value = bar.MaxValue;
    }

    /// <summary>
    /// Unpacks a 24-bit colour, honouring the sentinel the server uses for "not specified" -- a
    /// value no one would pick deliberately, which is presumably why it was chosen.
    /// </summary>
    private static Color ColourOf(int packed, Color fallback)
    {
        const int Unset = 0x123456;
        if (packed == Unset)
            return fallback;

        return new Color(
            (packed >> 16 & 0xFF) / 255f,
            (packed >> 8 & 0xFF) / 255f,
            (packed & 0xFF) / 255f);
    }

    /// <summary>Keeps player-authored text from being interpreted as markup.</summary>
    private static string Escape(string text) => text?.Replace("[", "[lb]") ?? string.Empty;
}

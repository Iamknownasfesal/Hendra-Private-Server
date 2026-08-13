using System;
using System.Collections.Generic;
using System.Globalization;
using Godot;
using Hendra.Account;

namespace Hendra.UI;

/// <summary>
/// The account behind the character: who is signed in, what they have, and what they have earned.
/// </summary>
/// <remarks>
/// <para>
/// The account and the character are different things and the interface used to show only the
/// second. Gold and fame are the account's, the guild is the account's, and the star rating on the
/// card is the account's total across every class it has played -- none of which is anywhere on the
/// character sheet, because none of it belongs there.
/// </para>
/// <para>
/// It comes from the same place the character sheet's tallies do: <c>/char/list</c>, which answers
/// with the account alongside the characters. Fetched when the panel opens rather than polled --
/// none of it changes while you stand still, and the two numbers that do move while you play, gold
/// and fame, are already live at the top of the screen.
/// </para>
/// </remarks>
public partial class AccountPanel : Control
{
    private const float Inset = 12f;
    private const float RowHeight = 28f;
    private const float HeaderHeight = 34f;

    /// <summary>How many classes the game has, which is the width of each star colour band.</summary>
    private const int Classes = 14;

    private readonly List<Row> _rows = new();

    private ModalPanel _shell;
    private Label _name;
    private Label _rank;
    private HudGlyph _star;
    private Label _note;

    private AppEngineClient _accounts;
    private string _guid;
    private string _password;
    private bool _fetching;

    public bool IsOpen => _shell is { Visible: true };

    /// <summary>Raised when the panel closes by any route.</summary>
    public event Action Closed;

    public void Connect(string appServerUrl, string guid, string password)
    {
        _accounts = new AppEngineClient(appServerUrl);
        _guid = guid;
        _password = password;
    }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;

        _shell = new ModalPanel("Account");
        _shell.Closed += () => Closed?.Invoke();
        AddChild(_shell);

        _name = new Label
        {
            ClipText = true,
            TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis,
        }.Typeset(18, Style.Text);
        _shell.Body.AddChild(_name);

        _star = new HudGlyph(HudIcons.Star, Fame.Colour(0, Classes));
        _shell.Body.AddChild(_star);

        _rank = new Label().Typeset(13, Style.TextDim);
        _shell.Body.AddChild(_rank);

        // Every row is the same shape, so the panel is a list rather than a layout.
        foreach (string label in Labels)
        {
            var row = new Row(label);
            _shell.Body.AddChild(row);
            _rows.Add(row);
        }

        // Only ever says what went wrong. A panel that explains itself in a sentence under the
        // numbers is a panel whose numbers are not labelled well enough.
        _note = new Label
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            AutowrapMode = TextServer.AutowrapMode.WordSmart,
        }.Typeset(Style.FontSmall, Style.TextDim);
        _shell.Body.AddChild(_note);

        Resized += Reflow;
        if (GetParent() is HudLayer layer)
            layer.Reflowed += Reflow;

        Reflow();
    }

    private static readonly string[] Labels =
    {
        "Gold",
        "Fame",
        "Guild",
        "Guild rank",
        "Characters",
        "Email",
    };

    private void Reflow()
    {
        if (_shell == null)
            return;

        var layout = new HudLayout(Size.X > 0f && Size.Y > 0f
            ? Size
            : new Vector2(HudLayout.ReferenceWidth, HudLayout.ReferenceHeight));

        // The same slot the character sheet uses: the two are never open together.
        var rect = layout.Modal;
        rect.Size = new Vector2(rect.Size.X, HeaderHeight + 44f + _rows.Count * RowHeight + 46f);

        _shell.Position = rect.Position;
        _shell.Size = rect.Size;

        float width = _shell.Body.Size.X;

        _name.Position = new Vector2(Inset, 10f);
        _name.Size = new Vector2(width - Inset * 2f - 24f, 22f);

        _star.Position = new Vector2(width - Inset - 18f, 12f);
        _star.Size = new Vector2(18f, 18f);

        _rank.Position = new Vector2(Inset, 32f);
        _rank.Size = new Vector2(width - Inset * 2f, 16f);

        for (int i = 0; i < _rows.Count; i++)
        {
            _rows[i].Position = new Vector2(Inset, HeaderHeight + 20f + i * RowHeight);
            _rows[i].Size = new Vector2(width - Inset * 2f, RowHeight);
            _rows[i].Striped = i % 2 == 1;
        }

        _note.Position = new Vector2(Inset, HeaderHeight + 26f + _rows.Count * RowHeight);
        _note.Size = new Vector2(width - Inset * 2f, 30f);
    }

    public void Toggle()
    {
        if (_shell.Visible)
        {
            _shell.Close();
            return;
        }

        _shell.Open();
        Fetch();
    }

    public void Close() => _shell?.Close();

    /// <summary>Reads the account off the character list, which is where the app server keeps it.</summary>
    private async void Fetch()
    {
        if (_accounts == null || _fetching)
            return;

        _fetching = true;
        _note.Text = "Reading the account…";

        try
        {
            string xml = await _accounts.PostAsync("/char/list", new Dictionary<string, string>
            {
                ["guid"] = _guid,
                ["password"] = _password,
            });

            Fill(CharListResult.Parse(xml));
        }
        catch (Exception ex)
        {
            _note.Text = "Could not reach the account server.";
            GD.PushWarning($"[account] {ex.Message}");
        }
        finally
        {
            _fetching = false;
        }
    }

    private void Fill(CharListResult list)
    {
        var account = list.Account;

        _name.Text = string.IsNullOrEmpty(account.Name) ? _guid : account.Name;

        // The rating on the card is this: the account's stars across every class it has played, and
        // the colour band they fall in.
        _star.Tint = Fame.Colour(account.Rank, Classes, account.Admin);
        _rank.Text = account.Admin
            ? $"{account.Rank} stars · administrator"
            : $"{account.Rank} stars";

        _rows[0].Set(account.Credits.ToString("N0", CultureInfo.InvariantCulture), Style.IconGold);
        _rows[1].Set(account.Fame.ToString("N0", CultureInfo.InvariantCulture), Style.IconFame);
        _rows[2].Set(string.IsNullOrEmpty(account.GuildName) ? "—" : account.GuildName, Style.Text);
        _rows[3].Set(GuildRank(account), Style.Text);
        _rows[4].Set(list.Characters.Count.ToString(CultureInfo.InvariantCulture), Style.Text);
        _rows[5].Set(account.VerifiedEmail ? "Verified" : "Not verified",
            account.VerifiedEmail ? Style.StatBonus : Style.StatPenalty);

        _note.Text = string.Empty;
    }

    /// <summary>The guild ranks the server uses, which are numbers on the wire.</summary>
    private static string GuildRank(AccountInfo account) => string.IsNullOrEmpty(account.GuildName)
        ? "—"
        : account.GuildRank switch
        {
            >= 40 => "Founder",
            >= 30 => "Leader",
            >= 20 => "Officer",
            >= 10 => "Member",
            _ => "Initiate",
        };

    /// <summary>One line of the account: its name on the left, its value on the right.</summary>
    private sealed partial class Row : Control
    {
        private readonly Label _label;
        private readonly Label _value;

        private bool _striped;

        public Row(string label)
        {
            MouseFilter = MouseFilterEnum.Ignore;

            _label = new Label { Text = label, VerticalAlignment = VerticalAlignment.Center }
                .Typeset(13, Style.Text);
            AddChild(_label);

            _value = new Label
            {
                HorizontalAlignment = HorizontalAlignment.Right,
                VerticalAlignment = VerticalAlignment.Center,
                ClipText = true,
                TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis,
            }.Typeset(13, Style.StatNumber);
            AddChild(_value);
        }

        public bool Striped
        {
            get => _striped;
            set { _striped = value; QueueRedraw(); }
        }

        public void Set(string value, Color colour)
        {
            _value.Text = value;
            _value.AddThemeColorOverride("font_color", colour);
        }

        public override void _Notification(int what)
        {
            base._Notification(what);

            if (what != NotificationResized)
                return;

            _label.Position = new Vector2(6f, 0f);
            _label.Size = new Vector2(Size.X / 2f, Size.Y);

            _value.Position = new Vector2(Size.X / 2f - 6f, 0f);
            _value.Size = new Vector2(Size.X / 2f, Size.Y);
        }

        public override void _Draw()
        {
            if (_striped)
                DrawRect(new Rect2(Vector2.Zero, Size), Style.ModalStripe);
        }
    }
}

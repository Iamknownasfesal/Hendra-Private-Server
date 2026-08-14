using System;
using System.Collections.Generic;
using System.Globalization;
using Godot;
using Hendra.Account;
using Hendra.Net;
using Hendra.Net.Packets;

namespace Hendra.UI;

/// <summary>
/// The guild roll, and the buttons that change it.
/// </summary>
/// <remarks>
/// <para>
/// Two halves that do not talk to each other. The roll comes over HTTP from
/// <c>/guild/listMembers</c>, which is the only place it exists; membership changes go over the
/// game socket as packets, and the server answers them with a GuildResult that lands in the chat
/// log. So an action here refetches the roll rather than editing the copy it is showing — the
/// server is the only thing that knows whether the change was allowed.
/// </para>
/// <para>
/// Everything the panel offers is also reachable by typing a slash command, which is how the
/// original expected most of it to be done. The panel exists because remembering that
/// <c>/gkick</c> exists is a lot to ask of someone who just wants to see who is in their guild.
/// </para>
/// <para>
/// The row actions are drawn icons rather than typed characters. They used to be <c>▲ ▼ ✕</c> in a
/// button's text, which the interface's pixel face has no glyphs for — three missing-glyph boxes
/// beside every member you outrank. See <see cref="HudIcons.Cross"/>.
/// </para>
/// </remarks>
public partial class GuildView : Control
{
    private const int PanelWidth = 540;
    private const int PanelHeight = 520;

    private const float Padding = 14f;
    private const float RowHeight = 26f;
    private const float FieldHeight = 30f;
    private const float ButtonHeight = 30f;

    /// <summary>The columns a member row is laid out on, measured from the row's left edge.</summary>
    private const float RankColumn = 210f;

    private const float FameColumn = 320f;
    private const float ActionSize = 22f;

    private ModalPanel _shell;
    private Label _name;
    private Label _summary;
    private Label _status;
    private ScrollContainer _scroll;
    private VBoxContainer _roll;
    private LineEdit _nameField;
    private HudMenuButton _invite;
    private HudMenuButton _create;
    private HudMenuButton _refresh;
    private HudMenuButton _leave;

    private GameSession _session;
    private string _appServerUrl = string.Empty;
    private string _guid = string.Empty;
    private string _password = string.Empty;
    private string _accountName = string.Empty;

    private GuildResult _fetched;
    private string _fetchError;

    public bool IsOpen => _shell is { Visible: true };

    public void Configure(GameSession session, string appServerUrl, string guid, string password)
    {
        _session = session;
        _appServerUrl = appServerUrl;
        _guid = guid;
        _password = password;
    }

    /// <summary>The player's own name, used to work out what they are allowed to do.</summary>
    public string AccountName { set => _accountName = value ?? string.Empty; }

    public override void _Ready()
    {
        // Sized by the HUD canvas, in reference pixels, like every other panel.
        MouseFilter = MouseFilterEnum.Ignore;

        _shell = new ModalPanel("Guild");
        AddChild(_shell);

        _name = new Label
        {
            ClipText = true,
            TextOverrunBehavior = TextServer.OverrunBehavior.TrimEllipsis,
        }.Typeset(Style.FontName, Style.Text);
        _shell.Body.AddChild(_name);

        _summary = new Label().Typeset(Style.FontSmall, Style.TextDim);
        _shell.Body.AddChild(_summary);

        _status = new Label { AutowrapMode = TextServer.AutowrapMode.WordSmart }
            .Typeset(Style.FontSmall, Style.StatLabel);
        _shell.Body.AddChild(_status);

        _scroll = new ScrollContainer();
        _shell.Body.AddChild(_scroll);

        _roll = new VBoxContainer { SizeFlagsHorizontal = SizeFlags.ExpandFill };
        _roll.AddThemeConstantOverride("separation", 2);
        _scroll.AddChild(_roll);

        _nameField = new LineEdit { PlaceholderText = "Player name" };
        _shell.Body.AddChild(_nameField);

        _invite = new HudMenuButton("Invite");
        _invite.Pressed += () => Send(new GuildInvitePacket { Name = _nameField.Text });
        _shell.Body.AddChild(_invite);

        _create = new HudMenuButton("Create a guild with this name");
        _create.Pressed += () => Send(new CreateGuildPacket { Name = _nameField.Text });
        _shell.Body.AddChild(_create);

        _refresh = new HudMenuButton("Refresh");
        _refresh.Pressed += Refresh;
        _shell.Body.AddChild(_refresh);

        _leave = new HudMenuButton("Leave") { Face = Style.HpFill.Darkened(0.35f) };
        _leave.Pressed += () => Send(new GuildRemovePacket { Name = _accountName });
        _shell.Body.AddChild(_leave);

        Resized += Reflow;
        if (GetParent() is HudLayer layer)
            layer.Reflowed += Reflow;

        Reflow();
    }

    /// <summary>Centres the panel and stacks its parts down the body.</summary>
    private void Reflow()
    {
        if (_shell == null)
            return;

        _shell.Size = new Vector2(PanelWidth, PanelHeight);
        _shell.Position = new Vector2(
            Mathf.Round((Size.X - PanelWidth) / 2f), Mathf.Round((Size.Y - PanelHeight) / 2f));

        float width = _shell.Body.Size.X - Padding * 2f;
        float y = Padding;

        Place(_name, y, width, 24f);
        y += 26f;

        Place(_summary, y, width, 16f);
        y += 20f;

        // The status line only takes room when it has something to say, so a healthy panel does not
        // carry an empty band across it.
        float statusHeight = string.IsNullOrEmpty(_status.Text) ? 0f : 34f;
        Place(_status, y, width, statusHeight);
        y += statusHeight > 0f ? statusHeight + 4f : 0f;

        // Everything below the roll is measured from the bottom, so the list absorbs the slack.
        float bottom = _shell.Body.Size.Y - Padding;

        Place(_refresh, bottom - ButtonHeight, Mathf.Round((width - 8f) / 2f), ButtonHeight);
        _leave.Position = new Vector2(
            Padding + Mathf.Round((width - 8f) / 2f) + 8f, bottom - ButtonHeight);
        _leave.Size = new Vector2(Mathf.Round((width - 8f) / 2f), ButtonHeight);

        bottom -= ButtonHeight + 8f;

        if (_create.Visible)
        {
            Place(_create, bottom - ButtonHeight, width, ButtonHeight);
            bottom -= ButtonHeight + 8f;
        }

        if (_invite.Visible)
        {
            float inviteWidth = 96f;

            _nameField.Position = new Vector2(Padding, bottom - FieldHeight);
            _nameField.Size = new Vector2(width - inviteWidth - 8f, FieldHeight);

            _invite.Position = new Vector2(Padding + width - inviteWidth, bottom - FieldHeight);
            _invite.Size = new Vector2(inviteWidth, FieldHeight);

            bottom -= FieldHeight + 8f;
        }
        else
        {
            // The field belongs to whichever of the two is showing; with neither, it goes with them.
            _nameField.Visible = _create.Visible;

            if (_create.Visible)
            {
                _nameField.Position = new Vector2(Padding, bottom - FieldHeight);
                _nameField.Size = new Vector2(width, FieldHeight);
                bottom -= FieldHeight + 8f;
            }
        }

        _scroll.Position = new Vector2(Padding, y);
        _scroll.Size = new Vector2(width, Mathf.Max(0f, bottom - y));
    }

    private void Place(Control control, float y, float width, float height)
    {
        control.Position = new Vector2(Padding, y);
        control.Size = new Vector2(width, height);
    }

    /// <summary>Opens the panel, or closes it if it is already open. Opening refetches the roll.</summary>
    public void Toggle()
    {
        if (_shell == null)
            return;

        _shell.Toggle();

        if (_shell.Visible)
            Refresh();
    }

    private void Refresh()
    {
        _status.Text = "Loading…";
        Reflow();
        _ = LoadAsync();
    }

    private async System.Threading.Tasks.Task LoadAsync()
    {
        try
        {
            using var client = new AppEngineClient(_appServerUrl);
            string xml = await client.PostAsync("/guild/listMembers", new Dictionary<string, string>
            {
                ["guid"] = _guid,
                ["password"] = _password,
            });

            _fetched = GuildResult.Parse(xml);
            _fetchError = null;
        }
        catch (Exception ex)
        {
            _fetched = null;

            // "Not in guild" is the ordinary answer for most accounts, not a fault, so it is shown
            // as the state it is rather than as an error.
            _fetchError = ex.Message;
        }

        CallDeferred(nameof(Apply));
    }

    private void Apply()
    {
        foreach (var child in _roll.GetChildren())
            child.QueueFree();

        if (_fetched == null)
        {
            bool notInGuild = _fetchError != null &&
                              _fetchError.Contains("Not in guild", StringComparison.OrdinalIgnoreCase);

            _name.Text = notInGuild ? "No guild" : "Guild";
            _summary.Text = string.Empty;

            _status.Text = notInGuild
                ? "Type a name below and create one, or ask someone for an invitation."
                : $"The roll could not be fetched — {_fetchError}";

            _status.AddThemeColorOverride(
                "font_color", notInGuild ? Style.StatLabel : Style.StatPenalty);

            _invite.Visible = false;
            _create.Visible = notInGuild;
            _leave.Visible = false;
            _nameField.Visible = notInGuild;

            Reflow();
            return;
        }

        _status.Text = string.Empty;
        _name.Text = _fetched.Name;
        _summary.Text =
            $"{_fetched.Members.Count} members · {Number(_fetched.CurrentFame)} fame · {_fetched.HallType}";

        _create.Visible = false;
        _leave.Visible = true;

        int myRank = _fetched.RankOf(_accountName);
        bool canManage = GuildRank.CanManage(myRank);

        _invite.Visible = canManage;
        _nameField.Visible = canManage;

        int at = 0;
        foreach (var member in _fetched.Members)
            _roll.AddChild(new MemberRow(member, myRank, canManage, this, at++));

        Reflow();
    }

    /// <summary>
    /// A grouped number.
    /// </summary>
    /// <remarks>
    /// Invariant rather than the machine's locale: the interface is English throughout, and a
    /// locale that groups with a full stop turns twelve hundred fame into one point two.
    /// </remarks>
    private static string Number(int value) => value.ToString("N0", CultureInfo.InvariantCulture);

    /// <summary>
    /// One person on the roll: name, rank, fame, and what you may do about them.
    /// </summary>
    /// <remarks>
    /// Drawn rather than assembled out of labels in a box container. The columns have to line up
    /// down the list, which a HBoxContainer only manages if every cell is given a minimum width —
    /// at which point it is a table expressed as a layout tree, and reading it is harder than the
    /// three DrawText calls it replaces.
    /// </remarks>
    private sealed partial class MemberRow : Control
    {
        private readonly GuildMemberInfo _member;
        private readonly bool _striped;

        public MemberRow(
            GuildMemberInfo member, int myRank, bool canManage, GuildView owner, int index)
        {
            _member = member;
            _striped = index % 2 == 1;
            CustomMinimumSize = new Vector2(0, RowHeight);
            MouseFilter = MouseFilterEnum.Ignore;

            // Only over people below you, and never over yourself. The server enforces the same
            // rule and answers a refusal with a message; this just keeps the buttons honest.
            bool over = canManage && member.Rank < myRank &&
                        !string.Equals(member.Name, owner._accountName, StringComparison.OrdinalIgnoreCase);

            if (!over)
                return;

            Action(0, HudIcons.ChevronUp, "Promote", () => owner.ChangeRank(member, up: true));
            Action(1, HudIcons.ChevronDown, "Demote", () => owner.ChangeRank(member, up: false));
            Action(2, HudIcons.Cross, "Remove from the guild",
                () => owner.Send(new GuildRemovePacket { Name = member.Name }));
        }

        private void Action(int index, Action<CanvasItem, Rect2, Color> icon, string tip, Action pressed)
        {
            var button = new HudIconButton(icon, tip, inset: 5f)
            {
                Tint = Style.TextDim,
                Position = new Vector2(FameColumn + 24f + index * (ActionSize + 4f),
                    Mathf.Round((RowHeight - ActionSize) / 2f)),
                Size = new Vector2(ActionSize, ActionSize),
            };

            button.Pressed += pressed;
            AddChild(button);
        }

        public override void _Draw()
        {
            // Alternate rows carry a faint wash. A roll runs to dozens of names and three columns,
            // and the stripe is what stops the eye sliding onto the wrong line's fame.
            if (_striped)
                DrawRect(new Rect2(Vector2.Zero, Size), Style.ModalStripe);

            float baseline = Style.BaselineIn(RowHeight, Style.FontSmall);

            // Online is the one thing worth colouring: it is the difference between inviting
            // someone to a dungeon and typing into the void.
            this.DrawText(new Vector2(0f, baseline), _member.Name, Style.FontSmall,
                _member.IsOnline ? Style.StatBonus : Style.Text);

            this.DrawText(new Vector2(RankColumn, baseline), GuildRank.Name(_member.Rank),
                Style.FontSmall, Style.TextDim);

            string fame = Number(_member.Fame);
            this.DrawText(
                new Vector2(FameColumn - Style.Measure(fame, Style.FontSmall), baseline),
                fame, Style.FontSmall, Style.StatNumber);
        }
    }

    /// <summary>
    /// Moves a member one step up or down the ladder.
    /// </summary>
    /// <remarks>
    /// The wire carries the rank itself rather than a direction, so the neighbouring rank has to be
    /// worked out here. Steps that would go off either end do nothing rather than sending a value
    /// the server will refuse.
    /// </remarks>
    private void ChangeRank(GuildMemberInfo member, bool up)
    {
        var ladder = GuildRank.Assignable;
        int index = Array.IndexOf(ladder, member.Rank);
        if (index < 0)
            return;

        int next = index + (up ? 1 : -1);
        if (next < 0 || next >= ladder.Length)
            return;

        Send(new ChangeGuildRankPacket { Name = member.Name, GuildRank = ladder[next] });
    }

    /// <summary>
    /// Sends a membership change and refetches.
    /// </summary>
    /// <remarks>
    /// The refetch is delayed because the server applies the change and answers on the game socket,
    /// while the roll is read over HTTP from the database — asking immediately races the write.
    /// </remarks>
    private void Send(ClientPacket packet)
    {
        if (_session == null)
            return;

        _session.Send(packet);
        GetTree().CreateTimer(0.6).Timeout += Refresh;
    }
}

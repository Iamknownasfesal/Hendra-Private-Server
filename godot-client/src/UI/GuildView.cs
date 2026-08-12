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
/// </remarks>
public partial class GuildView : Control
{
    private const int PanelWidth = 480;

    private CutEdgePanel _panel;
    private Label _title;
    private Label _summary;
    private Label _status;
    private VBoxContainer _roll;
    private VBoxContainer _manage;
    private LineEdit _nameField;
    private Button _inviteButton;
    private Button _createButton;

    private GameSession _session;
    private string _appServerUrl = string.Empty;
    private string _guid = string.Empty;
    private string _password = string.Empty;
    private string _accountName = string.Empty;

    private GuildResult _fetched;
    private string _fetchError;

    public bool IsOpen => _panel is { Visible: true };

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
        MouseFilter = MouseFilterEnum.Ignore;
        this.FillScreen();

        _panel = new CutEdgePanel { Visible = false, MouseFilter = MouseFilterEnum.Stop };
        _panel.Background = CutEdgePanel.PanelBackground;
        _panel.Border = new Color(0.42f, 0.42f, 0.42f);
        _panel.Padded(0);
        _panel.SetAnchorsPreset(LayoutPreset.Center);
        _panel.OffsetLeft = -PanelWidth / 2f;
        _panel.OffsetRight = PanelWidth / 2f;
        _panel.OffsetTop = -230;
        _panel.OffsetBottom = 230;
        AddChild(_panel);

        var margin = new MarginContainer();
        foreach (string side in new[] { "margin_left", "margin_top", "margin_right", "margin_bottom" })
            margin.AddThemeConstantOverride(side, 14);
        _panel.AddChild(margin);

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 8);
        margin.AddChild(column);

        _title = new Label { Text = "Guild" };
        _title.AddThemeFontSizeOverride("font_size", 20);
        column.AddChild(_title);

        _summary = new Label();
        _summary.AddThemeColorOverride("font_color", new Color(0.72f, 0.72f, 0.72f));
        column.AddChild(_summary);

        _status = new Label { AutowrapMode = TextServer.AutowrapMode.WordSmart };
        _status.AddThemeColorOverride("font_color", new Color(0.9f, 0.7f, 0.5f));
        column.AddChild(_status);

        var scroll = new ScrollContainer { SizeFlagsVertical = SizeFlags.ExpandFill };
        column.AddChild(scroll);

        _roll = new VBoxContainer { SizeFlagsHorizontal = SizeFlags.ExpandFill };
        _roll.AddThemeConstantOverride("separation", 2);
        scroll.AddChild(_roll);

        _manage = new VBoxContainer { Visible = false };
        _manage.AddThemeConstantOverride("separation", 6);
        column.AddChild(_manage);

        var inviteRow = new HBoxContainer();
        inviteRow.AddThemeConstantOverride("separation", 6);
        _manage.AddChild(inviteRow);

        _nameField = new LineEdit
        {
            PlaceholderText = "Player name",
            SizeFlagsHorizontal = SizeFlags.ExpandFill,
        };
        inviteRow.AddChild(_nameField);

        _inviteButton = new GameButton("Invite", compact: true);
        _inviteButton.Pressed += () => Send(new GuildInvitePacket { Name = _nameField.Text });
        inviteRow.AddChild(_inviteButton);

        var joinRow = new HBoxContainer();
        joinRow.AddThemeConstantOverride("separation", 6);
        column.AddChild(joinRow);

        _createButton = new GameButton("Create a guild with this name", compact: true);
        _createButton.Pressed += () => Send(new CreateGuildPacket { Name = _nameField.Text });
        joinRow.AddChild(_createButton);

        var buttons = new HBoxContainer();
        buttons.AddThemeConstantOverride("separation", 6);
        column.AddChild(buttons);

        var refresh = new GameButton("Refresh", compact: true);
        refresh.Pressed += Refresh;
        buttons.AddChild(refresh);

        var leave = new GameButton("Leave", compact: true);
        leave.Pressed += () => Send(new GuildRemovePacket { Name = _accountName });
        buttons.AddChild(leave);

        var close = new GameButton("Close", compact: true);
        close.Pressed += Toggle;
        buttons.AddChild(close);
    }

    /// <summary>Opens the panel, or closes it if it is already open. Opening refetches the roll.</summary>
    public void Toggle()
    {
        if (_panel == null)
            return;

        _panel.Visible = !_panel.Visible;

        if (_panel.Visible)
            Refresh();
    }

    private void Refresh()
    {
        _status.Text = "Loading…";
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

            _title.Text = "Guild";
            _summary.Text = string.Empty;
            _status.Text = notInGuild
                ? "You are not in a guild. Type a name and create one, or ask someone for an invitation."
                : $"The roll could not be fetched — {_fetchError}";

            _manage.Visible = false;
            _createButton.Visible = notInGuild;
            return;
        }

        _status.Text = string.Empty;
        _title.Text = _fetched.Name;
        _summary.Text = $"{_fetched.Members.Count} members · {Number(_fetched.CurrentFame)} fame · {_fetched.HallType}";
        _createButton.Visible = false;

        int myRank = _fetched.RankOf(_accountName);
        bool canManage = GuildRank.CanManage(myRank);
        _manage.Visible = canManage;

        foreach (var member in _fetched.Members)
            _roll.AddChild(BuildMemberRow(member, myRank, canManage));
    }

    /// <summary>
    /// A grouped number.
    /// </summary>
    /// <remarks>
    /// Invariant rather than the machine's locale: the interface is English throughout, and a
    /// locale that groups with a full stop turns twelve hundred fame into one point two.
    /// </remarks>
    private static string Number(int value) => value.ToString("N0", CultureInfo.InvariantCulture);

    private Control BuildMemberRow(GuildMemberInfo member, int myRank, bool canManage)
    {
        var row = new HBoxContainer();
        row.AddThemeConstantOverride("separation", 8);

        var name = new Label { Text = member.Name, CustomMinimumSize = new Vector2(150, 0) };
        if (member.IsOnline)
            name.AddThemeColorOverride("font_color", new Color(0.55f, 0.95f, 0.55f));
        row.AddChild(name);

        var rank = new Label { Text = GuildRank.Name(member.Rank), CustomMinimumSize = new Vector2(80, 0) };
        rank.AddThemeColorOverride("font_color", new Color(0.7f, 0.7f, 0.7f));
        row.AddChild(rank);

        var fame = new Label
        {
            Text = Number(member.Fame),
            CustomMinimumSize = new Vector2(70, 0),
            HorizontalAlignment = HorizontalAlignment.Right,
        };
        row.AddChild(fame);

        // Only over people below you, and never over yourself. The server enforces the same rule
        // and answers a refusal with a message; this just keeps the buttons honest.
        bool overThem = canManage && member.Rank < myRank &&
                        !string.Equals(member.Name, _accountName, StringComparison.OrdinalIgnoreCase);

        if (!overThem)
            return row;

        var promote = new GameButton("▲", compact: true) { TooltipText = "Promote" };
        promote.Pressed += () => ChangeRank(member, up: true);
        row.AddChild(promote);

        var demote = new GameButton("▼", compact: true) { TooltipText = "Demote" };
        demote.Pressed += () => ChangeRank(member, up: false);
        row.AddChild(demote);

        var remove = new GameButton("✕", compact: true) { TooltipText = "Remove from the guild" };
        remove.Pressed += () => Send(new GuildRemovePacket { Name = member.Name });
        row.AddChild(remove);

        return row;
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

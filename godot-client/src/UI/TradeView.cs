using System;
using System.Collections.Generic;
using Godot;
using Hendra.Assets;
using Hendra.Resources;
using Hendra.World;

namespace Hendra.UI;

/// <summary>
/// The trade window: both offers side by side, with accept and cancel.
/// </summary>
/// <remarks>
/// Appears only while a trade is open. Clicking one of your own slots puts it on the table or takes
/// it back; the partner's side is display only.
///
/// Slots that cannot be traded are drawn dimmed rather than hidden, so it is clear *why* something
/// cannot be offered rather than leaving the player wondering where it went.
/// </remarks>
public partial class TradeView : Control
{
    private const int Width = 460;
    private const int SlotSize = 40;
    private const int Columns = 4;

    private readonly List<SlotView> _mine = new();
    private readonly List<SlotView> _theirs = new();

    private CutEdgePanel _panel;
    private Label _title;
    private Label _status;
    private Button _accept;

    private Trading _trading;
    private GameData _data;
    private TextureResolver _textures;

    public void Configure(Trading trading, AssetLibrary assets, GameData data)
    {
        _trading = trading;
        _data = data;
        _textures = new TextureResolver(assets);

        _trading.Changed += Refresh;
        _trading.Ended += _ => Refresh();
    }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Ignore;
        this.FillScreen();

        _panel = new CutEdgePanel { Visible = false, MouseFilter = MouseFilterEnum.Stop };
        _panel.Background = CutEdgePanel.PanelBackground;
        _panel.Border = new Color(0.42f, 0.42f, 0.42f);
        _panel.Padded(0);
        _panel.SetAnchorsPreset(LayoutPreset.CenterTop);
        _panel.OffsetLeft = -Width / 2f;
        _panel.OffsetRight = Width / 2f;
        _panel.OffsetTop = 40;
        AddChild(_panel);

        var margin = new MarginContainer();
        foreach (string side in new[] { "margin_left", "margin_top", "margin_right", "margin_bottom" })
            margin.AddThemeConstantOverride(side, 10);
        _panel.AddChild(margin);

        var column = new VBoxContainer();
        column.AddThemeConstantOverride("separation", 8);
        margin.AddChild(column);

        _title = new Label { Text = "Trade" };
        column.AddChild(_title);

        var offers = new HBoxContainer();
        offers.AddThemeConstantOverride("separation", 16);
        column.AddChild(offers);

        offers.AddChild(BuildSide("You offer", _mine, mine: true));
        offers.AddChild(BuildSide("They offer", _theirs, mine: false));

        _status = new Label { AutowrapMode = TextServer.AutowrapMode.WordSmart };
        column.AddChild(_status);

        var buttons = new HBoxContainer();
        buttons.AddThemeConstantOverride("separation", 8);
        column.AddChild(buttons);

        _accept = new Button { Text = "Accept" };
        _accept.Pressed += () => _trading?.Accept();
        buttons.AddChild(_accept);

        var cancel = new Button { Text = "Cancel" };
        cancel.Pressed += () => _trading?.Cancel();
        buttons.AddChild(cancel);
    }

    private Control BuildSide(string heading, List<SlotView> into, bool mine)
    {
        var column = new VBoxContainer();
        column.AddChild(new Label { Text = heading });

        var grid = new GridContainer { Columns = Columns };
        grid.AddThemeConstantOverride("h_separation", 4);
        grid.AddThemeConstantOverride("v_separation", 4);
        column.AddChild(grid);

        for (int i = 0; i < 12; i++)
        {
            int slot = i;
            var view = new SlotView { CustomMinimumSize = new Vector2(SlotSize, SlotSize) };

            if (mine)
                view.Activated += () => _trading?.ToggleOffer(slot);

            grid.AddChild(view);
            into.Add(view);
        }

        return column;
    }

    private void Refresh()
    {
        if (_trading == null || _panel == null)
            return;

        _panel.Visible = _trading.IsOpen;
        if (!_trading.IsOpen)
            return;

        _title.Text = $"Trading with {_trading.PartnerName}";

        FillSide(_mine, _trading.MyItems, _trading.MyOffer, mine: true);
        FillSide(_theirs, _trading.TheirItems, _trading.TheirOffer, mine: false);

        string theirs = _trading.TheyAccepted ? "accepted" : "has not accepted";
        _status.Text = $"You are offering {_trading.OfferedCount}. {_trading.PartnerName} {theirs}.";
        _accept.Disabled = _trading.IAccepted;
    }

    private void FillSide(List<SlotView> views, Data.TradeItem[] items, bool[] offered, bool mine)
    {
        for (int i = 0; i < views.Count; i++)
        {
            if (i >= items.Length || items[i].Item < 0)
            {
                views[i].SetItem(default, null);
                views[i].Modulate = Colors.White;
                continue;
            }

            var desc = _data?.GetObject((ushort)items[i].Item);
            var resolved = _textures?.Resolve(desc?.Texture) ?? default;
            views[i].SetItem(resolved.Still, desc?.DisplayId ?? desc?.Id);

            bool onTable = offered != null && i < offered.Length && offered[i];
            bool tradeable = items[i].Tradeable && (!mine || i >= Trading.FirstTradeableSlot);

            // Offered items are highlighted; untradeable ones are dimmed rather than hidden, so it
            // is obvious why they cannot be put on the table.
            views[i].Modulate = !tradeable
                ? new Color(0.45f, 0.45f, 0.45f)
                : onTable
                    ? new Color(1f, 1f, 0.6f)
                    : Colors.White;
        }
    }
}

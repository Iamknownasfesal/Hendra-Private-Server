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
/// <para>
/// Appears only while a trade is open. Clicking one of your own slots puts it on the table or takes
/// it back; the partner's side is display only.
/// </para>
/// <para>
/// The state of a square is carried by its plate rather than by fading the artwork on it. An earlier
/// pass tinted the sprite -- yellow for offered, grey for untradeable -- which is the one thing an
/// item slot must not do: the artwork is how you tell a Ring of Paramount Health from a Ring of
/// Paramount Attack, and recolouring it to say something about the trade damages the only channel
/// that was already working. <see cref="SlotHighlight"/> exists for this, and now carries both
/// meanings.
/// </para>
/// <para>
/// Escape cancels rather than merely hiding. A trade window you can dismiss while the trade stays
/// open on the server is a way to lose items without being shown what you agreed to.
/// </para>
/// </remarks>
public partial class TradeView : Control
{
    private const int PanelWidth = 512;

    /// <summary>Slots per side. The server exchanges 4 upward, but the panel shows the lot.</summary>
    private const int Slots = 12;

    private const int Columns = 4;
    private const float SlotSize = 52f;
    private const float SlotGap = 4f;
    private const float Padding = 16f;
    private const float HeadingHeight = 22f;
    private const float StatusHeight = 24f;
    private const float ButtonHeight = 30f;

    /// <summary>One side's grid, which is what the two columns are sized from.</summary>
    private const float ColumnWidth = Columns * SlotSize + (Columns - 1) * SlotGap;

    private const float ColumnGap = 24f;

    private readonly List<SlotView> _mine = new(Slots);
    private readonly List<SlotView> _theirs = new(Slots);

    private ModalPanel _shell;
    private Label _mineHeading;
    private Label _theirsHeading;
    private Label _status;
    private HudMenuButton _accept;
    private HudMenuButton _cancel;

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
        // Sized by the HUD canvas, in reference pixels, like every other panel.
        MouseFilter = MouseFilterEnum.Ignore;

        _shell = new ModalPanel("Trade");

        // Dismissing the panel ends the trade. Cancel is idempotent on the client and the server
        // ignores it when nothing is open, so the guard is only to keep the log quiet.
        _shell.Closed += () =>
        {
            if (_trading is { IsOpen: true })
                _trading.Cancel();
        };

        AddChild(_shell);

        _mineHeading = new Label().Typeset(Style.FontSmall, Style.StatLabel);
        _shell.Body.AddChild(_mineHeading);

        _theirsHeading = new Label().Typeset(Style.FontSmall, Style.StatLabel);
        _shell.Body.AddChild(_theirsHeading);

        BuildSide(_mine, mine: true);
        BuildSide(_theirs, mine: false);

        _status = new Label { VerticalAlignment = VerticalAlignment.Center }
            .Typeset(Style.FontBody, Style.TextDim);
        _shell.Body.AddChild(_status);

        _accept = new HudMenuButton("Accept");
        _accept.Pressed += () => _trading?.Accept();
        _shell.Body.AddChild(_accept);

        _cancel = new HudMenuButton("Cancel");
        _cancel.Pressed += () => _trading?.Cancel();
        _shell.Body.AddChild(_cancel);

        Resized += Reflow;
        if (GetParent() is HudLayer layer)
            layer.Reflowed += Reflow;

        Reflow();
    }

    private void BuildSide(List<SlotView> into, bool mine)
    {
        for (int i = 0; i < Slots; i++)
        {
            int slot = i;
            var view = new SlotView();

            if (mine)
                view.Activated += () => _trading?.ToggleOffer(slot);

            _shell.Body.AddChild(view);
            into.Add(view);
        }
    }

    /// <summary>Centres the panel and lays out the two grids inside it.</summary>
    private void Reflow()
    {
        if (_shell == null)
            return;

        float rows = Mathf.Ceil(Slots / (float)Columns);
        float gridHeight = rows * SlotSize + (rows - 1) * SlotGap;

        float bodyHeight = Padding + HeadingHeight + gridHeight + Padding
                           + StatusHeight + 8f + ButtonHeight + Padding;

        float height = bodyHeight + ModalPanel.HeaderHeight + (ModalPanel.FrameWidth + 1f) * 2f;

        _shell.Size = new Vector2(PanelWidth, height);
        _shell.Position = new Vector2(
            Mathf.Round((Size.X - PanelWidth) / 2f), Mathf.Round((Size.Y - height) / 3f));

        float width = _shell.Body.Size.X;
        float left = Mathf.Round((width - (ColumnWidth * 2f + ColumnGap)) / 2f);
        float right = left + ColumnWidth + ColumnGap;
        float top = Padding;

        Place(_mineHeading, left, top, ColumnWidth, HeadingHeight);
        Place(_theirsHeading, right, top, ColumnWidth, HeadingHeight);

        float gridTop = top + HeadingHeight;
        LaySide(_mine, left, gridTop);
        LaySide(_theirs, right, gridTop);

        float below = gridTop + gridHeight + Padding;
        Place(_status, left, below, ColumnWidth * 2f + ColumnGap, StatusHeight);

        float buttonsTop = below + StatusHeight + 8f;
        float buttonWidth = Mathf.Round((ColumnWidth * 2f + ColumnGap - 8f) / 2f);

        Place(_accept, left, buttonsTop, buttonWidth, ButtonHeight);
        Place(_cancel, left + buttonWidth + 8f, buttonsTop, buttonWidth, ButtonHeight);
    }

    private static void Place(Control control, float x, float y, float width, float height)
    {
        control.Position = new Vector2(x, y);
        control.Size = new Vector2(width, height);
    }

    private static void LaySide(List<SlotView> views, float left, float top)
    {
        for (int i = 0; i < views.Count; i++)
        {
            views[i].Position = new Vector2(
                left + i % Columns * (SlotSize + SlotGap),
                top + i / Columns * (SlotSize + SlotGap));

            views[i].Size = new Vector2(SlotSize, SlotSize);
        }
    }

    private void Refresh()
    {
        if (_trading == null || _shell == null)
            return;

        if (!_trading.IsOpen)
        {
            // Straight to hidden rather than through Close(), which would cancel a trade that has
            // already finished on its own.
            _shell.Visible = false;
            return;
        }

        if (!_shell.Visible)
        {
            Reflow();
            _shell.Open();
        }

        _mineHeading.Text = "YOU OFFER";
        _theirsHeading.Text = $"{_trading.PartnerName.ToUpperInvariant()} OFFERS";

        FillSide(_mine, _trading.MyItems, _trading.MyOffer, mine: true);
        FillSide(_theirs, _trading.TheirItems, _trading.TheirOffer, mine: false);

        _status.Text = _trading.TheyAccepted
            ? $"{_trading.PartnerName} has accepted. {Offered()}"
            : $"{_trading.PartnerName} has not accepted yet. {Offered()}";

        _accept.Disabled = _trading.IAccepted;
    }

    /// <summary>The count, worded so that nought reads as a deliberate choice rather than a bug.</summary>
    private string Offered() => _trading.OfferedCount switch
    {
        0 => "You are offering nothing.",
        1 => "You are offering 1 item.",
        var n => $"You are offering {n} items.",
    };

    private void FillSide(List<SlotView> views, Data.TradeItem[] items, bool[] offered, bool mine)
    {
        for (int i = 0; i < views.Count; i++)
        {
            var view = views[i];

            if (i >= items.Length || items[i].Item < 0)
            {
                view.SetItem(default, null, _data);
                view.Highlight = null;
                continue;
            }

            var desc = _data?.GetObject((ushort)items[i].Item);
            view.SetItem((_textures?.Resolve(desc?.Texture) ?? default).Still, desc, _data);

            bool onTable = offered != null && i < offered.Length && offered[i];

            // Your own first four slots are what you are wearing, and the server will not move them
            // whatever the flags say -- so they are inert here rather than merely unflagged.
            bool tradeable = items[i].Tradeable && (!mine || i >= Trading.FirstTradeableSlot);

            view.Highlight = onTable ? SlotHighlight.Offered
                : tradeable ? null
                : SlotHighlight.Untradeable;
        }
    }
}

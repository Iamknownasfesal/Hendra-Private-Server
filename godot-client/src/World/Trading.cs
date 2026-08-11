using System;
using System.Linq;
using Hendra.Data;
using Hendra.Net;
using Hendra.Net.Packets;

namespace Hendra.World;

/// <summary>
/// An open trade: what each side is offering, and whether they have accepted.
/// </summary>
public sealed class Trading
{
    /// <summary>
    /// Slots below this index are equipment and are never traded, whatever the flags say.
    /// </summary>
    /// <remarks>
    /// The server enforces this when it performs the exchange — it only moves slots from index four
    /// upward — so offering a worn item would silently do nothing rather than fail.
    /// </remarks>
    public const int FirstTradeableSlot = 4;

    private readonly GameSession _session;

    public Trading(GameSession session)
    {
        _session = session;
    }

    public bool IsOpen { get; private set; }

    /// <summary>The other party's name.</summary>
    public string PartnerName { get; private set; } = string.Empty;

    /// <summary>Our own items, and whether each may be traded at all.</summary>
    public TradeItem[] MyItems { get; private set; } = Array.Empty<TradeItem>();

    public TradeItem[] TheirItems { get; private set; } = Array.Empty<TradeItem>();

    /// <summary>Which of our slots are on the table.</summary>
    public bool[] MyOffer { get; private set; } = Array.Empty<bool>();

    public bool[] TheirOffer { get; private set; } = Array.Empty<bool>();

    public bool IAccepted { get; private set; }

    public bool TheyAccepted { get; private set; }

    /// <summary>Someone has asked to trade with us.</summary>
    public event Action<string> Requested;

    /// <summary>The trade opened, changed, or someone accepted. Redraw the panel.</summary>
    public event Action Changed;

    /// <summary>The trade ended, with something to tell the player.</summary>
    public event Action<string> Ended;

    public void Handle(ServerPacket packet)
    {
        switch (packet)
        {
            case TradeRequestedPacket requested:
                Requested?.Invoke(requested.Name);
                break;

            case TradeStartPacket start:
                Open(start);
                break;

            case TradeChangedPacket changed:
                TheirOffer = changed.Offer;

                // Any change to either offer clears both acceptances, which is what stops an item
                // being swapped out from under someone who has already agreed.
                IAccepted = false;
                TheyAccepted = false;
                Changed?.Invoke();
                break;

            case TradeAcceptedPacket accepted:
                MyOffer = accepted.MyOffer;
                TheirOffer = accepted.YourOffer;
                TheyAccepted = true;
                Changed?.Invoke();
                break;

            case TradeDonePacket done:
                Close();
                Ended?.Invoke(done.Code == TradeDonePacket.Successful ? "Trade complete." : "Trade cancelled.");
                break;
        }
    }

    private void Open(TradeStartPacket start)
    {
        IsOpen = true;
        PartnerName = start.YourName ?? string.Empty;
        MyItems = start.MyItems;
        TheirItems = start.YourItems;
        MyOffer = new bool[MyItems.Length];
        TheirOffer = new bool[TheirItems.Length];
        IAccepted = false;
        TheyAccepted = false;
        Changed?.Invoke();
    }

    private void Close()
    {
        IsOpen = false;
        PartnerName = string.Empty;
        MyItems = Array.Empty<TradeItem>();
        TheirItems = Array.Empty<TradeItem>();
        MyOffer = Array.Empty<bool>();
        TheirOffer = Array.Empty<bool>();
        IAccepted = false;
        TheyAccepted = false;
    }

    /// <summary>Asks a player, by name, to trade.</summary>
    public void Request(string playerName) =>
        _session.Send(new RequestTradePacket { Name = playerName });

    /// <summary>
    /// Adds or removes one of our slots from the offer.
    /// </summary>
    /// <remarks>
    /// Untradeable and equipment slots are refused here rather than sent: flagging an empty slot
    /// makes the server dereference a null item and drop the connection.
    /// </remarks>
    public void ToggleOffer(int slot)
    {
        if (!IsOpen || slot < 0 || slot >= MyOffer.Length)
            return;

        if (slot < FirstTradeableSlot || !MyItems[slot].Tradeable || MyItems[slot].Item < 0)
            return;

        MyOffer[slot] = !MyOffer[slot];
        IAccepted = false;
        TheyAccepted = false;

        _session.Send(new ChangeTradePacket { Offer = MyOffer });
        Changed?.Invoke();
    }

    /// <summary>
    /// Accepts the trade as it currently stands.
    /// </summary>
    /// <remarks>
    /// The partner's offer is echoed back exactly as we last saw it. The server compares it against
    /// what it holds and ignores the packet if they differ — that comparison is the whole defence
    /// against an item being swapped out between seeing an offer and agreeing to it.
    /// </remarks>
    public void Accept()
    {
        if (!IsOpen)
            return;

        IAccepted = true;
        _session.Send(new AcceptTradePacket { MyOffer = MyOffer, YourOffer = TheirOffer });
        Changed?.Invoke();
    }

    public void Cancel()
    {
        if (!IsOpen)
            return;

        _session.Send(new CancelTradePacket());
    }

    /// <summary>How many of our slots are currently offered. For the panel's summary line.</summary>
    public int OfferedCount => MyOffer.Count(offered => offered);
}

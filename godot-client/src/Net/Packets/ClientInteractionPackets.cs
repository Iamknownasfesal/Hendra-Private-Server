using System;
using System.Collections.Generic;
using Hendra.Data;

namespace Hendra.Net.Packets;

/// <summary>How an item activation was triggered. Matches the AS3 UseType constants.</summary>
public enum ItemUseType : byte
{
    Default = 0,
    StartUse = 1,
    EndUse = 2,
}

/// <summary>
/// Consumes or activates an item, including ability shots.
/// </summary>
/// <remarks>
/// The server rejects this (with an InvResult failure) when the target entity is more than 3 tiles
/// away, when MP is short, or when the slot type does not match and the item is not a consumable.
/// Consumables run through a database transaction, so their effect is applied asynchronously and
/// its ordering relative to other packets is not guaranteed.
/// </remarks>
public sealed class UseItemPacket : ClientPacket
{
    public override PacketId Id => PacketId.UseItem;

    public int Time;
    public SlotObject Slot;
    public WorldPos ItemUsePos;
    public ItemUseType UseType = ItemUseType.Default;

    public override void Write(NetWriter w)
    {
        w.Write(Time);
        Slot.Write(w);
        ItemUsePos.Write(w);
        w.Write((sbyte)UseType);
    }
}

/// <summary>Enters a portal. Success comes back as a Reconnect.</summary>
public sealed class UsePortalPacket : ClientPacket
{
    public override PacketId Id => PacketId.UsePortal;

    public int ObjectId;

    public override void Write(NetWriter w) => w.Write(ObjectId);
}

/// <summary>
/// Moves an item between two slots, possibly across entities (looting a bag, stocking a vault).
/// </summary>
/// <remarks>
/// The server requires the two entities to be within one tile of each other, refuses while a trade
/// is open, and rejects slots at index 16 or above unless we have a backpack. It replies with
/// InvResult — 0 for success, 1 for failure — and on failure also re-sends the true contents of
/// both slots, so the optimistic local swap the AS3 client performs is safe to keep.
///
/// It also dereferences our player without a null check, so this must never be sent before
/// CreateSuccess has arrived.
/// </remarks>
public sealed class InvSwapPacket : ClientPacket
{
    public override PacketId Id => PacketId.InvSwap;

    public int Time;
    public WorldPos Position;
    public SlotObject Slot1;
    public SlotObject Slot2;

    public override void Write(NetWriter w)
    {
        w.Write(Time);
        Position.Write(w);
        Slot1.Write(w);
        Slot2.Write(w);
    }
}

/// <summary>
/// Drops an item on the ground. Silently ignored in the Nexus, and every dropped item lands in a
/// soulbound bag on this server build.
/// </summary>
public sealed class InvDropPacket : ClientPacket
{
    public override PacketId Id => PacketId.InvDrop;

    public SlotObject Slot;

    public override void Write(NetWriter w) => Slot.Write(w);
}

/// <summary>
/// Teleports to another player.
/// </summary>
/// <remarks>
/// Failures come back as chat errors rather than disconnects. On success the server broadcasts a
/// Goto to *every* player in the world, so everyone present owes it a GotoAck.
/// </remarks>
public sealed class TeleportPacket : ClientPacket
{
    public override PacketId Id => PacketId.Teleport;

    public int ObjectId;

    public override void Write(NetWriter w) => w.Write(ObjectId);
}

/// <summary>
/// Sends a chat line or a slash command.
/// </summary>
/// <remarks>
/// Must never be empty: the server indexes character zero to test for a leading '/' before checking
/// the length, so an empty string throws in the handler and drops the connection. Anything over 512
/// characters is discarded, and the server applies its own spam heuristics on top.
/// </remarks>
public sealed class PlayerTextPacket : ClientPacket
{
    public override PacketId Id => PacketId.PlayerText;

    public string Text = string.Empty;

    public override void Write(NetWriter w) => w.WriteUtf(Text);
}

/// <summary>Buys from a merchant or vendor. The quantity field is parsed but ignored by the server.</summary>
public sealed class BuyPacket : ClientPacket
{
    public override PacketId Id => PacketId.Buy;

    public int ObjectId;
    public int Quantity = 1;

    public override void Write(NetWriter w)
    {
        w.Write(ObjectId);
        w.Write(Quantity);
    }
}

/// <summary>Asks the server to re-read and re-send our credit balance.</summary>
public sealed class CheckCreditsPacket : ClientPacket
{
    public override PacketId Id => PacketId.CheckCredits;

    public override void Write(NetWriter w)
    {
        // No body.
    }
}

/// <summary>
/// Claims an account name. 3-10 letters, must be unique; renaming an already-named account costs
/// 5000 fame. The outcome arrives as NameResult.
/// </summary>
public sealed class ChooseNamePacket : ClientPacket
{
    public override PacketId Id => PacketId.ChooseName;

    public string Name = string.Empty;

    public override void Write(NetWriter w) => w.WriteUtf(Name);
}

/// <summary>Which of the two per-account player lists an edit applies to.</summary>
/// <remarks>
/// The numbers are the server's: <c>ConnectManager</c> sends list 0 as the lock list and list 1 as
/// the ignore list, in that order, immediately after MapInfo.
/// </remarks>
public enum AccountListId
{
    /// <summary>Players who may not teleport to us.</summary>
    LockedOut = 0,

    /// <summary>Ignored players, whose chat is suppressed.</summary>
    Ignored = 1,
}

/// <summary>Adds or removes a player from the lock or ignore list.</summary>
public sealed class EditAccountListPacket : ClientPacket
{
    public override PacketId Id => PacketId.EditAccountList;

    public AccountListId ListId;
    public bool Add;
    public int ObjectId;

    public override void Write(NetWriter w)
    {
        w.Write((int)ListId);
        w.Write(Add);
        w.Write(ObjectId);
    }
}

/// <summary>Asks for the name/description/creator of a dungeon key, for its tooltip.</summary>
public sealed class KeyInfoRequestPacket : ClientPacket
{
    public override PacketId Id => PacketId.KeyInfoRequest;

    public int ItemType;

    public override void Write(NetWriter w) => w.Write(ItemType);
}

/// <summary>
/// Requests transfer to the daily quest room.
/// </summary>
/// <remarks>
/// This id is not in the server's enum and no handler is registered for it, so it is dropped on
/// arrival. Kept because the AS3 client sends it and a future server build may implement it.
/// </remarks>
public sealed class GoToQuestRoomPacket : ClientPacket
{
    public override PacketId Id => PacketId.GoToQuestRoom;

    public override void Write(NetWriter w)
    {
        // No body.
    }
}

/// <summary>Opens the prestige interface.</summary>
public sealed class PrestigeRequestPacket : ClientPacket
{
    public override PacketId Id => PacketId.PrestigeRequest;

    public override void Write(NetWriter w)
    {
        // No body.
    }
}

/// <summary>Buys a prestige reward.</summary>
public sealed class PrestigeBuyRequestPacket : ClientPacket
{
    public override PacketId Id => PacketId.PrestigeBuyRequest;

    public int BuyId;

    public override void Write(NetWriter w) => w.Write(BuyId);
}

/// <summary>Which market operation a MarketCommand carries.</summary>
public enum MarketCommandId : byte
{
    RequestMyItems = 0,
    AddOffer = 1,
    RemoveOffer = 2,
}

/// <summary>
/// A player-market operation. The body shape depends on <see cref="Command"/>, which is why this is
/// one packet rather than three.
/// </summary>
public sealed class MarketCommandPacket : ClientPacket
{
    public override PacketId Id => PacketId.MarketCommand;

    public MarketCommandId Command;

    /// <summary>Only read when <see cref="Command"/> is <see cref="MarketCommandId.AddOffer"/>.</summary>
    public List<MarketOffer> Offers = new();

    /// <summary>Only read when <see cref="Command"/> is <see cref="MarketCommandId.RemoveOffer"/>.</summary>
    public uint OfferId;

    public override void Write(NetWriter w)
    {
        w.Write((sbyte)Command);
        switch (Command)
        {
            case MarketCommandId.RequestMyItems:
                break;

            case MarketCommandId.AddOffer:
                w.Write(Offers.Count);
                foreach (var offer in Offers)
                    offer.Write(w);
                break;

            case MarketCommandId.RemoveOffer:
                w.Write(OfferId);
                break;

            default:
                throw new ArgumentOutOfRangeException(nameof(Command), Command, "Unknown market command.");
        }
    }
}

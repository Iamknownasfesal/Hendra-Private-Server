using System;
using System.Collections.Generic;

namespace Hendra.Net.Packets;

/// <summary>Invites another player to trade, by name.</summary>
public sealed class RequestTradePacket : ClientPacket
{
    public override PacketId Id => PacketId.RequestTrade;

    public string Name = string.Empty;

    public override void Write(NetWriter w) => w.WriteUtf(Name);
}

/// <summary>
/// Updates which of our slots are on the table. One flag per inventory slot.
/// </summary>
/// <remarks>
/// Only meaningful while a trade is open — the server returns immediately otherwise. Flagging an
/// empty slot throws in the server's soulbound check and drops the connection, so only ever set
/// flags for slots that actually hold an item. Any change resets both sides' acceptance.
/// </remarks>
public sealed class ChangeTradePacket : ClientPacket
{
    public override PacketId Id => PacketId.ChangeTrade;

    public bool[] Offer = Array.Empty<bool>();

    public override void Write(NetWriter w)
    {
        w.Write((short)Offer.Length);
        foreach (bool included in Offer)
            w.Write(included);
    }
}

/// <summary>
/// Accepts the trade as it currently stands.
/// </summary>
/// <remarks>
/// <see cref="YourOffer"/> must match the partner's current offer exactly, or the server silently
/// ignores the packet — that comparison is what stops a partner swapping items after you accept.
/// Only slots from index 4 upward are ever exchanged; 0-3 are equipment.
/// </remarks>
public sealed class AcceptTradePacket : ClientPacket
{
    public override PacketId Id => PacketId.AcceptTrade;

    public bool[] MyOffer = Array.Empty<bool>();
    public bool[] YourOffer = Array.Empty<bool>();

    public override void Write(NetWriter w)
    {
        w.Write((short)MyOffer.Length);
        foreach (bool included in MyOffer)
            w.Write(included);

        w.Write((short)YourOffer.Length);
        foreach (bool included in YourOffer)
            w.Write(included);
    }
}

/// <summary>Cancels an open trade.</summary>
public sealed class CancelTradePacket : ClientPacket
{
    public override PacketId Id => PacketId.CancelTrade;

    public override void Write(NetWriter w)
    {
        // No body.
    }
}

/// <summary>Founds a guild. The outcome arrives as GuildResult.</summary>
public sealed class CreateGuildPacket : ClientPacket
{
    public override PacketId Id => PacketId.CreateGuild;

    public string Name = string.Empty;

    public override void Write(NetWriter w) => w.WriteUtf(Name);
}

/// <summary>Invites a player to our guild, by name.</summary>
public sealed class GuildInvitePacket : ClientPacket
{
    public override PacketId Id => PacketId.GuildInvite;

    public string Name = string.Empty;

    public override void Write(NetWriter w) => w.WriteUtf(Name);
}

/// <summary>Removes a member from our guild — or ourselves, to leave.</summary>
public sealed class GuildRemovePacket : ClientPacket
{
    public override PacketId Id => PacketId.GuildRemove;

    public string Name = string.Empty;

    public override void Write(NetWriter w) => w.WriteUtf(Name);
}

/// <summary>Accepts a guild invitation.</summary>
public sealed class JoinGuildPacket : ClientPacket
{
    public override PacketId Id => PacketId.JoinGuild;

    public string GuildName = string.Empty;

    public override void Write(NetWriter w) => w.WriteUtf(GuildName);
}

/// <summary>Promotes or demotes a guild member.</summary>
public sealed class ChangeGuildRankPacket : ClientPacket
{
    public override PacketId Id => PacketId.ChangeGuildRank;

    public string Name = string.Empty;
    public int GuildRank;

    public override void Write(NetWriter w)
    {
        w.WriteUtf(Name);
        w.Write(GuildRank);
    }
}

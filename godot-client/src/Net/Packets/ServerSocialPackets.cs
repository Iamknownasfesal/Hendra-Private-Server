using System;
using Hendra.Data;

namespace Hendra.Net.Packets;

/// <summary>
/// A chat line.
/// </summary>
/// <remarks>
/// <see cref="Recipient"/> starting with '#' or '*' marks a system channel, which bypasses the
/// client's own chat filters. <see cref="CleanText"/> is the profanity-masked variant; which of the
/// two is displayed depends on the local filter setting, and we never censor our own messages.
/// <see cref="Text"/> may be a JSON localisation blob rather than a literal.
/// </remarks>
public sealed class TextPacket : ServerPacket
{
    public override PacketId Id => PacketId.Text;

    public string Name;
    public int ObjectId;
    public int NumStars;
    public int Admin;

    /// <summary>Seconds to keep the speech balloon up. Zero means no balloon.</summary>
    public byte BubbleTime;

    public string Recipient;
    public string Text;
    public string CleanText;
    public int NameColor;
    public int TextColor;

    public override void Read(ref NetReader r)
    {
        Name = r.ReadUtf();
        ObjectId = r.ReadInt32();
        NumStars = r.ReadInt32();
        Admin = r.ReadInt32();
        BubbleTime = r.ReadByte();
        Recipient = r.ReadUtf();
        Text = r.ReadUtf();
        CleanText = r.ReadUtf();
        NameColor = r.ReadInt32();
        TextColor = r.ReadInt32();
    }
}

/// <summary>
/// The starred or ignored player list. Sent twice during the handshake to seed both.
/// </summary>
/// <remarks>
/// <see cref="LockAction"/> is 1 to add the listed ids, 0 to remove them, and -1 to replace the
/// list wholesale.
/// </remarks>
public sealed class AccountListPacket : ServerPacket
{
    public override PacketId Id => PacketId.AccountList;

    public AccountListId ListId;
    public string[] AccountIds = Array.Empty<string>();
    public int LockAction;

    public override void Read(ref NetReader r)
    {
        ListId = (AccountListId)r.ReadInt32();

        AccountIds = new string[r.ReadUInt16()];
        for (int i = 0; i < AccountIds.Length; i++)
            AccountIds[i] = r.ReadUtf();

        LockAction = r.ReadInt32();
    }
}

/// <summary>
/// The outcome of an inventory operation: 0 for success, anything else for failure. On failure the
/// server also re-sends the true contents of the affected slots, so an optimistic local swap can
/// simply be left to be corrected.
/// </summary>
public sealed class InvResultPacket : ServerPacket
{
    public override PacketId Id => PacketId.InvResult;

    public int Result;

    public bool Success => Result == 0;

    public override void Read(ref NetReader r) => Result = r.ReadInt32();
}

/// <summary>Known values of <see cref="BuyResultPacket.Result"/>.</summary>
public static class BuyResultCode
{
    public const int Unknown = -1;
    public const int Success = 0;
    public const int InvalidCharacter = 1;
    public const int ItemNotFound = 2;
    public const int NotEnoughGold = 3;
    public const int InventoryFull = 4;
    public const int TooLowRank = 5;
    public const int NotEnoughFame = 6;
    public const int PetFeedSuccess = 7;
}

/// <summary>
/// The outcome of a purchase. <see cref="ResultString"/> is a JSON localisation blob except when
/// <see cref="Result"/> is <see cref="BuyResultCode.Unknown"/>, where it is a literal.
/// </summary>
public sealed class BuyResultPacket : ServerPacket
{
    public override PacketId Id => PacketId.BuyResult;

    public int Result;
    public string ResultString;

    public override void Read(ref NetReader r)
    {
        Result = r.ReadInt32();
        ResultString = r.ReadUtf();
    }
}

/// <summary>The outcome of claiming an account name.</summary>
public sealed class NameResultPacket : ServerPacket
{
    public override PacketId Id => PacketId.NameResult;

    public bool Success;
    public string ErrorText;

    public override void Read(ref NetReader r)
    {
        Success = r.ReadBoolean();
        ErrorText = r.ReadUtf();
    }
}

/// <summary>The outcome of a guild operation. The message is a JSON localisation blob.</summary>
public sealed class GuildResultPacket : ServerPacket
{
    public override PacketId Id => PacketId.GuildResult;

    public bool Success;
    public string LineBuilderJson;

    public override void Read(ref NetReader r)
    {
        Success = r.ReadBoolean();
        LineBuilderJson = r.ReadUtf();
    }
}

/// <summary>Someone has invited us to their guild.</summary>
public sealed class InvitedToGuildPacket : ServerPacket
{
    public override PacketId Id => PacketId.InvitedToGuild;

    public string Name;
    public string GuildName;

    public override void Read(ref NetReader r)
    {
        Name = r.ReadUtf();
        GuildName = r.ReadUtf();
    }
}

/// <summary>Someone wants to trade with us.</summary>
public sealed class TradeRequestedPacket : ServerPacket
{
    public override PacketId Id => PacketId.TradeRequested;

    public string Name;

    public override void Read(ref NetReader r) => Name = r.ReadUtf();
}

/// <summary>A trade has opened. Carries both inventories and which slots are tradeable.</summary>
public sealed class TradeStartPacket : ServerPacket
{
    public override PacketId Id => PacketId.TradeStart;

    public TradeItem[] MyItems = Array.Empty<TradeItem>();
    public string YourName;
    public TradeItem[] YourItems = Array.Empty<TradeItem>();

    public override void Read(ref NetReader r)
    {
        MyItems = new TradeItem[r.ReadUInt16()];
        for (int i = 0; i < MyItems.Length; i++)
            MyItems[i] = TradeItem.Read(ref r);

        YourName = r.ReadUtf();

        YourItems = new TradeItem[r.ReadUInt16()];
        for (int i = 0; i < YourItems.Length; i++)
            YourItems[i] = TradeItem.Read(ref r);
    }
}

/// <summary>The partner changed their offer. Both sides' acceptance is reset.</summary>
public sealed class TradeChangedPacket : ServerPacket
{
    public override PacketId Id => PacketId.TradeChanged;

    public bool[] Offer = Array.Empty<bool>();

    public override void Read(ref NetReader r)
    {
        Offer = new bool[r.ReadUInt16()];
        for (int i = 0; i < Offer.Length; i++)
            Offer[i] = r.ReadBoolean();
    }
}

/// <summary>The partner accepted. Echoes both offers as the server understands them.</summary>
public sealed class TradeAcceptedPacket : ServerPacket
{
    public override PacketId Id => PacketId.TradeAccepted;

    public bool[] MyOffer = Array.Empty<bool>();
    public bool[] YourOffer = Array.Empty<bool>();

    public override void Read(ref NetReader r)
    {
        MyOffer = new bool[r.ReadUInt16()];
        for (int i = 0; i < MyOffer.Length; i++)
            MyOffer[i] = r.ReadBoolean();

        YourOffer = new bool[r.ReadUInt16()];
        for (int i = 0; i < YourOffer.Length; i++)
            YourOffer[i] = r.ReadBoolean();
    }
}

/// <summary>The trade ended. Code 0 succeeded, 1 was cancelled.</summary>
public sealed class TradeDonePacket : ServerPacket
{
    public override PacketId Id => PacketId.TradeDone;

    public const int Successful = 0;
    public const int PlayerCanceled = 1;

    public int Code;

    /// <summary>A JSON localisation blob.</summary>
    public string Description;

    public override void Read(ref NetReader r)
    {
        Code = r.ReadInt32();
        Description = r.ReadUtf();
    }
}

/// <summary>Which market operation a MarketResult is answering.</summary>
public enum MarketResultId : byte
{
    Error = 0,
    Success = 1,
    RequestResult = 2,
}

/// <summary>
/// The outcome of a market operation. The body shape depends on <see cref="Command"/>.
/// </summary>
/// <remarks>
/// The AS3 client never registered a handler for this id, so receiving one tore down its
/// connection. Handled properly here.
/// </remarks>
public sealed class MarketResultPacket : ServerPacket
{
    public override PacketId Id => PacketId.MarketResult;

    public MarketResultId Command;
    public string Message;
    public PlayerShopItem[] Items = Array.Empty<PlayerShopItem>();

    public override void Read(ref NetReader r)
    {
        Command = (MarketResultId)r.ReadByte();
        switch (Command)
        {
            case MarketResultId.Error:
            case MarketResultId.Success:
                Message = r.ReadUtf();
                break;

            case MarketResultId.RequestResult:
                Items = new PlayerShopItem[r.ReadInt32()];
                for (int i = 0; i < Items.Length; i++)
                    Items[i] = PlayerShopItem.Read(ref r);
                break;

            default:
                throw new PacketFormatException($"Unknown market result command {(byte)Command}.");
        }
    }
}

/// <summary>A skin has been unlocked. Clears the matching locked-slot markers.</summary>
public sealed class ReskinUnlockPacket : ServerPacket
{
    public override PacketId Id => PacketId.ReskinUnlock;

    public int SkinId;

    public override void Read(ref NetReader r) => SkinId = r.ReadInt32();
}

/// <summary>
/// Dungeon key metadata for a tooltip.
/// </summary>
/// <remarks>
/// The three fields use .NET's <c>BinaryWriter</c> string encoding, not the protocol's: the server
/// writes them with <c>wtr.Write(...)</c> rather than <c>wtr.WriteUTF(...)</c>, and
/// <c>NWriter</c> does not override <c>Write(string)</c>. The AS3 client read them as ordinary
/// protocol strings and therefore misparsed this packet. See
/// <see cref="NetReader.ReadDotNetString"/>.
/// </remarks>
public sealed class KeyInfoResponsePacket : ServerPacket
{
    public override PacketId Id => PacketId.KeyInfoResponse;

    public string Name;
    public string Description;
    public string Creator;

    public override void Read(ref NetReader r)
    {
        Name = r.ReadDotNetString();
        Description = r.ReadDotNetString();
        Creator = r.ReadDotNetString();
    }
}

using System;
using Hendra.Net.Packets;

namespace Hendra.Net;

/// <summary>
/// Maps an incoming packet id onto the type that decodes it.
/// </summary>
/// <remarks>
/// Deliberately an explicit switch rather than the reflection-over-the-assembly scan the server
/// does at static-init time. It costs nothing at runtime, it makes "which packets do we actually
/// handle?" answerable by reading the file, and an unknown id becomes a visible decision rather
/// than a silent dictionary miss.
///
/// That last point matters: the AS3 client simply left MarketResult (100) and ServerFull (110) out
/// of its table, and receiving either one tore down the connection. Both are handled here.
/// </remarks>
public static class PacketRegistry
{
    /// <summary>
    /// Creates an empty packet instance for <paramref name="id"/>, or null if the id is one we do
    /// not decode. A null result is safe to skip — the body has already been decrypted, so the RC4
    /// keystream stays in step either way.
    /// </summary>
    public static ServerPacket Create(PacketId id) => id switch
    {
        // Session and world state
        PacketId.Failure => new FailurePacket(),
        PacketId.MapInfo => new MapInfoPacket(),
        PacketId.CreateSuccess => new CreateSuccessPacket(),
        PacketId.Update => new UpdatePacket(),
        PacketId.NewTick => new NewTickPacket(),
        PacketId.Goto => new GotoPacket(),
        PacketId.Ping => new PingPacket(),
        PacketId.QueuePing => new QueuePingPacket(),
        PacketId.ServerFull => new ServerFullPacket(),
        PacketId.Reconnect => new ReconnectPacket(),
        PacketId.Death => new DeathPacket(),
        PacketId.QuestObjId => new QuestObjIdPacket(),
        PacketId.SetFocus => new SetFocusPacket(),
        PacketId.SwitchMusic => new SwitchMusicPacket(),
        PacketId.ClientStat => new ClientStatPacket(),
        PacketId.File => new FilePacket(),
        PacketId.Pic => new PicPacket(),
        PacketId.VerifyEmail => new VerifyEmailPacket(),
        PacketId.PasswordPrompt => new PasswordPromptPacket(),

        // Combat and effects
        PacketId.Damage => new DamagePacket(),
        PacketId.Aoe => new AoePacket(),
        PacketId.EnemyShoot => new EnemyShootPacket(),
        PacketId.AllyShoot => new AllyShootPacket(),
        PacketId.ServerPlayerShoot => new ServerPlayerShootPacket(),
        PacketId.ShowEffect => new ShowEffectPacket(),
        PacketId.PlaySound => new PlaySoundPacket(),
        PacketId.Notification => new NotificationPacket(),
        PacketId.GlobalNotification => new GlobalNotificationPacket(),

        // Inventory, chat and social
        PacketId.Text => new TextPacket(),
        PacketId.AccountList => new AccountListPacket(),
        PacketId.InvResult => new InvResultPacket(),
        PacketId.VaultUpdate => new VaultUpdatePacket(),
        PacketId.BuyResult => new BuyResultPacket(),
        PacketId.NameResult => new NameResultPacket(),
        PacketId.GuildResult => new GuildResultPacket(),
        PacketId.InvitedToGuild => new InvitedToGuildPacket(),
        PacketId.TradeRequested => new TradeRequestedPacket(),
        PacketId.TradeStart => new TradeStartPacket(),
        PacketId.TradeChanged => new TradeChangedPacket(),
        PacketId.TradeAccepted => new TradeAcceptedPacket(),
        PacketId.TradeDone => new TradeDonePacket(),
        PacketId.MarketResult => new MarketResultPacket(),
        PacketId.ReskinUnlock => new ReskinUnlockPacket(),
        PacketId.KeyInfoResponse => new KeyInfoResponsePacket(),

        _ => null,
    };

    /// <summary>Whether <paramref name="id"/> is one the server ever sends to us.</summary>
    public static bool IsKnownServerPacket(PacketId id) => Create(id) != null;
}

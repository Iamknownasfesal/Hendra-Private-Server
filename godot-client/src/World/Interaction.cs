using System;
using Hendra.Resources;

namespace Hendra.World;

/// <summary>What kind of thing the player is standing next to.</summary>
public enum InteractionKind
{
    None,

    /// <summary>A portal to another world. Entering it produces a Reconnect.</summary>
    Portal,

    /// <summary>A loot bag, vault chest or similar. Items move in and out with InvSwap.</summary>
    Container,

    /// <summary>A vendor. Buying is a separate packet.</summary>
    Merchant,

    /// <summary>
    /// The vault, which is one object opening one panel over every chest the account owns.
    /// </summary>
    /// <remarks>
    /// Not a Container: it holds nothing and its contents never arrive as equipment on an entity.
    /// The panel is fed by VaultUpdate and the object is only the place you have to be standing.
    /// </remarks>
    Vault,
}

/// <summary>Whatever the player can currently act on, and how.</summary>
public readonly struct Interactable
{
    public readonly Entity Entity;
    public readonly InteractionKind Kind;
    public readonly string Label;

    public Interactable(Entity entity, InteractionKind kind, string label)
    {
        Entity = entity;
        Kind = kind;
        Label = label;
    }

    public bool Exists => Entity != null && Kind != InteractionKind.None;
}

/// <summary>
/// Finds the thing the player is standing next to.
/// </summary>
/// <remarks>
/// <para>
/// The reach is one tile, which is the original's MAXIMUM_INTERACTION_DISTANCE and is deliberately
/// tight: in a crowded Nexus the portals sit close enough together that a longer reach would make
/// it ambiguous which one you meant.
/// </para>
/// <para>
/// The original ran this scan on a hundred-millisecond timer over every object in the world. It is
/// cheap enough to do per frame over the visible set, but the timer is kept because the answer
/// drives a piece of UI, and re-deciding it sixty times a second makes the prompt flicker between
/// two equidistant candidates.
/// </para>
/// </remarks>
public sealed class Interaction
{
    /// <summary>Reach, in tiles.</summary>
    public const float Range = 1f;

    /// <summary>How often the nearest candidate is re-evaluated, in milliseconds.</summary>
    private const int RescanIntervalMs = 100;

    private readonly GameMap _map;
    private int _nextScanMs;

    public Interaction(GameMap map)
    {
        _map = map;
    }

    /// <summary>The current candidate. Updated by <see cref="Update"/>.</summary>
    public Interactable Current { get; private set; }

    public void Update(int nowMs)
    {
        if (nowMs < _nextScanMs)
            return;

        _nextScanMs = nowMs + RescanIntervalMs;
        Current = FindNearest();
    }

    private Interactable FindNearest()
    {
        var player = _map.Player;
        if (player == null)
            return default;

        Entity best = null;
        var bestKind = InteractionKind.None;
        float bestDistance = Range * Range;

        foreach (var entity in _map.Entities)
        {
            var kind = KindOf(entity);
            if (kind == InteractionKind.None)
                continue;

            float dx = entity.X - player.X;
            float dy = entity.Y - player.Y;
            float distance = dx * dx + dy * dy;

            if (distance > bestDistance)
                continue;

            bestDistance = distance;
            best = entity;
            bestKind = kind;
        }

        return best == null ? default : new Interactable(best, bestKind, LabelFor(best, bestKind));
    }

    private static InteractionKind KindOf(Entity entity)
    {
        var desc = entity?.Desc;
        if (desc == null || entity.Dead)
            return InteractionKind.None;

        return desc.Class switch
        {
            "Portal" or "GuildHallPortal" => InteractionKind.Portal,

            // Only the chests that actually hold something. A ClosedVaultChest is a SellableObject
            // in the original -- a chest you buy, not one you open -- and a ClosedGiftChest is a
            // gift waiting to be claimed. Treating either as a container put an empty eight-slot
            // grid on screen for something that has no contents to show.
            "Container" or "OneWayContainer" => InteractionKind.Container,

            "VaultAccess" => InteractionKind.Vault,

            // Bought rather than opened, and the server sends both of them the same merchandise
            // stats a merchant has, so the same panel serves.
            "Merchant" or "GuildMerchant" or "ClosedVaultChest" => InteractionKind.Merchant,

            _ => InteractionKind.None,
        };
    }

    /// <summary>
    /// What the prompt calls the thing in front of the player.
    /// </summary>
    /// <remarks>
    /// The data's name for it, not the Name stat. A vault chest's Name is how full it is -- "0/8" --
    /// which belongs over the chest, where the server means it to go, and not in a sentence reading
    /// "Open 0/8". The stat is only preferred where it is genuinely a name, which is a player's.
    /// </remarks>
    /// <summary>
    /// Puts the spaces back into an identifier: VaultChest becomes Vault Chest.
    /// </summary>
    /// <remarks>
    /// Only reached when an object has no display name, which happens more than it should: a world
    /// that ships its own XML can redefine an object without one, and the prompt then reads out the
    /// identifier the data file uses rather than the words on the panel it opens.
    /// </remarks>
    private static string Spaced(string id)
    {
        if (string.IsNullOrEmpty(id))
            return id;

        var text = new System.Text.StringBuilder(id.Length + 4);

        for (int i = 0; i < id.Length; i++)
        {
            if (i > 0 && char.IsUpper(id[i]) && !char.IsUpper(id[i - 1]))
                text.Append(' ');

            text.Append(id[i]);
        }

        return text.ToString();
    }

    private static string LabelFor(Entity entity, InteractionKind kind)
    {
        string name = entity.Desc?.DisplayId ?? Spaced(entity.Desc?.Id);
        if (string.IsNullOrEmpty(name))
            name = !string.IsNullOrEmpty(entity.Name) ? entity.Name : "it";

        return kind switch
        {
            InteractionKind.Portal => $"Enter {name}",
            InteractionKind.Container => $"Open {name}",
            InteractionKind.Vault => $"Open {name}",
            InteractionKind.Merchant => $"Buy from {name}",
            _ => name,
        };
    }
}

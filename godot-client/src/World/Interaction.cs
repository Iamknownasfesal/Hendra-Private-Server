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
            "Container" or "OneWayContainer" or "ClosedVaultChest" or "ClosedGiftChest" =>
                InteractionKind.Container,
            "Merchant" or "GuildMerchant" => InteractionKind.Merchant,
            _ => InteractionKind.None,
        };
    }

    private static string LabelFor(Entity entity, InteractionKind kind)
    {
        string name = !string.IsNullOrEmpty(entity.Name)
            ? entity.Name
            : entity.Desc?.DisplayId ?? entity.Desc?.Id ?? "it";

        return kind switch
        {
            InteractionKind.Portal => $"Enter {name}",
            InteractionKind.Container => $"Open {name}",
            InteractionKind.Merchant => $"Buy from {name}",
            _ => name,
        };
    }
}

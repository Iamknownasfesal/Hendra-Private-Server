using System.Collections.Generic;
using System.Linq;

using Hendra.Net.Packets;

namespace Hendra.World;

/// <summary>One nearby player, as the party list shows them.</summary>
public readonly struct PartyMember
{
    public readonly string Name;
    public readonly int Hp;
    public readonly int MaxHp;
    public readonly float Distance;

    /// <summary>Whether we have locked them out, so they cannot teleport to us.</summary>
    public readonly bool LockedOut;

    /// <summary>Whether we are ignoring them, so nothing they say is shown.</summary>
    public readonly bool Ignored;

    /// <summary>Their class, which is what the list draws a portrait of.</summary>
    public readonly ushort ObjectType;

    public PartyMember(
        string name, int hp, int maxHp, float distance,
        bool lockedOut, bool ignored, ushort objectType)
    {
        Name = name;
        Hp = hp;
        MaxHp = maxHp;
        Distance = distance;
        LockedOut = lockedOut;
        Ignored = ignored;
        ObjectType = objectType;
    }
}

/// <summary>
/// The nearby players, for the party list.
/// </summary>
/// <remarks>
/// <para>
/// Rebuilt on an interval rather than every frame. The original used half a second; the reason is
/// not cost but stability — the list is sorted by distance, and recomputing it sixty times a second
/// makes two players at similar range swap places continuously.
/// </para>
/// <para>
/// Both of the account's lists arrive in AccountList — one for the players who may not teleport to
/// us and one for the players we are ignoring — and each member carries whether they are on either.
/// The server enforces both regardless; carrying them here is what puts the marker beside the name,
/// which is the only way to see who has already been dealt with.
/// </para>
/// </remarks>
public sealed class Party
{
    /// <summary>The original showed six, which is about what fits beside the world.</summary>
    public const int MaxMembers = 6;

    /// <summary>Beyond this many tiles a player is not really nearby. Fifty, squared.</summary>
    private const float RangeSquared = 2500f;

    private const int RebuildIntervalMs = 500;

    private readonly GameMap _map;
    private readonly List<PartyMember> _members = new(MaxMembers);
    private readonly HashSet<string> _lockedOut = new(System.StringComparer.OrdinalIgnoreCase);
    private readonly HashSet<string> _ignored = new(System.StringComparer.OrdinalIgnoreCase);

    private int _nextRebuildMs;

    public Party(GameMap map)
    {
        _map = map;
    }

    public IReadOnlyList<PartyMember> Members => _members;

    /// <summary>Replaces one of the two lists, each of which arrives whole in an AccountList.</summary>
    public void SetList(AccountListId which, IEnumerable<string> names)
    {
        var set = which == AccountListId.Ignored ? _ignored : _lockedOut;
        set.Clear();
        foreach (string name in names)
            set.Add(name);
    }

    /// <summary>Whether this player is one we are ignoring, for anything that filters what is said.</summary>
    public bool IsIgnored(string name) => !string.IsNullOrEmpty(name) && _ignored.Contains(name);

    public void Update(int nowMs)
    {
        if (nowMs < _nextRebuildMs)
            return;

        _nextRebuildMs = nowMs + RebuildIntervalMs;
        Rebuild();
    }

    private void Rebuild()
    {
        _members.Clear();

        var self = _map.Player;
        if (self == null)
            return;

        var nearby = new List<PartyMember>();

        foreach (var entity in _map.Entities)
        {
            if (entity.Desc is not { IsPlayer: true } || ReferenceEquals(entity, self) || entity.Dead)
                continue;

            if (string.IsNullOrEmpty(entity.Name))
                continue;

            float dx = entity.X - self.X;
            float dy = entity.Y - self.Y;
            float distance = dx * dx + dy * dy;
            if (distance > RangeSquared)
                continue;

            nearby.Add(new PartyMember(
                entity.Name, entity.Hp, entity.MaxHp, distance,
                _lockedOut.Contains(entity.Name), _ignored.Contains(entity.Name),
                entity.ObjectType));
        }

        _members.AddRange(nearby
            .OrderBy(member => member.Distance)
            .Take(MaxMembers));
    }
}

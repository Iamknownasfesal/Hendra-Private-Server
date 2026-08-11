using System.Collections.Generic;
using System.Linq;

namespace Hendra.World;

/// <summary>One nearby player, as the party list shows them.</summary>
public readonly struct PartyMember
{
    public readonly string Name;
    public readonly int Hp;
    public readonly int MaxHp;
    public readonly float Distance;
    public readonly bool Starred;

    public PartyMember(string name, int hp, int maxHp, float distance, bool starred)
    {
        Name = name;
        Hp = hp;
        MaxHp = maxHp;
        Distance = distance;
        Starred = starred;
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
/// Starred players sort first regardless of distance. The starred set arrives in AccountList and is
/// keyed by account id, which is why the ids are worth carrying even though nothing else uses them.
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
    private readonly HashSet<string> _starred = new();

    private int _nextRebuildMs;

    public Party(GameMap map)
    {
        _map = map;
    }

    public IReadOnlyList<PartyMember> Members => _members;

    /// <summary>Replaces the starred set, which arrives whole in an AccountList.</summary>
    public void SetStarred(IEnumerable<string> accountIds)
    {
        _starred.Clear();
        foreach (string id in accountIds)
            _starred.Add(id);
    }

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

            nearby.Add(new PartyMember(entity.Name, entity.Hp, entity.MaxHp, distance, _starred.Contains(entity.Name)));
        }

        _members.AddRange(nearby
            .OrderByDescending(member => member.Starred)
            .ThenBy(member => member.Distance)
            .Take(MaxMembers));
    }
}

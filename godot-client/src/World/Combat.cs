using System;
using System.Collections.Generic;
using Hendra.Core;
using Hendra.Data;
using Hendra.Net;
using Hendra.Net.Packets;
using Hendra.Resources;

namespace Hendra.World;

/// <summary>
/// Firing, projectile lifetime, and reporting hits to the server.
/// </summary>
/// <remarks>
/// <para>
/// The division of labour is unusual and worth stating. The client decides almost everything: when
/// it may fire, where its bullets go, and what they hit. The server accepts hit reports without
/// checking distance or timing at all. What it does check is *rate*: a shot arriving before its
/// cooldown has elapsed is silently dropped, and the server compensates by burning one draw of the
/// shared random stream so its damage rolls stay in step with ours. So firing too fast does not
/// merely waste a shot, it desynchronises damage prediction until the next map change reseeds both
/// sides.
/// </para>
/// <para>
/// A volley is sent as one packet per projectile, all sharing a single timestamp. The server groups
/// them by that value and rejects the batch if the count disagrees with the weapon's declared
/// projectile count.
/// </para>
/// </remarks>
public sealed class Combat
{
    /// <summary>Bullet ids are a byte on the wire, and the original cycles only the low half.</summary>
    private const int BulletIdWrap = 128;

    /// <summary>Distance ahead of the shooter that a bullet appears, in tiles.</summary>
    private const float MuzzleOffset = 0.3f;

    /// <summary>
    /// Beyond this long without an acknowledged tick, our shots do no damage.
    /// </summary>
    /// <remarks>
    /// The original zeroes damage when the move-record buffer has not been cleared recently, which
    /// amounts to "we have not heard from the server in a while". It stops a lagging client from
    /// predicting kills the server will not agree with.
    /// </remarks>
    private const int StaleTickGuardMs = 600;

    private readonly GameMap _map;
    private readonly GameData _data;
    private readonly GameSession _session;
    private readonly GameClock _clock;

    private readonly List<Projectile> _projectiles = new();
    private readonly List<Projectile> _finished = new();

    private byte _nextBulletId;
    private int _nextAttackAllowedMs;

    public Combat(GameMap map, GameData data, GameSession session, GameClock clock)
    {
        _map = map;
        _data = data;
        _session = session;
        _clock = clock;
    }

    public IReadOnlyList<Projectile> Projectiles => _projectiles;

    /// <summary>The weapon currently equipped, or null if the slot is empty.</summary>
    public ObjectDesc EquippedWeapon
    {
        get
        {
            var player = _map.Player;
            if (player?.Equipment == null || player.Equipment.Length == 0)
                return null;

            int type = player.Equipment[0];
            return type < 0 ? null : _data.GetObject((ushort)type);
        }
    }

    /// <summary>Whether the cooldown has elapsed and a shot would be accepted.</summary>
    public bool CanShoot(int nowMs) =>
        _map.Player != null && !_map.Player.IsPaused && nowMs >= _nextAttackAllowedMs;

    /// <summary>
    /// Fires the equipped weapon towards <paramref name="angle"/>, if the cooldown allows.
    /// </summary>
    /// <returns>Whether a volley was fired.</returns>
    public bool TryShoot(int nowMs, float angle)
    {
        var player = _map.Player;
        var weapon = EquippedWeapon;

        if (player == null || weapon?.Projectiles == null || !CanShoot(nowMs))
            return false;

        if (!weapon.Projectiles.TryGetValue(0, out var projectileDesc))
            return false;

        _nextAttackAllowedMs = nowMs + (int)player.GetAttackPeriodMs(weapon.RateOfFire);

        FireVolley(weapon, projectileDesc, angle, nowMs, player.GetAttackMultiplier());
        return true;
    }

    /// <summary>
    /// Fires an ability's volley.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Needed because the server does not send a player their own ability shots: it creates the
    /// projectiles, then broadcasts the AllyShoot to everyone <i>except</i> the shooter. Without
    /// this the player would fire and see nothing.
    /// </para>
    /// <para>
    /// The PlayerShoot packets this sends are ignored on purpose — the handler checks whether the
    /// named item is the one in the ability slot and returns without doing anything, because the
    /// UseItem already did the work. Note that it returns <i>without</i> dropping a random, which is
    /// the detail that makes this safe: the client draws once per projectile here and the server
    /// draws once per projectile in the activation, so the two stay in step.
    /// </para>
    /// </remarks>
    public void FireAbility(ObjectDesc ability, float angle, int nowMs)
    {
        if (ability?.Projectiles == null || !ability.Projectiles.TryGetValue(0, out var desc))
            return;

        // No attack multiplier: the server passes isAbility to its damage roll, which makes the
        // multiplier exactly one however strong the character is.
        FireVolley(ability, desc, angle, nowMs, multiplier: 1f);
    }

    /// <summary>
    /// Spawns a volley, predicts its damage and tells the server about it.
    /// </summary>
    /// <param name="multiplier">Applied to each roll. One for abilities, the attack stat otherwise.</param>
    private void FireVolley(ObjectDesc item, ProjectileDesc projectileDesc, float angle, int nowMs, float multiplier)
    {
        var player = _map.Player;

        int count = Math.Max(1, item.NumProjectiles);
        float totalArc = item.ArcGap * (count - 1);
        float shotAngle = angle - totalArc / 2f;

        bool stale = _session.MoveRecords.LastClearTime >= 0 &&
                     nowMs > _session.MoveRecords.LastClearTime + StaleTickGuardMs;

        for (int i = 0; i < count; i++)
        {
            byte bulletId = NextBulletId();

            int damage = 0;
            if (!stale && _session.Random != null)
            {
                // One draw per projectile, in order. The server draws in exactly the same place, so
                // adding or skipping a call here breaks every subsequent roll.
                uint rolled = _session.Random.NextIntRange(
                    (uint)projectileDesc.MinDamage, (uint)projectileDesc.MaxDamage);
                damage = (int)(rolled * multiplier);
            }

            Spawn(
                projectileDesc,
                item.Type,
                player.ObjectId,
                bulletId,
                shotAngle,
                player.X + MathF.Cos(shotAngle) * MuzzleOffset,
                player.Y + MathF.Sin(shotAngle) * MuzzleOffset,
                nowMs,
                damage,
                damagesEnemies: true);

            _session.Send(new PlayerShootPacket
            {
                Time = nowMs,
                BulletId = bulletId,
                ContainerType = item.Type,
                StartingPos = new WorldPos(player.X, player.Y),
                Angle = shotAngle,
            });

            shotAngle += item.ArcGap;
        }

        player.SetAttack(angle, nowMs);
    }

    /// <summary>
    /// Spawns a projectile that is only there to be looked at: it hits nothing and reports nothing.
    /// </summary>
    /// <remarks>
    /// Someone else's shot. It exists on the server, where its owner's client is responsible for
    /// reporting what it hits, so joining in would double the damage reported for it.
    /// </remarks>
    public void SpawnCosmetic(
        ProjectileDesc desc,
        ushort containerType,
        int ownerId,
        byte bulletId,
        float angle,
        float startX,
        float startY,
        int nowMs)
    {
        Spawn(desc, containerType, ownerId, bulletId, angle, startX, startY, nowMs,
            damage: 0, damagesEnemies: false, cosmetic: true);
    }

    /// <summary>Spawns a projectile the server told us about, from an enemy or another player.</summary>
    public void SpawnRemote(
        ProjectileDesc desc,
        ushort containerType,
        int ownerId,
        byte bulletId,
        float angle,
        float startX,
        float startY,
        int nowMs,
        int damage,
        bool damagesPlayers)
    {
        Spawn(desc, containerType, ownerId, bulletId, angle, startX, startY, nowMs, damage,
            damagesEnemies: !damagesPlayers);
    }

    private void Spawn(
        ProjectileDesc desc,
        ushort containerType,
        int ownerId,
        byte bulletId,
        float angle,
        float startX,
        float startY,
        int nowMs,
        int damage,
        bool damagesEnemies,
        bool cosmetic = false)
    {
        var projectile = new Projectile
        {
            ObjectId = Entity.NextFakeObjectId(),
            ObjectType = containerType,
            ProjectileDesc = desc,
            ContainerType = containerType,
            OwnerId = ownerId,
            BulletId = bulletId,
            StartTimeMs = nowMs,
            StartX = startX,
            StartY = startY,
            Angle = angle,
            Damage = damage,
            DamagesEnemies = damagesEnemies,
            DamagesPlayers = !cosmetic && !damagesEnemies,
            Z = 0.5f,
        };

        // Artwork comes from whichever object the projectile names, not from the shooter.
        projectile.Desc = _data.GetObject(desc.ObjectId);

        projectile.Place(startX, startY);
        _projectiles.Add(projectile);
    }

    /// <summary>
    /// The projectile a Damage packet is talking about, if it is still in flight.
    /// </summary>
    /// <remarks>
    /// Only its bearing and speed are wanted, to throw the hit spray away from the shooter. A miss
    /// is ordinary — the projectile retires the moment it connects locally, which is usually before
    /// the server's word on the damage comes back — and callers fall back to a spray in every
    /// direction.
    /// </remarks>
    public Projectile Find(int ownerId, byte bulletId)
    {
        foreach (var projectile in _projectiles)
        {
            if (projectile.OwnerId == ownerId && projectile.BulletId == bulletId)
                return projectile;
        }

        return null;
    }

    /// <summary>Advances every projectile and reports whatever they hit.</summary>
    public void Update(int nowMs)
    {
        _finished.Clear();

        foreach (var projectile in _projectiles)
        {
            if (!projectile.Advance(nowMs, _map, out var outcome))
                _finished.Add(projectile);

            if (outcome.Ending != ProjectileEnding.Expired)
                Report(projectile, outcome, nowMs);
        }

        foreach (var projectile in _finished)
            _projectiles.Remove(projectile);
    }

    /// <summary>
    /// Tells the server what a projectile hit.
    /// </summary>
    /// <remarks>
    /// Which packet to send depends entirely on who was hit, and only two of the four do anything:
    /// PlayerHit applies damage to us, EnemyHit applies it to a monster and can mark it dead.
    /// SquareHit and OtherHit have empty handlers on this server build, and are sent for fidelity.
    /// </remarks>
    private void Report(Projectile projectile, in ProjectileOutcome outcome, int nowMs)
    {
        var player = _map.Player;
        if (player == null)
            return;

        switch (outcome.Ending)
        {
            case ProjectileEnding.HitTerrain:
                if (projectile.DamagesPlayers)
                {
                    _session.Send(new SquareHitPacket
                    {
                        Time = nowMs,
                        BulletId = projectile.BulletId,
                        ObjectId = projectile.OwnerId,
                    });
                }
                break;

            case ProjectileEnding.HitCover:
                if (projectile.DamagesPlayers)
                {
                    _session.Send(new OtherHitPacket
                    {
                        Time = nowMs,
                        BulletId = projectile.BulletId,
                        ObjectId = projectile.OwnerId,
                        TargetId = outcome.Target?.ObjectId ?? 0,
                    });
                }
                break;

            case ProjectileEnding.HitEntity:
                ReportEntityHit(projectile, outcome.Target, player, nowMs);
                break;
        }
    }

    private void ReportEntityHit(Projectile projectile, Entity target, LocalPlayer player, int nowMs)
    {
        if (target == null)
            return;

        int damage = Entity.ApplyDefense(
            projectile.Damage,
            target.Defense,
            projectile.ProjectileDesc.ArmorPiercing,
            target.Conditions);

        if (ReferenceEquals(target, player))
        {
            // We were hit. The server takes this without any validation, so it is entirely on us to
            // be honest about it.
            _session.Send(new PlayerHitPacket
            {
                BulletId = projectile.BulletId,
                ObjectId = projectile.OwnerId,
            });

            target.Hp -= damage;
            return;
        }

        if (target.Desc is { IsEnemy: true } && projectile.OwnerId == player.ObjectId)
        {
            bool killed = target.Hp <= damage;
            _session.Send(new EnemyHitPacket
            {
                Time = nowMs,
                BulletId = projectile.BulletId,
                TargetId = target.ObjectId,
                Killed = killed,
            });

            // Applied locally so health bars respond immediately; the server's Damage packet is
            // authoritative and will correct it.
            target.Hp -= damage;
            if (killed)
                target.Dead = true;
            return;
        }

        if (!projectile.ProjectileDesc.MultiHit)
        {
            _session.Send(new OtherHitPacket
            {
                Time = nowMs,
                BulletId = projectile.BulletId,
                ObjectId = projectile.OwnerId,
                TargetId = target.ObjectId,
            });
        }
    }

    /// <summary>Drops every projectile, for a map change.</summary>
    public void Clear() => _projectiles.Clear();

    private byte NextBulletId()
    {
        byte id = _nextBulletId;
        _nextBulletId = (byte)((_nextBulletId + 1) % BulletIdWrap);
        return id;
    }
}

using System;
using System.Linq;
using common.resources;
using wServer.networking.packets.outgoing;
using wServer.realm.terrain;

namespace wServer.realm.entities
{
    public partial class Player
    {
        long l;

        private void HandleOceanTrenchGround(RealmTime time)
        {
            try
            {
                // don't suffocate hidden players
                if (HasConditionEffect(ConditionEffects.Hidden)) return;

                if (time.TotalElapsedMs - l <= 100 || Owner?.Name != "OceanTrench") return;

                if (!(Owner?.StaticObjects.Where(i => i.Value.ObjectType == 0x0731).Count(i => (X - i.Value.X) * (X - i.Value.X) + (Y - i.Value.Y) * (Y - i.Value.Y) < 1) > 0))
                {
                    if (OxygenBar == 0)
                        HP -= 10;
                    else
                        OxygenBar -= 2;

                    if (HP <= 0)
                        Death("suffocation");
                }
                else
                {
                    if (OxygenBar < 100)
                        OxygenBar += 8;
                    if (OxygenBar > 100)
                        OxygenBar = 100;
                }

                l = time.TotalElapsedMs;
            }
            catch (Exception ex)
            {
                Log.Error(ex);
            }
        }

        /// <summary>How often the client rolls ground damage for itself.</summary>
        private const int GroundDamagePeriodMs = 500;

        /// <summary>How late a client's report may be before the server rolls it instead.</summary>
        /// <remarks>
        /// Twice the cadence again, because both sides must not roll: each roll steps the shared
        /// random stream, and a stream stepped once here and twice there puts every subsequent shot's
        /// predicted damage out.
        /// </remarks>
        private const int GroundDamageGraceMs = 1000;

        /// <summary>How far from the player a reported burn may be, in tiles.</summary>
        private const float GroundReportSlack = 1.5f;

        public void ForceGroundHit(RealmTime time, Position pos, int timeHit)
        {
            if (HasConditionEffect(ConditionEffects.Paused) ||
                HasConditionEffect(ConditionEffects.Invincible))
                return;

            // Reported by the client, so it can name a harmless tile while standing in fire and
            // reset the clock that would otherwise have the server roll the burn itself.
            var dx = pos.X - X;
            var dy = pos.Y - Y;
            if (dx * dx + dy * dy > GroundReportSlack * GroundReportSlack)
            {
                Strike("reporting ground damage somewhere else",
                    $"burn at {pos.X:0.0},{pos.Y:0.0} but player at {X:0.0},{Y:0.0}");
                return;
            }

            WmapTile tile = Owner.Map[(int) pos.X, (int) pos.Y];
            ObjectDesc objDesc = tile.ObjType == 0 ? null : Manager.Resources.GameData.ObjectDescs[tile.ObjType];
            TileDesc tileDesc = Manager.Resources.GameData.Tiles[tile.TileId];
            if (tileDesc.Damaging && (objDesc == null || !objDesc.ProtectFromGroundDamage))
            {
                GroundDamageTaken(time);
                ApplyGroundDamage(tileDesc, tile);
            }
        }

        /// <summary>
        /// Rolls a tile's damage and applies it.
        /// </summary>
        /// <remarks>
        /// The one place that draws for a burn, whether the client asked for it or the server gave
        /// up waiting: the draw has to happen exactly once per burn on each side.
        /// </remarks>
        private void ApplyGroundDamage(TileDesc tileDesc, WmapTile tile)
        {
            int dmg = (int)Client.Random.NextIntRange((uint)tileDesc.MinDamage, (uint)tileDesc.MaxDamage);

            HP -= dmg;

            Owner.BroadcastPacketNearby(new Damage()
            {
                TargetId = Id,
                DamageAmount = (ushort)dmg,
                Kill = HP <= 0,
            }, this, this, PacketPriority.Low);

            if (HP <= 0)
            {
                Death(tileDesc.ObjectId, tile: tile);
            }
        }
    }
}

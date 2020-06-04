﻿using System;
using System.Linq;
using common.resources;
using wServer.realm;
using wServer.realm.entities;
using Player = wServer.realm.entities.Player;

namespace wServer.logic.behaviors
{
    class RemoveObjectOnDeath : Behavior
    {
        private readonly string _objName;
        private readonly int _range;

        public RemoveObjectOnDeath(string objName, int range)
        {
            _objName = objName;
            _range = range;
        }

        protected internal override void Resolve(State parent)
        {
            parent.Death += (sender, e) =>
            {

                XmlData dat = e.Host.Manager.Resources.GameData;
                var objType = dat.IdToObjectType[_objName];

                var map = e.Host.Owner.Map;

                var w = map.Width;
                var h = map.Height;

                for (var y = 0; y < h; y++)
                    for (var x = 0; x < w; x++)
                    {
                        var tile = map[x, y];

                        if (tile.ObjType != objType)
                            continue;

                        var dx = Math.Abs(x - (int)e.Host.X);
                        var dy = Math.Abs(y - (int)e.Host.Y);

                        if (dx > _range || dy > _range)
                            continue;

                        if (tile.ObjDesc?.BlocksSight == true)
                        {
                            if (e.Host.Owner.Blocking == 3)
                                Sight.UpdateRegion(map, x, y);

                            foreach (var plr in e.Host.Owner.Players.Values
                                .Where(p => MathsUtils.DistSqr(p.X, p.Y, x, y) < Player.RadiusSqr))
                                plr.Sight.UpdateCount++;
                        }

                        tile.ObjType = 0;
                        tile.UpdateCount++;
                        map[x, y] = tile;
                    }
            };
        }
        protected override void TickCore(Entity host, RealmTime time, ref object state)
        { }
    }
}

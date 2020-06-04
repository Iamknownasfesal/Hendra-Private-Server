package com.company.assembleegameclient.ui.panels.itemgrids
{
import com.company.assembleegameclient.objects.GameObject;
import com.company.assembleegameclient.objects.Player;
import com.company.assembleegameclient.ui.panels.itemgrids.itemtiles.EquipmentTile;
import com.company.assembleegameclient.ui.panels.itemgrids.itemtiles.InventoryTile;
import com.company.assembleegameclient.ui.panels.itemgrids.itemtiles.ItemTile;
import com.company.util.ArrayIterator;
import com.company.util.IIterator;

import kabam.lib.util.VectorAS3Util;

public class EquippedGrid extends ItemGrid
{
    private const NUM_SLOTS:uint = 8;

    private var _player:Player;
    private var _tiles:Vector.<EquipmentTile>;

    public function EquippedGrid(player:Player)
    {
        this._player = player;

        super(player, player, 0);

        this.ConfigureInventory();
    }

    private function ConfigureInventory():void
    {
        var types:Vector.<int> = this._player.slotTypes_;

        this._tiles = new Vector.<EquipmentTile>(NUM_SLOTS);

        for (var i:int = 0; i < NUM_SLOTS; i++)
        {
            var equipTile:EquipmentTile = new EquipmentTile(i, this, this.interactive);
            equipTile.setType(types[i]);

            addToGrid(equipTile, 1, i);

            this._tiles[i] = equipTile;
        }
    }

    public function createInteractiveItemTileIterator():IIterator {
        return (new ArrayIterator(VectorAS3Util.toArray(this._tiles)));
    }

    override public function setItems(_arg1:Vector.<int> = null, _arg2:int = 0):void
    {
        var equips:Vector.<int> = this._player.equipment_;

        for (var i:int = 0; i < NUM_SLOTS; i++)
        {
            this._tiles[i].setItem(equips[i]);
            this._tiles[i].updateDim(this._player);
        }
    }

    public function toggleTierTags(_arg_1:Boolean) : void
    {
        var _local_2:ItemTile = null;
        for each(_local_2 in this._tiles)
        {
            _local_2.toggleTierTag(_arg_1);
        }
    }

}
}

package com.company.assembleegameclient.ui.panels.itemgrids
{
import com.company.assembleegameclient.objects.Player;
import com.company.assembleegameclient.ui.panels.itemgrids.itemtiles.InventoryTile;
import com.company.assembleegameclient.ui.panels.itemgrids.itemtiles.ItemTile;

import kabam.rotmg.core.StaticInjectorContext;

import robotlegs.bender.framework.api.ILogger;

public class InventoryGrid extends ItemGrid
{
    private static var _logger:ILogger = StaticInjectorContext.getInjector().getInstance(ILogger);

    private const NUM_SLOTS:uint = 8;

    private var _player:Player;
    private var _invOffset:int;
    private var _tiles:Vector.<InventoryTile>;

    public function InventoryGrid(player:Player, invOffset:int = 0)
    {
        this._player = player;
        this._invOffset = invOffset;

        super(player, player, invOffset);

        this.ConfigureInventory();
    }

    private function ConfigureInventory():void
    {
        this._tiles = new Vector.<InventoryTile>(NUM_SLOTS);

        for (var i:int = 0; i < NUM_SLOTS; i++)
        {
            var invTile:InventoryTile = new InventoryTile(i + this._invOffset, this, this.interactive);
            invTile.addTileNumber(i + 1);

            addToGrid(invTile, 2, i);

            this._tiles[i] = invTile;
        }
    }

    override public function setItems(_arg1:Vector.<int> = null, _arg2:int = 0):void
    {
        var isSuccess:Boolean = false;
        var equips:Vector.<int> = this._player.equipment_;

        for (var i:int = 0; i < NUM_SLOTS; i++)
            if (this._tiles[i].setItem(equips[i + this._invOffset]))
                isSuccess = true;

        if (isSuccess)
            refreshTooltip();
    }

    public function toggleTierTags(_arg_1:Boolean) : void
    {
        var _local_2:ItemTile = null;

        for each(_local_2 in this._tiles)
            _local_2.toggleTierTag(_arg_1);
    }
}
}
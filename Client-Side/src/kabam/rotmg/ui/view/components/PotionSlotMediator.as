﻿package kabam.rotmg.ui.view.components {
import com.company.assembleegameclient.map.Map;
import com.company.assembleegameclient.objects.Player;
import com.company.assembleegameclient.parameters.Parameters;
import com.company.assembleegameclient.ui.panels.itemgrids.itemtiles.InteractiveItemTile;
import com.company.assembleegameclient.util.DisplayHierarchy;

import flash.display.DisplayObject;

import kabam.rotmg.constants.ItemConstants;
import kabam.rotmg.game.model.PotionInventoryModel;
import kabam.rotmg.game.model.UseBuyPotionVO;
import kabam.rotmg.game.signals.UseBuyPotionSignal;
import kabam.rotmg.messaging.impl.GameServerConnection;
import kabam.rotmg.ui.model.HUDModel;
import kabam.rotmg.ui.model.PotionModel;
import kabam.rotmg.ui.signals.UpdateHUDSignal;

import robotlegs.bender.bundles.mvcs.Mediator;

public class PotionSlotMediator extends Mediator {

    [Inject]
    public var view:PotionSlotView;
    [Inject]
    public var hudModel:HUDModel;
    [Inject]
    public var updateHUD:UpdateHUDSignal;
    [Inject]
    public var potionInventoryModel:PotionInventoryModel;
    [Inject]
    public var useBuyPotionSignal:UseBuyPotionSignal;
    private var blockingUpdate:Boolean = false;


    override public function initialize():void {
        this.updateHUD.addOnce(this.initializeData);
        this.view.drop.add(this.onDrop);
        this.view.buyUse.add(this.onBuyUse);
        this.updateHUD.add(this.update);
    }

    override public function destroy():void {
        this.view.drop.remove(this.onDrop);
        this.view.buyUse.remove(this.onBuyUse);
        this.updateHUD.remove(this.update);
    }

    private function initializeData(_arg1:Player):void {
        var _local2:PotionModel = this.potionInventoryModel.potionModels[this.view.position];
        var _local3:int = _arg1.getPotionCount(_local2.objectId);
        this.view.setData(_local3, _local2.currentCost(_local3), _local2.available, _local2.objectId);
    }

    private function update(_arg1:Player):void {
        var _local2:PotionModel;
        var _local3:int;
        if ((((((this.view.objectType == PotionInventoryModel.HEALTH_POTION_ID)) || ((this.view.objectType == PotionInventoryModel.MAGIC_POTION_ID)))) && (!(this.blockingUpdate)))) {
            _local2 = this.potionInventoryModel.getPotionModel(this.view.objectType);
            _local3 = _arg1.getPotionCount(_local2.objectId);
            this.view.setData(_local3, _local2.currentCost(_local3), _local2.available);
        }
    }

    private function onDrop(_arg1:DisplayObject):void {
        var _local4:InteractiveItemTile;

    }

    private function onBuyUse():void {
        var _local2:UseBuyPotionVO;
        var _local1:PotionModel = this.potionInventoryModel.potionModels[this.view.position];
        if (_local1.available) {
            _local2 = new UseBuyPotionVO(_local1.objectId, UseBuyPotionVO.SHIFTCLICK);
            this.useBuyPotionSignal.dispatch(_local2);
        }
    }


}
}

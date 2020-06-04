package kabam.rotmg.game.view {
import com.company.assembleegameclient.game.GameSprite;
import com.company.assembleegameclient.game.events.ReconnectEvent;
import com.company.assembleegameclient.objects.Player;

import flash.utils.getTimer;

import kabam.rotmg.core.StaticInjectorContext;
import kabam.rotmg.core.model.MapModel;
import kabam.rotmg.core.model.PlayerModel;
import kabam.rotmg.core.signals.InvalidateDataSignal;
import kabam.rotmg.core.signals.SetScreenSignal;
import kabam.rotmg.core.signals.SetScreenWithValidDataSignal;
import kabam.rotmg.dialogs.control.AddPopupToStartupQueueSignal;
import kabam.rotmg.dialogs.control.CloseDialogsSignal;
import kabam.rotmg.dialogs.control.FlushPopupStartupQueueSignal;
import kabam.rotmg.dialogs.control.OpenDialogSignal;
import kabam.rotmg.dialogs.model.DialogsModel;
import kabam.rotmg.game.logging.LoopMonitor;
import kabam.rotmg.game.model.GameInitData;
import kabam.rotmg.game.signals.GameClosedSignal;
import kabam.rotmg.game.signals.PlayGameSignal;
import kabam.rotmg.game.signals.SetWorldInteractionSignal;
import kabam.rotmg.maploading.signals.HideMapLoadingSignal;
import kabam.rotmg.maploading.signals.ShowLoadingViewSignal;
import kabam.rotmg.ui.signals.HUDModelInitialized;
import kabam.rotmg.ui.signals.HUDSetupStarted;
import kabam.rotmg.ui.signals.UpdateHUDSignal;

import robotlegs.bender.bundles.mvcs.Mediator;

public class GameSpriteMediator extends Mediator {

    [Inject]
    public var view:GameSprite;
    [Inject]
    public var setWorldInteraction:SetWorldInteractionSignal;
    [Inject]
    public var invalidate:InvalidateDataSignal;
    [Inject]
    public var setScreenWithValidData:SetScreenWithValidDataSignal;
    [Inject]
    public var setScreen:SetScreenSignal;
    [Inject]
    public var playGame:PlayGameSignal;
    [Inject]
    public var playerModel:PlayerModel;
    [Inject]
    public var gameClosed:GameClosedSignal;
    [Inject]
    public var mapModel:MapModel;
    [Inject]
    public var closeDialogs:CloseDialogsSignal;
    [Inject]
    public var monitor:LoopMonitor;
    [Inject]
    public var hudSetupStarted:HUDSetupStarted;
    [Inject]
    public var updateHUDSignal:UpdateHUDSignal;
    [Inject]
    public var hudModelInitialized:HUDModelInitialized;
    [Inject]
    public var showLoadingViewSignal:ShowLoadingViewSignal;
    [Inject]
    public var openDialog:OpenDialogSignal;
    [Inject]
    public var dialogsModel:DialogsModel;
    [Inject]
    public var addToQueueSignal:AddPopupToStartupQueueSignal;
    [Inject]
    public var flushQueueSignal:FlushPopupStartupQueueSignal;


    public static function sleepForMs(_arg1:int):void {
        var _local2:int = getTimer();
        while (true) {
            if ((getTimer() - _local2) >= _arg1) break;
        }
    }


    override public function initialize():void {
        this.showLoadingViewSignal.dispatch();
        this.setWorldInteraction.add(this.onSetWorldInteraction);
        addViewListener(ReconnectEvent.RECONNECT, this.onReconnect);
        this.view.modelInitialized.add(this.onGameSpriteModelInitialized);
        this.view.drawCharacterWindow.add(this.onStatusPanelDraw);
        this.hudModelInitialized.add(this.onHUDModelInitialized);
        this.view.monitor.add(this.onMonitor);
        this.view.closed.add(this.onClosed);
        this.view.mapModel = this.mapModel;
        this.view.openDialog = this.openDialog;
        this.view.connect();
    }

    override public function destroy():void {
        this.setWorldInteraction.remove(this.onSetWorldInteraction);
        removeViewListener(ReconnectEvent.RECONNECT, this.onReconnect);
        this.view.modelInitialized.remove(this.onGameSpriteModelInitialized);
        this.view.drawCharacterWindow.remove(this.onStatusPanelDraw);
        this.hudModelInitialized.remove(this.onHUDModelInitialized);
        this.view.closed.remove(this.onClosed);
        this.view.monitor.remove(this.onMonitor);
        this.view.disconnect();
    }

    private function onMonitor(_arg1:String, _arg2:int):void {
        this.monitor.recordTime(_arg1, _arg2);
    }

    public function onSetWorldInteraction(_arg1:Boolean):void {
        this.view.mui_.setEnablePlayerInput(_arg1);
    }

    private function onClosed():void {
        if (!this.view.isEditor) {
            this.gameClosed.dispatch();
        }
        this.closeDialogs.dispatch();
        var _local1:HideMapLoadingSignal = StaticInjectorContext.getInjector().getInstance(HideMapLoadingSignal);
        _local1.dispatch();
        sleepForMs(100);
    }

    private function onReconnect(_arg1:ReconnectEvent):void {
        if (this.view.isEditor) {
            return;
        }
        var _local2:GameInitData = new GameInitData();
        _local2.server = _arg1.server_;
        _local2.gameId = _arg1.gameId_;
        _local2.createCharacter = _arg1.createCharacter_;
        _local2.charId = _arg1.charId_;
        _local2.keyTime = _arg1.keyTime_;
        _local2.key = _arg1.key_;
        _local2.isFromArena = _arg1.isFromArena_;
        this.playGame.dispatch(_local2);
    }

    private function onGameSpriteModelInitialized():void {
        this.hudSetupStarted.dispatch(this.view);
    }

    private function onStatusPanelDraw(_arg1:Player):void {
        this.updateHUDSignal.dispatch(_arg1);
    }

    private function onHUDModelInitialized():void {
        this.view.hudModelInitialized();
    }


}
}

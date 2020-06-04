﻿package com.company.assembleegameclient.screens {
import com.company.assembleegameclient.ui.GuildText;
import com.company.assembleegameclient.ui.RankText;
import com.company.assembleegameclient.ui.tooltip.RankToolTip;

import flash.display.DisplayObject;
import flash.display.Shape;
import flash.display.Sprite;
import flash.events.MouseEvent;

import kabam.rotmg.account.core.view.AccountInfoView;

import org.osflash.signals.Signal;

public class AccountScreen extends Sprite {

    public var tooltip:Signal;
    private var rankLayer:Sprite;
    private var guildLayer:Sprite;
    private var accountInfoLayer:Sprite;
    private var guildName:String;
    private var guildRank:int;
    private var stars:int;
    private var rank:int;
    private var admin:Boolean;
    private var rankText:RankText;
    private var guildText:GuildText;
    private var accountInfo:AccountInfoView;

    public function AccountScreen() {
        this.tooltip = new Signal();
        this.makeLayers();
    }

    private function makeLayers():void {
        addChild((this.rankLayer = new Sprite()));
        addChild((this.guildLayer = new Sprite()));
        addChild((this.accountInfoLayer = new Sprite()));
    }

    private function returnHeaderBackground():DisplayObject {
        var _local1:Shape = new Shape();
        _local1.graphics.beginFill(0, 0.5);
        _local1.graphics.drawRect(0, 0, 800, 35);
        return (_local1);
    }

    public function setGuild(_arg1:String, _arg2:int):void {
        this.guildName = _arg1;
        this.guildRank = _arg2;
        this.makeGuildText();
    }

    private function makeGuildText():void {
        this.guildText = new GuildText(this.guildName, this.guildRank);
        this.guildText.x = 36;
        this.guildText.y = 6;
        while (this.guildLayer.numChildren > 0) {
            this.guildLayer.removeChildAt(0);
        }
        this.guildLayer.addChild(this.guildText);
    }

    public function setAccountInfo(_arg1:AccountInfoView):void {
        var _local2:DisplayObject;
        this.accountInfo = _arg1;
        _local2 = (_arg1 as DisplayObject);
        _local2.x = (stage.stageWidth - 10);
        _local2.y = 2;
        while (this.accountInfoLayer.numChildren > 0) {
            this.accountInfoLayer.removeChildAt(0);
        }
        this.accountInfoLayer.addChild(_local2);
    }

    protected function onMouseOver(_arg1:MouseEvent):void {
        this.tooltip.dispatch(new RankToolTip(this.stars));
    }

    protected function onRollOut(_arg1:MouseEvent):void {
        this.tooltip.dispatch(null);
    }


}
}

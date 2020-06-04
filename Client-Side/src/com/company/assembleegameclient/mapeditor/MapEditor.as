﻿package com.company.assembleegameclient.mapeditor
{
import com.company.assembleegameclient.game.GameSprite;
import com.company.assembleegameclient.game.events.DeathEvent;
import com.company.assembleegameclient.game.events.ReconnectEvent;
import com.company.assembleegameclient.parameters.Parameters;
import com.company.assembleegameclient.screens.TitleMenuOption;
import flash.display.Sprite;
import flash.events.Event;
import kabam.rotmg.core.model.PlayerModel;
import kabam.rotmg.servers.api.Server;

public class MapEditor extends Sprite
{


    private var model:PlayerModel;

    private var server:Server;

    public var editingScreen_:EditingScreen;

    private var gameSprite_:GameSprite;

    public var returnButton_:TitleMenuOption;

    private var blackBackground:Sprite;

    public function MapEditor()
    {
        super();
        this.editingScreen_ = new EditingScreen();
        this.editingScreen_.addEventListener(MapTestEvent.MAP_TEST,this.onMapTest);
        addChild(this.editingScreen_);
    }

    public function initialize(_arg1:PlayerModel, _arg2:Server) : void
    {
        this.model = _arg1;
        this.server = _arg2;
    }

    private function onMapTest(_arg1:MapTestEvent) : void
    {
        removeChild(this.editingScreen_);
        this.gameSprite_ = new GameSprite(this.server,Parameters.MAPTEST_GAMEID,false,this.model.getSavedCharacters()[0].charId(),-1,null,this.model,_arg1.mapJSON_,false);
        this.gameSprite_.isEditor = true;
        this.gameSprite_.addEventListener(Event.COMPLETE,this.onMapTestDone);
        this.gameSprite_.addEventListener(ReconnectEvent.RECONNECT,this.onMapTestDone);
        this.gameSprite_.addEventListener(DeathEvent.DEATH,this.onMapTestDone);
        this.gameSprite_.closed.add(this.onMapTestDoneSub);
        addChild(this.gameSprite_);
    }

    private function onMapTestDoneSub() : void
    {
        this.onMapTestDone(null);
    }

    private function onMapTestDone(_arg1:Event) : void
    {
        this.cleanupGameSprite();
        addChild(this.editingScreen_);
    }

    private function onSubmitClose() : void
    {
        this.hideCurtain();
        this.editingScreen_.enableInput();
    }

    private function onClientUpdate(_arg1:Event) : void
    {
        this.cleanupGameSprite();
        addChild(this.editingScreen_);
    }

    private function showCurtain() : void
    {
        if(this.blackBackground == null)
        {
            this.blackBackground = new Sprite();
            this.blackBackground.graphics.beginFill(0,0.4);
            this.blackBackground.graphics.drawRect(0,0,width,height);
            this.blackBackground.graphics.endFill();
        }
        addChild(this.blackBackground);
    }

    private function hideCurtain() : void
    {
        if(this.blackBackground != null && this.blackBackground.parent)
        {
            removeChild(this.blackBackground);
        }
    }

    private function cleanupGameSprite() : void
    {
        this.gameSprite_.removeEventListener(Event.COMPLETE,this.onMapTestDone);
        this.gameSprite_.removeEventListener(ReconnectEvent.RECONNECT,this.onMapTestDone);
        this.gameSprite_.removeEventListener(DeathEvent.DEATH,this.onMapTestDone);
        this.gameSprite_.closed.remove(this.onMapTestDoneSub);
        removeChild(this.gameSprite_);
        this.gameSprite_ = null;
    }
}
}

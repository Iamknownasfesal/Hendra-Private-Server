﻿package com.company.assembleegameclient.map {
import com.company.assembleegameclient.background.Background;
import com.company.assembleegameclient.game.AGameSprite;
import com.company.assembleegameclient.map.mapoverlay.MapOverlay;
import com.company.assembleegameclient.map.partyoverlay.PartyOverlay;
import com.company.assembleegameclient.objects.BasicObject;
import com.company.assembleegameclient.objects.Party;
import com.company.assembleegameclient.objects.Player;

import flash.display.Sprite;
import flash.geom.Point;
import flash.utils.Dictionary;

import org.osflash.signals.Signal;

public class AbstractMap extends Sprite {

    public var goDict_:Dictionary;
    public var gs_:AGameSprite;
    public var name_:String;
    public var player_:Player = null;
    public var showDisplays_:Boolean;
    public var width_:int;
    public var height_:int;
    public var back_:int;
    protected var allowPlayerTeleport_:Boolean;
    public var background_:Background = null;
    public var map_:Sprite;
    public var hurtOverlay_:HurtOverlay = null;
    public var gradientOverlay_:GradientOverlay = null;
    public var mapOverlay_:MapOverlay = null;
    public var partyOverlay_:PartyOverlay = null;
    public var squareList_:Vector.<Square>;
    public var squares_:Vector.<Square>;
    public var boDict_:Dictionary;
    public var merchLookup_:Object;
    public var party_:Party = null;
    public var quest_:Quest = null;
    public var signalRenderSwitch:Signal;
    protected var wasLastFrameGpu:Boolean = false;
    public var isPetYard:Boolean = false;

    public function AbstractMap() {
        this.goDict_ = new Dictionary();
        this.map_ = new Sprite();
        this.squareList_ = new Vector.<Square>();
        this.squares_ = new Vector.<Square>();
        this.boDict_ = new Dictionary();
        this.merchLookup_ = new Object();
        this.signalRenderSwitch = new Signal(Boolean);
        super();
    }

    public function setProps(_arg1:int, _arg2:int, _arg3:String, _arg4:int, _arg5:Boolean, _arg6:Boolean):void {
    }

    public function addObj(_arg1:BasicObject, _arg2:Number, _arg3:Number):void {
    }

    public function setGroundTile(_arg1:int, _arg2:int, _arg3:uint):void {
    }

    public function initialize():void {
    }

    public function dispose():void {
    }

    public function update(_arg1:int, _arg2:int):void {
    }

    public function pSTopW(_arg1:Number, _arg2:Number):Point {
        return (null);
    }

    public function removeObj(_arg1:int):void {
    }

    public function draw(_arg1:Camera, _arg2:int):void {
    }

    public function allowPlayerTeleport():Boolean {
        return (((!((this.name_ == Map.NEXUS))) && (this.allowPlayerTeleport_)));
    }


}
}

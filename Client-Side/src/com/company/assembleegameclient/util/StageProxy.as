package com.company.assembleegameclient.util
{
import flash.display.DisplayObject;
import flash.display.DisplayObjectContainer;
import flash.display.InteractiveObject;
import flash.events.Event;
import flash.events.IEventDispatcher;

public class StageProxy implements IEventDispatcher
{

    private static var stage3D:Stage3DProxy = null;


    protected var reference:DisplayObject;

    public function StageProxy(param1:DisplayObject)
    {
        super();
        this.reference = param1;
    }

    public function getStage() : DisplayObjectContainer
    {
        return this.reference.stage;
    }

    public function getStageWidth() : Number
    {
        if(this.reference.stage != null)
        {
            return this.reference.stage.stageWidth;
        }
        return 800;
    }

    public function getStageHeight() : Number
    {
        if(this.reference.stage != null)
        {
            return this.reference.stage.stageHeight;
        }
        return 600;
    }

    public function getFocus() : InteractiveObject
    {
        return this.reference.stage.focus;
    }

    public function setFocus(param1:InteractiveObject) : void
    {
        this.reference.stage.focus = param1;
    }

    public function addEventListener(param1:String, param2:Function, param3:Boolean = false, param4:int = 0, param5:Boolean = false) : void
    {
        this.reference.stage.addEventListener(param1,param2,param3,param4,param5);
    }

    public function removeEventListener(param1:String, param2:Function, param3:Boolean = false) : void
    {
        this.reference.stage.removeEventListener(param1,param2,param3);
    }

    public function hasEventListener(param1:String) : Boolean
    {
        return this.reference.stage.hasEventListener(param1);
    }

    public function willTrigger(param1:String) : Boolean
    {
        return this.reference.stage.willTrigger(param1);
    }

    public function getQuality() : String
    {
        return this.reference.stage.quality;
    }

    public function setQuality(param1:String) : void
    {
        this.reference.stage.quality = param1;
    }

    public function getStage3Ds(param1:int) : Stage3DProxy
    {
        if(stage3D == null)
        {
            stage3D = new Stage3DProxy(this.reference.stage.stage3Ds[param1]);
        }
        return stage3D;
    }

    public function dispatchEvent(param1:Event) : Boolean
    {
        return this.reference.stage.dispatchEvent(param1);
    }
}
}

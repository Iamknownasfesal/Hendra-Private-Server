package com.company.assembleegameclient.util
{
import flash.display.Stage3D;
import flash.events.Event;
import flash.events.IEventDispatcher;
import kabam.rotmg.stage3D.proxies.Context3DProxy;

public class Stage3DProxy implements IEventDispatcher
{

    private static var context3D:Context3DProxy;


    private var stage3D:Stage3D;

    public function Stage3DProxy(param1:Stage3D)
    {
        super();
        this.stage3D = param1;
    }

    public function requestContext3D() : void
    {
        this.stage3D.requestContext3D();
    }

    public function getContext3D() : Context3DProxy
    {
        if(context3D == null)
        {
            context3D = new Context3DProxy(this.stage3D.context3D);
        }
        return context3D;
    }

    public function addEventListener(param1:String, param2:Function, param3:Boolean = false, param4:int = 0, param5:Boolean = false) : void
    {
        this.stage3D.addEventListener(param1,param2,param3,param4,param5);
    }

    public function removeEventListener(param1:String, param2:Function, param3:Boolean = false) : void
    {
        this.stage3D.removeEventListener(param1,param2,param3);
    }

    public function hasEventListener(param1:String) : Boolean
    {
        return this.stage3D.hasEventListener(param1);
    }

    public function willTrigger(param1:String) : Boolean
    {
        return this.stage3D.willTrigger(param1);
    }

    public function dispatchEvent(param1:Event) : Boolean
    {
        return this.stage3D.dispatchEvent(param1);
    }
}
}

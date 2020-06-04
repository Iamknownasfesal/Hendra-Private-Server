package com.company.assembleegameclient.screens
{
import com.company.assembleegameclient.sound.SoundEffectLibrary;
import com.company.util.MoreColorUtil;
import flash.display.Sprite;
import flash.events.Event;
import flash.events.MouseEvent;
import flash.filters.DropShadowFilter;
import flash.geom.ColorTransform;
import flash.utils.getTimer;
import kabam.rotmg.text.view.TextFieldDisplayConcrete;
import kabam.rotmg.text.view.stringBuilder.LineBuilder;
import org.osflash.signals.Signal;

public class TitleMenuOption extends Sprite
{

    protected static const OVER_COLOR_TRANSFORM:ColorTransform = new ColorTransform(1,220 / 255,133 / 255);

    private static const DROP_SHADOW_FILTER:DropShadowFilter = new DropShadowFilter(0,0,0,0.5,12,12);


    public const clicked:Signal = new Signal();

    public var textField:TextFieldDisplayConcrete;

    public var changed:Signal;

    private var colorTransform:ColorTransform;

    private var size:int;

    private var isPulse:Boolean;

    private var originalWidth:Number;

    private var originalHeight:Number;

    private var active:Boolean;

    private var color:uint = 16777215;

    private var hoverColor:uint;

    public function TitleMenuOption(param1:String, param2:int, param3:Boolean)
    {
        this.textField = this.makeTextFieldDisplayConcrete();
        this.changed = this.textField.textChanged;
        super();
        this.size = param2;
        this.isPulse = param3;
        this.textField.setSize(param2).setColor(16777215).setBold(true);
        this.setTextKey(param1);
        this.originalWidth = width;
        this.originalHeight = height;
        this.activate();
    }

    public function activate() : void
    {
        addEventListener(MouseEvent.MOUSE_OVER,this.onMouseOver);
        addEventListener(MouseEvent.MOUSE_OUT,this.onMouseOut);
        addEventListener(MouseEvent.CLICK,this.onMouseClick);
        addEventListener(Event.ADDED_TO_STAGE,this.onAddedToStage);
        addEventListener(Event.REMOVED_FROM_STAGE,this.onRemovedFromStage);
        this.active = true;
    }

    public function deactivate() : void
    {
        var _loc1_:ColorTransform = new ColorTransform();
        _loc1_.color = 3552822;
        this.setColorTransform(_loc1_);
        removeEventListener(MouseEvent.MOUSE_OVER,this.onMouseOver);
        removeEventListener(MouseEvent.MOUSE_OUT,this.onMouseOut);
        removeEventListener(MouseEvent.CLICK,this.onMouseClick);
        removeEventListener(Event.ADDED_TO_STAGE,this.onAddedToStage);
        removeEventListener(Event.REMOVED_FROM_STAGE,this.onRemovedFromStage);
        this.active = false;
    }

    public function setColor(param1:uint) : void
    {
        this.color = param1;
        var _loc2_:uint = (param1 & 16711680) >> 16;
        var _loc3_:uint = (param1 & 65280) >> 8;
        var _loc4_:uint = param1 & 255;
        var _loc5_:ColorTransform = new ColorTransform(_loc2_ / 255,_loc3_ / 255,_loc4_ / 255);
        this.setColorTransform(_loc5_);
    }

    public function isActive() : Boolean
    {
        return this.active;
    }

    private function makeTextFieldDisplayConcrete() : TextFieldDisplayConcrete
    {
        var _loc1_:TextFieldDisplayConcrete = null;
        _loc1_ = new TextFieldDisplayConcrete();
        _loc1_.filters = [DROP_SHADOW_FILTER];
        addChild(_loc1_);
        return _loc1_;
    }

    public function setTextKey(param1:String) : void
    {
        name = param1;
        this.textField.setStringBuilder(new LineBuilder().setParams(param1));
    }

    public function setAutoSize(param1:String) : void
    {
        this.textField.setAutoSize(param1);
    }

    public function setVerticalAlign(param1:String) : void
    {
        this.textField.setVerticalAlign(param1);
    }

    private function onAddedToStage(param1:Event) : void
    {
        if(this.isPulse)
        {
            addEventListener(Event.ENTER_FRAME,this.onEnterFrame);
        }
    }

    private function onRemovedFromStage(param1:Event) : void
    {
        if(this.isPulse)
        {
            removeEventListener(Event.ENTER_FRAME,this.onEnterFrame);
        }
    }

    private function onEnterFrame(param1:Event) : void
    {
        var _loc2_:Number = 1.05 + 0.05 * Math.sin(getTimer() / 200);
        this.textField.scaleX = _loc2_;
        this.textField.scaleY = _loc2_;
    }

    public function setColorTransform(param1:ColorTransform) : void
    {
        if(param1 == this.colorTransform)
        {
            return;
        }
        this.colorTransform = param1;
        if(this.colorTransform == null)
        {
            this.textField.transform.colorTransform = MoreColorUtil.identity;
        }
        else
        {
            this.textField.transform.colorTransform = this.colorTransform;
        }
    }

    protected function onMouseOver(param1:MouseEvent) : void
    {
        this.setColorTransform(OVER_COLOR_TRANSFORM);
    }

    protected function onMouseOut(param1:MouseEvent) : void
    {
        if(this.color != 16777215)
        {
            this.setColor(this.color);
        }
        else
        {
            this.setColorTransform(null);
        }
    }

    protected function onMouseClick(param1:MouseEvent) : void
    {
        SoundEffectLibrary.play("button_click");
        this.clicked.dispatch();
    }

    override public function toString() : String
    {
        return "[TitleMenuOption " + this.textField.getText() + "]";
    }

    public function createNoticeTag(param1:String, param2:int, param3:uint, param4:Boolean) : void
    {
        var _loc5_:TextFieldDisplayConcrete = null;
        _loc5_ = new TextFieldDisplayConcrete();
        _loc5_.setSize(param2).setColor(param3).setBold(param4);
        _loc5_.setStringBuilder(new LineBuilder().setParams(param1));
        _loc5_.x = this.textField.x - 4;
        _loc5_.y = this.textField.y - 20;
        addChild(_loc5_);
    }
}
}

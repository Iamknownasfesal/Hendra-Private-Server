package com.company.assembleegameclient.screens {
import com.company.ui.BaseSimpleText;

import flash.display.Bitmap;
import flash.display.BitmapData;
import flash.display.Shape;
import flash.display.Sprite;
import flash.events.MouseEvent;
import flash.filters.DropShadowFilter;
import flash.net.URLRequest;
import flash.net.navigateToURL;

import kabam.rotmg.text.view.TextFieldDisplayConcrete;
import kabam.rotmg.text.view.stringBuilder.LineBuilder;
import kabam.rotmg.text.view.stringBuilder.StaticStringBuilder;
import kabam.rotmg.text.view.stringBuilder.StringBuilder;

import org.osflash.signals.Signal;

public class GraveyardLine extends Sprite {

    public static const WIDTH:int = 167;
    public static const HEIGHT:int = 77;
    public static const COLOR:uint = 0xB3B3B3;
    public static const OVER_COLOR:uint = 0xFFC800;

    public var overColor:uint;
    public var viewCharacterFame:Signal;
    public var icon_:Bitmap;
    public var titleText_:TextFieldDisplayConcrete;
    public var taglineText_:TextFieldDisplayConcrete;
    public var killText_:TextFieldDisplayConcrete;
    public var link:String;
    public var accountId:String;
    private var box:Shape;

    public function GraveyardLine(_arg1:BitmapData, _arg2:String, _arg3:String, _arg4:String, _arg5:int, _arg6:String) {
        this.box = new Shape();
        this.makeBox();
        this.viewCharacterFame = new Signal(int);
        super();
        this.link = _arg4;
        this.accountId = _arg6;
        buttonMode = false;
        useHandCursor = false;
        tabEnabled = false;
        this.titleText_ = new TextFieldDisplayConcrete().setSize(14).setColor(0xFFFFFF);
        this.titleText_.setStringBuilder(new StaticStringBuilder(_arg2));
        this.titleText_.setBold(true);
        this.titleText_.x = 15;
        this.titleText_.y = 4;
        addChild(this.titleText_);
        this.taglineText_ = new TextFieldDisplayConcrete().setSize(14).setColor(COLOR);
        this.taglineText_.setStringBuilder(new StaticStringBuilder(_arg3));
        this.taglineText_.x = 15;
        this.taglineText_.y = 24;
        addChild(this.taglineText_);
        addEventListener(MouseEvent.MOUSE_OVER, this.onMouseOver);
        addEventListener(MouseEvent.ROLL_OUT, this.onRollOut);
        addEventListener(MouseEvent.MOUSE_DOWN, this.onMouseDown);
    }

    protected function makeBox():void
    {
        this.drawBox(false);
        addChild(this.box);
    }

    private function drawBox(_arg1:Boolean) : void
    {
        this.box.graphics.clear();
        this.box.graphics.beginFill(((_arg1) ? this.overColor : 0x333333));
        this.box.graphics.drawRect(0,0,WIDTH,HEIGHT);
        this.box.graphics.endFill();
    }

    protected function onMouseOver(_arg1:MouseEvent):void {
        this.titleText_.setColor(OVER_COLOR);
        this.taglineText_.setColor(OVER_COLOR);
    }

    protected function onRollOut(_arg1:MouseEvent):void {
        this.titleText_.setColor(0xFFFFFF);
        this.taglineText_.setColor(COLOR);
    }

    protected function onMouseDown(_arg1:MouseEvent):void {
        var _local2:Array = this.link.split(":", 2);
        switch (_local2[0]) {
            case "fame":
                this.viewCharacterFame.dispatch(int(_local2[1]));
                return;
            case "http":
            case "https":
            default:
                navigateToURL(new URLRequest(this.link), "_blank");
        }
    }

    private function getTimeDiff(_arg1:int):String {
        var _local2:Number = (new Date().getTime() / 1000);
        var _local3:int = (_local2 - _arg1);
        if (_local3 <= 0) {
            return ("now");
        }
        if (_local3 < 60) {
            return ((_local3 + " secs"));
        }
        if (_local3 < (60 * 60)) {
            return ((int((_local3 / 60)) + " mins"));
        }
        if (_local3 < ((60 * 60) * 24)) {
            return ((int((_local3 / (60 * 60))) + " hours"));
        }
        return ((int((_local3 / ((60 * 60) * 24))) + " days"));
    }


}
}

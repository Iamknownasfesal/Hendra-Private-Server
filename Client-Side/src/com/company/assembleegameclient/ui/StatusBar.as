﻿package com.company.assembleegameclient.ui {
import com.company.assembleegameclient.parameters.Parameters;

import flash.display.Sprite;
import flash.events.Event;
import flash.events.MouseEvent;
import flash.filters.DropShadowFilter;

import kabam.rotmg.text.view.TextFieldDisplayConcrete;
import kabam.rotmg.text.view.stringBuilder.LineBuilder;
import kabam.rotmg.text.view.stringBuilder.StaticStringBuilder;

import org.osflash.signals.Signal;

public class StatusBar extends Sprite {

    public static var barTextSignal:Signal = new Signal(Boolean);

    public var w_:int;
    public var h_:int;
    public var color_:uint;
    public var backColor_:uint;
    public var pulseBackColor:uint;
    public var textColor_:uint;
    public var val_:int = -1;
    public var max_:int = -1;
    public var boost_:int = -1;
    public var maxMax_:int = -1;
    public var level_:int = 0;
    private var labelText_:TextFieldDisplayConcrete;
    private var labelTextStringBuilder_:LineBuilder;
    private var valueText_:TextFieldDisplayConcrete;
    private var valueTextStringBuilder_:StaticStringBuilder;
    private var boostText_:TextFieldDisplayConcrete;
    private var multiplierText:TextFieldDisplayConcrete;
    public var multiplierIcon:Sprite;
    private var colorSprite:Sprite;
    private var defaultForegroundColor:Number;
    private var defaultBackgroundColor:Number;
    public var mouseOver_:Boolean = false;
    private var isPulsing:Boolean = false;
    private var forceNumText_:Boolean = false;
    private var isProgressBar_:Boolean = false;
    private var repetitions:int;
    private var direction:int = -1;
    private var speed:Number = 0.1;

    public function StatusBar(_arg1:int, _arg2:int, _arg3:uint, _arg4:uint, _arg5:String = null, param6:Boolean = false, param7:Boolean = false) {
        this.colorSprite = new Sprite();
        super();
        this.isProgressBar_ = param7;
        addChild(this.colorSprite);
        this.w_ = _arg1;
        this.h_ = _arg2;
        this.forceNumText_ = param6;
        this.defaultForegroundColor = (this.color_ = _arg3);
        this.defaultBackgroundColor = (this.backColor_ = _arg4);
        this.textColor_ = 0xFFFFFF;
        if (((!((_arg5 == null))) && (!((_arg5.length == 0))))) {
            this.labelText_ = new TextFieldDisplayConcrete().setSize(14).setColor(this.textColor_);
            this.labelText_.setBold(true);
            this.labelTextStringBuilder_ = new LineBuilder().setParams(_arg5);
            this.labelText_.setStringBuilder(this.labelTextStringBuilder_);
            this.centerVertically(this.labelText_);
            this.labelText_.filters = [new DropShadowFilter(0, 0, 0)];
            addChild(this.labelText_);
        }
        this.valueText_ = new TextFieldDisplayConcrete().setSize(14).setColor(0xFFFFFF);
        this.valueText_.setBold(true);
        this.valueText_.filters = [new DropShadowFilter(0, 0, 0)];
        this.centerVertically(this.valueText_);
        this.valueTextStringBuilder_ = new StaticStringBuilder();
        this.boostText_ = new TextFieldDisplayConcrete().setSize(14).setColor(this.textColor_);
        this.boostText_.setBold(true);
        this.boostText_.alpha = 0.6;
        this.centerVertically(this.boostText_);
        this.boostText_.filters = [new DropShadowFilter(0, 0, 0)];
        this.multiplierIcon = new Sprite();
        this.multiplierIcon.x = (this.w_ - 25);
        this.multiplierIcon.y = -3;
        this.multiplierIcon.graphics.beginFill(0xFF00FF, 0);
        this.multiplierIcon.graphics.drawRect(0, 0, 20, 20);
        this.multiplierIcon.addEventListener(MouseEvent.MOUSE_OVER, this.onMultiplierOver);
        this.multiplierIcon.addEventListener(MouseEvent.MOUSE_OUT, this.onMultiplierOut);
        this.multiplierText = new TextFieldDisplayConcrete().setSize(14).setColor(9493531);
        this.multiplierText.setBold(true);
        this.multiplierText.setStringBuilder(new StaticStringBuilder("x2"));
        this.multiplierText.filters = [new DropShadowFilter(0, 0, 0)];
        this.multiplierIcon.addChild(this.multiplierText);
        if (!Parameters.data_.toggleBarText) {
            addEventListener(MouseEvent.ROLL_OVER, this.onMouseOver);
            addEventListener(MouseEvent.ROLL_OUT, this.onMouseOut);
        }
        barTextSignal.add(this.setBarText);
    }

    public function centerVertically(_arg1:TextFieldDisplayConcrete):void {
        _arg1.setVerticalAlign(TextFieldDisplayConcrete.MIDDLE);
        _arg1.y = ((this.h_ / 2) + 1);
    }

    private function onMultiplierOver(_arg1:MouseEvent):void {
        dispatchEvent(new Event("MULTIPLIER_OVER"));
    }

    private function onMultiplierOut(_arg1:MouseEvent):void {
        dispatchEvent(new Event("MULTIPLIER_OUT"));
    }

    public function draw(_arg1:int, _arg2:int, _arg3:int, _arg4:int = -1, param5:int = 0):void {
        if (_arg2 > 0) {
            _arg1 = Math.min(_arg2, Math.max(0, _arg1));
        }
        if ((((((((_arg1 == this.val_)) && ((_arg2 == this.max_)))) && ((_arg3 == this.boost_)))) && ((_arg4 == this.maxMax_)))) {
            return;
        }
        this.val_ = _arg1;
        this.max_ = _arg2;
        this.boost_ = _arg3;
        this.maxMax_ = _arg4;
        this.level_ = param5;
        this.internalDraw();
    }

    public function setLabelText(_arg1:String, _arg2:Object = null):void {
        this.labelTextStringBuilder_.setParams(_arg1, _arg2);
        this.labelText_.setStringBuilder(this.labelTextStringBuilder_);
    }

    private function setTextColor(_arg1:uint):void {
        this.textColor_ = _arg1;
        if (this.boostText_ != null) {
            this.boostText_.setColor(this.textColor_);
        }
        this.valueText_.setColor(this.textColor_);
    }

    public function setBarText(_arg1:Boolean):void {
        this.mouseOver_ = false;
        if (_arg1) {
            removeEventListener(MouseEvent.ROLL_OVER, this.onMouseOver);
            removeEventListener(MouseEvent.ROLL_OUT, this.onMouseOut);
        }
        else {
            addEventListener(MouseEvent.ROLL_OVER, this.onMouseOver);
            addEventListener(MouseEvent.ROLL_OUT, this.onMouseOut);
        }
        this.internalDraw();
    }

    private function bTextEnabled(param1:int) : Boolean
    {
        return param1 && (param1 == 1 || param1 == 2 && this.isProgressBar_ || param1 == 3 && !this.isProgressBar_);
    }

    private function internalDraw():void {
        graphics.clear();
        this.colorSprite.graphics.clear();
        var _local1:uint = 0xFFFFFF;
        if ((((this.maxMax_ > 0)) && (((this.max_ - this.boost_) == this.maxMax_)))) {
            _local1 = 0xFCDF00;
        }
        else {
            if (this.boost_ > 0) {
                _local1 = 6206769;
            }
        }
        if (this.textColor_ != _local1) {
            this.setTextColor(_local1);
        }
        graphics.beginFill(this.backColor_);
        graphics.drawRect(0, 0, this.w_, this.h_);
        graphics.endFill();
        if (this.isPulsing) {
            this.colorSprite.graphics.beginFill(this.pulseBackColor);
            this.colorSprite.graphics.drawRect(0, 0, this.w_, this.h_);
        }
        this.colorSprite.graphics.beginFill(this.color_);
        if (this.max_ > 0) {
            this.colorSprite.graphics.drawRect(0, 0, (this.w_ * (this.val_ / this.max_)), this.h_);
        }
        else {
            this.colorSprite.graphics.drawRect(0, 0, this.w_, this.h_);
        }
        this.colorSprite.graphics.endFill();
        if (contains(this.valueText_)) {
            removeChild(this.valueText_);
        }
        if (contains(this.boostText_)) {
            removeChild(this.boostText_);
        }
        if (this.bTextEnabled(Parameters.data_.toggleBarText) || this.mouseOver_ && this.h_ > 4 || this.forceNumText_) {
            this.drawWithMouseOver();
        }
    }

    public function drawWithMouseOver():void {
        var _loc2_:int = 0;
        var _loc1_:String = "";
        if(Parameters.data_.toggleToMaxText)
        {
            _loc2_ = this.maxMax_ - (this.max_ - this.boost_);
            if(this.level_ >= 20 && _loc2_ > 0)
            {
                _loc1_ = _loc1_ + ("|" + Math.ceil(_loc2_ / 5).toString());
            }
        }
        if (this.max_ > 0) {
            this.valueText_.setStringBuilder(this.valueTextStringBuilder_.setString(((this.val_ + "/") + this.max_ + _loc1_)));
        }
        else {
            this.valueText_.setStringBuilder(this.valueTextStringBuilder_.setString(("" + this.val_)));
        }
        if (!contains(this.valueText_)) {
            this.valueText_.mouseEnabled = false;
            this.valueText_.mouseChildren = false;
            addChild(this.valueText_);
        }
        if (this.boost_ != 0) {
            this.boostText_.setStringBuilder(this.valueTextStringBuilder_.setString((((" (" + (((this.boost_ > 0)) ? "+" : "")) + this.boost_.toString()) + ")")));
            if (!contains(this.boostText_)) {
                this.boostText_.mouseEnabled = false;
                this.boostText_.mouseChildren = false;
                addChild(this.boostText_);
            }
            this.valueText_.x = ((this.w_ / 2) - ((this.valueText_.width + this.boostText_.width) / 2));
            this.boostText_.x = (this.valueText_.x + this.valueText_.width);
        }
        else {
            this.valueText_.x = ((this.w_ / 2) - (this.valueText_.width / 2));
            if (contains(this.boostText_)) {
                removeChild(this.boostText_);
            }
        }
    }

    public function showMultiplierText():void {
        this.multiplierIcon.mouseEnabled = false;
        this.multiplierIcon.mouseChildren = false;
        addChild(this.multiplierIcon);
        this.startPulse(3, 9493531, 0xFFFFFF);
    }

    public function hideMultiplierText():void {
        if (this.multiplierIcon.parent) {
            removeChild(this.multiplierIcon);
        }
    }

    public function startPulse(_arg1:Number, _arg2:Number, _arg3:Number):void {
        this.isPulsing = true;
        this.color_ = _arg2;
        this.pulseBackColor = _arg3;
        this.repetitions = _arg1;
        this.internalDraw();
        addEventListener(Event.ENTER_FRAME, this.onPulse);
    }

    private function onPulse(_arg1:Event):void {
        if ((((this.colorSprite.alpha > 1)) || ((this.colorSprite.alpha < 0)))) {
            this.direction = (this.direction * -1);
            if (this.colorSprite.alpha > 1) {
                this.repetitions--;
                if (!this.repetitions) {
                    this.isPulsing = false;
                    this.color_ = this.defaultForegroundColor;
                    this.backColor_ = this.defaultBackgroundColor;
                    this.colorSprite.alpha = 1;
                    this.internalDraw();
                    removeEventListener(Event.ENTER_FRAME, this.onPulse);
                }
            }
        }
        this.colorSprite.alpha = (this.colorSprite.alpha + (this.speed * this.direction));
    }

    private function onMouseOver(_arg1:MouseEvent):void {
        this.mouseOver_ = true;
        this.internalDraw();
    }

    private function onMouseOut(_arg1:MouseEvent):void {
        this.mouseOver_ = false;
        this.internalDraw();
    }


}
}

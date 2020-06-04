package com.company.assembleegameclient.screens.charrects
{
import avmplus.argXml;

import com.company.rotmg.graphics.StarGraphic;
import flash.display.Shape;
import flash.display.Sprite;
import flash.events.MouseEvent;
import flash.filters.DropShadowFilter;
import flash.geom.ColorTransform;
import kabam.rotmg.text.view.TextFieldDisplayConcrete;
import kabam.rotmg.text.view.stringBuilder.StringBuilder;

public class CharacterRect extends Sprite
{

    public static const WIDTH:int = 167;

    public static const HEIGHT:int = 77;


    public var color:uint;

    public var overColor:uint;

    public var selectContainer:Sprite;

    protected var selectedLocX:int = 0;
    protected var selectedLocY:int = 0;


    protected var classNameText:TextFieldDisplayConcrete;

    protected var className:StringBuilder;

    private var box:Shape;

    public function CharacterRect()
    {
        this.box = new Shape();
        super();
    }

    protected static function makeDropShadowFilter() : Array
    {
        return [new DropShadowFilter(0,0,0,1,8,8)];
    }

    public function init() : void
    {
        tabChildren = false;
        this.makeBox();
        this.makeContainer();
        this.makeClassNameText();
        this.addEventListeners();
    }

    public function makeBox() : void
    {
        this.drawBox(false);
        addChild(this.box);
    }

    public function makeContainer() : void
    {
        this.selectContainer = new Sprite();
        this.selectContainer.mouseChildren = false;
        this.selectContainer.buttonMode = true;
        this.selectContainer.graphics.beginFill(6710886,0.51);
        this.selectContainer.graphics.drawRect(0,0,WIDTH,HEIGHT);
        addChild(this.selectContainer);
    }

    protected function makeClassNameText() : void
    {
        this.classNameText = new TextFieldDisplayConcrete().setSize(14).setColor(0xffffff);
        this.classNameText.setBold(true);
        this.classNameText.setStringBuilder(this.className);
        if(selectedLocX != 0 || selectedLocY != 0)
        {
            this.classNameText.x = selectedLocX;
            this.classNameText.y = selectedLocY;
        }
        else
        {
            this.classNameText.x = CharacterRectConstants.CLASS_NAME_POS_X;
            this.classNameText.y = CharacterRectConstants.CLASS_NAME_POS_Y;
        }
        this.selectContainer.addChild(this.classNameText);
    }

    private function addEventListeners() : void
    {
        addEventListener(MouseEvent.MOUSE_OVER,this.onMouseOver);
        addEventListener(MouseEvent.ROLL_OUT,this.onRollOut);
    }

    private function drawBox(_arg1:Boolean) : void
    {
        this.box.graphics.clear();
        this.box.graphics.beginFill(((_arg1) ? this.overColor : this.color));
        this.box.graphics.drawRect(0,0,WIDTH,HEIGHT);
        this.box.graphics.endFill();
    }

    protected function onMouseOver(param1:MouseEvent) : void
    {
        this.drawBox(true);
    }

    protected function onRollOut(param1:MouseEvent) : void
    {
        this.drawBox(false);
    }
}
}

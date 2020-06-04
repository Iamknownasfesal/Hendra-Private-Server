package com.company.assembleegameclient.screens
{
import com.company.assembleegameclient.screens.charrects.CharacterRectList;
import flash.display.Graphics;
import flash.display.Shape;
import flash.display.Sprite;
import kabam.rotmg.core.model.PlayerModel;

public class CharacterList extends Sprite
{

    public static const WIDTH:int = 680;

    public static const HEIGHT:int = 415;

    public static const TYPE_CHAR_SELECT:int = 1;
    public static const TYPE_GRAVE_SELECT:int = 2;


    public var charRectList_:Sprite;

    public function CharacterList(param1:PlayerModel, param2:int)
    {
        var _loc3_:Shape = null;
        var _loc4_:Graphics = null;
        super();
        switch(param2)
        {
            case TYPE_CHAR_SELECT:
                this.charRectList_ = new CharacterRectList();
                break;
            case TYPE_GRAVE_SELECT:
                this.charRectList_ = new Graveyard(param1);
                break;
            default:
                this.charRectList_ = new Sprite();
        }
        addChild(this.charRectList_);
        if(height > 400)
        {
            _loc3_ = new Shape();
            _loc4_ = _loc3_.graphics;
            _loc4_.beginFill(0);
            _loc4_.drawRect(0,0,WIDTH,HEIGHT);
            _loc4_.endFill();
            addChild(_loc3_);
            mask = _loc3_;
        }
    }

    public function setPos(param1:Number) : void
    {
        this.charRectList_.y = param1;
    }
}
}

package com.company.assembleegameclient.screens.charrects
{
import com.company.assembleegameclient.appengine.CharacterStats;
import com.company.assembleegameclient.appengine.SavedCharacter;
import com.company.assembleegameclient.parameters.Parameters;
import flash.display.Bitmap;
import flash.display.BitmapData;
import flash.display.DisplayObject;
import flash.display.Sprite;
import flash.events.Event;
import flash.events.MouseEvent;
import kabam.rotmg.assets.services.CharacterFactory;
import kabam.rotmg.classes.model.CharacterClass;
import kabam.rotmg.classes.model.CharacterSkin;
import kabam.rotmg.classes.model.ClassesModel;
import kabam.rotmg.core.StaticInjectorContext;
import kabam.rotmg.core.model.PlayerModel;
import org.osflash.signals.Signal;
import org.swiftsuspenders.Injector;

public class CharacterRectList extends Sprite
{


    public var newCharacter:Signal;

    public var buyCharacterSlot:Signal;

    private var classes:ClassesModel;

    private var model:PlayerModel;

    private var assetFactory:CharacterFactory;

    public function CharacterRectList()
    {
        var _loc2_:BuyCharacterRect = null;
        var _loc1_:SavedCharacter = null;
        _loc2_ = null;
        var _loc3_:CharacterClass = null;
        var _loc4_:CharacterStats = null;
        var _loc5_:CurrentCharacterRect = null;
        var _loc6_:int = 0;
        var _loc7_:CreateNewCharacterRect = null;
        super();
        var _loc8_:Injector = StaticInjectorContext.getInjector();
        this.classes = _loc8_.getInstance(ClassesModel);
        this.model = _loc8_.getInstance(PlayerModel);
        this.assetFactory = _loc8_.getInstance(CharacterFactory);
        this.newCharacter = new Signal();
        this.buyCharacterSlot = new Signal();
        var _loc9_:String = this.model.getName();
        var rowwidth:int = 0;
        var rowheight:int = 4;
        var _loc11_:Vector.<SavedCharacter> = this.model.getSavedCharacters();
        for each(_loc1_ in _loc11_)
        {
            _loc3_ = this.classes.getCharacterClass(_loc1_.objectType());
            _loc4_ = this.model.getCharStats()[_loc1_.objectType()];
            _loc5_ = new CurrentCharacterRect(_loc9_,_loc3_,_loc1_,_loc4_);
            if(Parameters.skinTypes16.indexOf(_loc1_.skinType()) != -1)
            {
                _loc5_.setIcon(this.getIcon(_loc1_,35));
            }
            else
            {
                _loc5_.setIcon(this.getIcon(_loc1_,70));
            }
            if(rowwidth > 680)
            {
                rowwidth = 0;
                rowheight = rowheight + (CharacterRect.HEIGHT + 4);
            }
            _loc5_.x = rowwidth;
            _loc5_.y = rowheight;
            addChild(_loc5_);
            rowwidth = rowwidth + (CharacterRect.WIDTH + 4);
        }
        if(this.model.hasAvailableCharSlot())
        {
            _loc6_ = 0;
            while(_loc6_ < this.model.getAvailableCharSlots())
            {
                _loc7_ = new CreateNewCharacterRect(this.model);
                _loc7_.addEventListener(MouseEvent.MOUSE_DOWN,this.onNewChar);
                if(rowwidth > 680)
                {
                    rowwidth = 0;
                    rowheight = rowheight + (CharacterRect.HEIGHT + 4);
                }
                _loc7_.x = rowwidth;
                _loc7_.y = rowheight;
                addChild(_loc7_);
                rowwidth = rowwidth + (CharacterRect.WIDTH + 4);
                _loc6_++;
            }
        }
        _loc2_ = new BuyCharacterRect(this.model);
        _loc2_.addEventListener(MouseEvent.MOUSE_DOWN,this.onBuyCharSlot);
        if(rowwidth > 680)
        {
            rowwidth = 0;
            rowheight = rowheight + (CharacterRect.HEIGHT + 4);
        }
        _loc2_.x = rowwidth;
        _loc2_.y = rowheight;
        addChild(_loc2_);
    }

    private function getIcon(param1:SavedCharacter, param2:int = 100) : DisplayObject
    {
        var _loc3_:CharacterClass = this.classes.getCharacterClass(param1.objectType());
        var _loc4_:CharacterSkin = _loc3_.skins.getSkin(param1.skinType()) || _loc3_.skins.getDefaultSkin();
        var _loc5_:BitmapData = this.assetFactory.makeIcon(_loc4_.template,param2,param1.tex1(),param1.tex2());
        return new Bitmap(_loc5_);
    }

    private function onNewChar(param1:Event) : void
    {
        this.newCharacter.dispatch();
    }

    private function onBuyCharSlot(param1:Event) : void
    {
        this.buyCharacterSlot.dispatch(this.model.getCharSlotPrice());
    }
}
}

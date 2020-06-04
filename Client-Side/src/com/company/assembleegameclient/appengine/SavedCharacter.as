package com.company.assembleegameclient.appengine {
import com.company.assembleegameclient.objects.ObjectLibrary;
import com.company.assembleegameclient.objects.Player;
import com.company.assembleegameclient.parameters.Parameters;
import com.company.assembleegameclient.util.AnimatedChar;
import com.company.assembleegameclient.util.AnimatedChars;
import com.company.assembleegameclient.util.MaskedImage;
import com.company.assembleegameclient.util.TextureRedrawer;
import com.company.assembleegameclient.util.redrawers.GlowRedrawer;
import com.company.util.CachingColorTransformer;

import flash.display.BitmapData;
import flash.geom.ColorTransform;

import kabam.rotmg.assets.services.CharacterFactory;
import kabam.rotmg.classes.model.CharacterClass;
import kabam.rotmg.classes.model.CharacterSkin;

import kabam.rotmg.classes.model.ClassesModel;
import kabam.rotmg.constants.GeneralConstants;

import kabam.rotmg.core.StaticInjectorContext;

import org.swiftsuspenders.Injector;

public class SavedCharacter {

    private static const notAvailableCT:ColorTransform = new ColorTransform(0, 0, 0, 0.5, 0, 0, 0, 0);
    private static const dimCT:ColorTransform = new ColorTransform(0.75, 0.75, 0.75, 1, 0, 0, 0, 0);

    public var charXML_:XML;
    public var name_:String = null;

    public function SavedCharacter(_arg1:XML, _arg2:String) {
        var _local3:XML;
        var _local4:int;
        super();
        this.charXML_ = _arg1;
        this.name_ = _arg2;
        if (this.charXML_.hasOwnProperty("Pet")) {
            _local3 = new XML(this.charXML_.Pet);
            _local4 = _local3.@instanceId;
        }
    }

    public static function getImage(_arg1:SavedCharacter, _arg2:XML, _arg3:int, _arg4:int, _arg5:Number, available:Boolean, active:Boolean):BitmapData {
        var _local8:AnimatedChar = AnimatedChars.getAnimatedChar(String(_arg2.AnimatedTexture.File), int(_arg2.AnimatedTexture.Index));
        var _local9:MaskedImage = _local8.imageFromDir(_arg3, _arg4, _arg5);
        var _local10:int = (((_arg1) != null) ? _arg1.tex1() : null);
        var _local11:int = (((_arg1) != null) ? _arg1.tex2() : null);
        var _local12:BitmapData = TextureRedrawer.resize(_local9.image_, _local9.mask_, 100, false, _local10, _local11);
        _local12 = GlowRedrawer.outlineGlow(_local12, 0);
        if (!available) {
            _local12 = CachingColorTransformer.transformBitmapData(_local12, notAvailableCT);
        }
        else {
            if (!active) {
                _local12 = CachingColorTransformer.transformBitmapData(_local12, dimCT);
            }
        }
        return (_local12);
    }

    public static function compare(_arg1:SavedCharacter, _arg2:SavedCharacter):Number {
        var _local3:Number = ((Parameters.data_.charIdUseMap.hasOwnProperty(_arg1.charId().toString())) ? Parameters.data_.charIdUseMap[_arg1.charId()] : 0);
        var _local4:Number = ((Parameters.data_.charIdUseMap.hasOwnProperty(_arg2.charId().toString())) ? Parameters.data_.charIdUseMap[_arg2.charId()] : 0);
        if (_local3 != _local4) {
            return ((_local4 - _local3));
        }
        return ((_arg2.xp() - _arg1.xp()));
    }

    public function fameBonus() : int
    {
        var _loc4_:int = 0;
        var _loc5_:XML = null;
        var _loc1_:Player = Player.fromPlayerXML("",this.charXML_);
        var _loc2_:int = 0;
        var _loc3_:uint = 0;
        while(_loc3_ < GeneralConstants.NUM_EQUIPMENT_SLOTS)
        {
            if(_loc1_.equipment_ && _loc1_.equipment_.length > _loc3_)
            {
                _loc4_ = _loc1_.equipment_[_loc3_];
                if(_loc4_ != -1)
                {
                    _loc5_ = ObjectLibrary.xmlLibrary_[_loc4_];
                    if(_loc5_ != null && _loc5_.hasOwnProperty("FameBonus"))
                    {
                        _loc2_ = _loc2_ + int(_loc5_.FameBonus);
                    }
                }
            }
            _loc3_++;
        }
        return _loc2_;
    }


    public function charId():int {
        return (int(this.charXML_.@id));
    }

    public function name():String {
        return (this.name_);
    }

    public function objectType():int {
        return (int(this.charXML_.ObjectType));
    }

    public function skinType():int {
        return (int(this.charXML_.Texture));
    }

    public function level():int {
        return (int(this.charXML_.Level));
    }

    public function tex1():int {
        return (int(this.charXML_.Tex1));
    }

    public function tex2():int {
        return (int(this.charXML_.Tex2));
    }

    public function xp():int {
        return (int(this.charXML_.Exp));
    }

    public function fame():int {
        return (int(this.charXML_.CurrentFame));
    }

    public function hp() : int
    {
        return int(this.charXML_.MaxHitPoints);
    }

    public function mp() : int
    {
        return int(this.charXML_.MaxMagicPoints);
    }

    public function att() : int
    {
        return int(this.charXML_.Attack);
    }

    public function def() : int
    {
        return int(this.charXML_.Defense);
    }

    public function spd() : int
    {
        return int(this.charXML_.Speed);
    }

    public function dex() : int
    {
        return int(this.charXML_.Dexterity);
    }

    public function vit() : int
    {
        return int(this.charXML_.HpRegen);
    }

    public function wis() : int
    {
        return int(this.charXML_.MpRegen);
    }

    public function displayId():String {
        return (ObjectLibrary.typeToDisplayId_[this.objectType()]);
    }

    public function getIcon(param1:int = 100) : BitmapData
    {
        var _loc2_:Injector = StaticInjectorContext.getInjector();
        var _loc3_:ClassesModel = _loc2_.getInstance(ClassesModel);
        var _loc4_:CharacterFactory = _loc2_.getInstance(CharacterFactory);
        var _loc5_:CharacterClass = _loc3_.getCharacterClass(this.objectType());
        var _loc6_:CharacterSkin = _loc5_.skins.getSkin(this.skinType()) || _loc5_.skins.getDefaultSkin();
        var _loc7_:BitmapData = _loc4_.makeIcon(_loc6_.template,param1,this.tex1(),this.tex2());
        return _loc7_;
    }

    public function bornOn() : String
    {
        if(!this.charXML_.hasOwnProperty("CreateTime"))
        {
            return "Unknown";
        }
        return this.charXML_.CreateTime;
    }

}
}

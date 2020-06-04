package com.company.assembleegameclient.screens.charrects
{
import com.company.assembleegameclient.appengine.CharacterStats;
import com.company.assembleegameclient.appengine.SavedCharacter;
import com.company.assembleegameclient.screens.events.DeleteCharacterEvent;
import com.company.assembleegameclient.ui.tooltip.MyPlayerToolTip;
import com.company.assembleegameclient.util.FameUtil;
import com.company.rotmg.graphics.DeleteXGraphic;

import flash.display.Bitmap;

import flash.display.BitmapData;
import flash.display.DisplayObject;
import flash.display.Sprite;
import flash.events.Event;
import flash.events.MouseEvent;

import kabam.rotmg.assets.services.IconFactory;
import kabam.rotmg.classes.model.CharacterClass;
import kabam.rotmg.core.StaticInjectorContext;
import kabam.rotmg.dialogs.control.OpenDialogSignal;
import kabam.rotmg.fame.FameContentPopup;
import kabam.rotmg.text.model.TextKey;
import kabam.rotmg.text.view.TextFieldDisplayConcrete;
import kabam.rotmg.text.view.stringBuilder.LineBuilder;
import kabam.rotmg.text.view.stringBuilder.StaticStringBuilder;

import org.osflash.signals.Signal;
import org.swiftsuspenders.Injector;

public class CurrentCharacterRect extends CharacterRect
{

    private static var toolTip_:MyPlayerToolTip = null;


    public const selected:Signal = new Signal();

    public const deleteCharacter:Signal = new Signal();

    public const showToolTip:Signal = new Signal(Sprite);

    public const hideTooltip:Signal = new Signal();

    public var charName:String;

    public var charStats:CharacterStats;

    public var char:SavedCharacter;

    public var myPlayerToolTipFactory:MyPlayerToolTipFactory;

    private var charType:CharacterClass;

    private var deleteButton:Sprite;

    private var icon:DisplayObject;

    protected var statsMaxedText:TextFieldDisplayConcrete;

    protected var removeText:TextFieldDisplayConcrete;

    private var removeContainer:Sprite;

    public function CurrentCharacterRect(param1:String, param2:CharacterClass, param3:SavedCharacter, param4:CharacterStats)
    {
        this.myPlayerToolTipFactory = new MyPlayerToolTipFactory();
        super();
        this.charName = param1;
        this.charType = param2;
        this.char = param3;
        this.charStats = param4;
        var _loc5_:String = param2.name;
        super.className = new LineBuilder().setParams(TextKey.CURRENT_CHARACTER_DESCRIPTION,{
            "className":_loc5_,
            "level":""
        });
        super.color = 6710886;
        super.overColor = 8355711;
        super.init();
        this.makeStatsMaxedText();
        this.makeDeleteText();
        this.addEventListeners();
    }

    private function addEventListeners() : void
    {
        addEventListener(Event.REMOVED_FROM_STAGE,this.onRemovedFromStage);
        selectContainer.addEventListener(MouseEvent.CLICK,this.onSelect);
        removeContainer.addEventListener(MouseEvent.CLICK,this.onDelete);
    }

    private function onSelect(param1:MouseEvent) : void
    {
        this.selected.dispatch(this.char);
    }

    private function onDelete(param1:MouseEvent) : void
    {
        this.deleteCharacter.dispatch(this.char);
    }

    public function setIcon(param1:DisplayObject) : void
    {
        this.icon && selectContainer.removeChild(this.icon);
        this.icon = param1;
        this.icon.x = CharacterRectConstants.ICON_POS_X;
        this.icon.y = CharacterRectConstants.ICON_POS_Y;
        this.icon && selectContainer.addChild(this.icon);
    }

    private function makeStatsMaxedText() : void
    {
        var _loc1_:int = this.getMaxedStats();
        var _loc2_:uint = 16572160;
        if(_loc1_ >= 8)
        {
            _loc2_ = 16572160;
        }
        this.statsMaxedText = new TextFieldDisplayConcrete().setSize(18).setColor(16777215);
        this.statsMaxedText.setBold(true);
        this.statsMaxedText.setColor(_loc2_);
        this.statsMaxedText.setStringBuilder(new StaticStringBuilder(_loc1_ + "/8"));
        this.statsMaxedText.filters = makeDropShadowFilter();
        this.statsMaxedText.x = CharacterRectConstants.STATS_MAXED_POS_X;
        this.statsMaxedText.y = CharacterRectConstants.STATS_MAXED_POS_Y;
        selectContainer.addChild(this.statsMaxedText);
    }

    private function makeDeleteText() : void
    {
        var _loc2_:uint = 0x999999;
        this.removeContainer = new Sprite();
        this.removeContainer.name = "remove_char";
        this.removeText = new TextFieldDisplayConcrete().setSize(15).setColor(0x999999);
        this.removeText.setBold(true);
        this.removeText.setColor(_loc2_);
        this.removeText.setStringBuilder(new StaticStringBuilder("Delete Char"));
        this.removeContainer.x = 99;
        this.removeContainer.y = 57;
        removeText.mouseChildren = false;
        removeText.buttonMode = true;
        this.removeContainer.addChild(this.removeText);
        addChild(this.removeContainer);
    }

    private function getMaxedStats() : int
    {
        var _loc1_:int = 0;
        if(this.char.hp() == this.charType.hp.max)
        {
            _loc1_++;
        }
        if(this.char.mp() == this.charType.mp.max)
        {
            _loc1_++;
        }
        if(this.char.att() == this.charType.attack.max)
        {
            _loc1_++;
        }
        if(this.char.def() == this.charType.defense.max)
        {
            _loc1_++;
        }
        if(this.char.spd() == this.charType.speed.max)
        {
            _loc1_++;
        }
        if(this.char.dex() == this.charType.dexterity.max)
        {
            _loc1_++;
        }
        if(this.char.vit() == this.charType.hpRegeneration.max)
        {
            _loc1_++;
        }
        if(this.char.wis() == this.charType.mpRegeneration.max)
        {
            _loc1_++;
        }
        return _loc1_;
    }

    override protected function onMouseOver(param1:MouseEvent) : void
    {
        super.onMouseOver(param1);
        this.removeToolTip();
        toolTip_ = this.myPlayerToolTipFactory.create(this.charName,this.char.charXML_,this.charStats);
        toolTip_.createUI();
        this.showToolTip.dispatch(toolTip_);
    }

    override protected function onRollOut(param1:MouseEvent) : void
    {
        super.onRollOut(param1);
        this.removeToolTip();
    }

    private function onRemovedFromStage(param1:Event) : void
    {
        this.removeToolTip();
    }

    private function removeToolTip() : void
    {
        this.hideTooltip.dispatch();
    }

    private function onDeleteDown(param1:MouseEvent) : void
    {
        param1.stopImmediatePropagation();
        dispatchEvent(new DeleteCharacterEvent(this.char));
    }
}
}

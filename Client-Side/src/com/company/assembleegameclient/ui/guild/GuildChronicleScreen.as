package com.company.assembleegameclient.ui.guild
{
import com.company.assembleegameclient.game.AGameSprite;
import com.company.assembleegameclient.game.events.GuildResultEvent;
import com.company.assembleegameclient.objects.Player;
import com.company.assembleegameclient.screens.TitleMenuOption;
import com.company.assembleegameclient.ui.dialogs.Dialog;
import com.company.rotmg.graphics.ScreenGraphic;
import flash.display.Sprite;
import flash.events.Event;
import flash.events.KeyboardEvent;
import flash.events.MouseEvent;
import flash.text.TextFieldAutoSize;
import kabam.rotmg.text.model.TextKey;
import kabam.rotmg.text.view.TextFieldDisplayConcrete;

public class GuildChronicleScreen extends Sprite
{


    private var gs_:AGameSprite;

    private var container:Sprite;

    private var guildPlayerList_:GuildPlayerList;

    private var continueButton_:TitleMenuOption;

    public function GuildChronicleScreen(param1:AGameSprite)
    {
        super();
        this.gs_ = param1;
        graphics.clear();
        graphics.beginFill(2829099,0.8);
        graphics.drawRect(0,0,800,600);
        graphics.endFill();
        addChild(this.container = new Sprite());
        this.addList();
        addChild(new ScreenGraphic());
        this.continueButton_ = new TitleMenuOption(TextKey.OPTIONS_CONTINUE_BUTTON,36,false);
        this.continueButton_.setAutoSize(TextFieldAutoSize.CENTER);
        this.continueButton_.setVerticalAlign(TextFieldDisplayConcrete.MIDDLE);
        this.continueButton_.addEventListener(MouseEvent.CLICK,this.onContinueClick);
        addChild(this.continueButton_);
        addEventListener(Event.ADDED_TO_STAGE,this.onAddedToStage);
        addEventListener(Event.REMOVED_FROM_STAGE,this.onRemovedFromStage);
    }

    private function addList() : void
    {
        if(this.guildPlayerList_ && this.guildPlayerList_.parent)
        {
            this.container.removeChild(this.guildPlayerList_);
        }
        var _loc1_:Player = this.gs_.map.player_;
        this.guildPlayerList_ = new GuildPlayerList(50,0,_loc1_ == null?"":_loc1_.name_,_loc1_.guildRank_);
        this.guildPlayerList_.addEventListener(GuildPlayerListEvent.SET_RANK,this.onSetRank);
        this.guildPlayerList_.addEventListener(GuildPlayerListEvent.REMOVE_MEMBER,this.onRemoveMember);
        this.container.addChild(this.guildPlayerList_);
    }

    private function removeList() : void
    {
        this.guildPlayerList_.removeEventListener(GuildPlayerListEvent.SET_RANK,this.onSetRank);
        this.guildPlayerList_.removeEventListener(GuildPlayerListEvent.REMOVE_MEMBER,this.onRemoveMember);
        this.container.removeChild(this.guildPlayerList_);
        this.guildPlayerList_ = null;
    }

    private function showError(param1:String) : void
    {
        var _loc2_:Dialog = new Dialog(TextKey.GUILD_CHRONICLE_LEFT,param1,TextKey.GUILD_CHRONICLE_RIGHT,null,"/guildError");
        _loc2_.addEventListener(Dialog.LEFT_BUTTON,this.onErrorTextDone);
        stage.addChild(_loc2_);
    }

    private function close() : void
    {
        stage.focus = null;
        parent.removeChild(this);
    }

    private function onSetRank(param1:GuildPlayerListEvent) : void
    {
        this.removeList();
        this.gs_.addEventListener(GuildResultEvent.EVENT,this.onSetRankResult);
        this.gs_.gsc_.changeGuildRank(param1.name_,param1.rank_);
    }

    private function onSetRankResult(param1:GuildResultEvent) : void
    {
        this.gs_.removeEventListener(GuildResultEvent.EVENT,this.onSetRankResult);
        if(!param1.success_)
        {
            this.showError(param1.errorKey);
        }
        else
        {
            this.addList();
        }
    }

    private function onRemoveMember(param1:GuildPlayerListEvent) : void
    {
        this.removeList();
        this.gs_.addEventListener(GuildResultEvent.EVENT,this.onRemoveResult);
        this.gs_.gsc_.guildRemove(param1.name_);
    }

    private function onRemoveResult(param1:GuildResultEvent) : void
    {
        this.gs_.removeEventListener(GuildResultEvent.EVENT,this.onRemoveResult);
        if(!param1.success_)
        {
            this.showError(param1.errorKey);
        }
        else
        {
            this.addList();
        }
    }

    private function onErrorTextDone(param1:Event) : void
    {
        var _loc2_:Dialog = param1.currentTarget as Dialog;
        stage.removeChild(_loc2_);
        this.addList();
    }

    private function onContinueClick(param1:MouseEvent) : void
    {
        this.close();
    }

    private function onAddedToStage(param1:Event) : void
    {
        this.continueButton_.x = 800 / 2;
        this.continueButton_.y = 550;
        stage.addEventListener(KeyboardEvent.KEY_DOWN,this.onKeyDown,false,1);
        stage.addEventListener(KeyboardEvent.KEY_UP,this.onKeyUp,false,1);
    }

    private function onRemovedFromStage(param1:Event) : void
    {
        stage.removeEventListener(KeyboardEvent.KEY_DOWN,this.onKeyDown,false);
        stage.removeEventListener(KeyboardEvent.KEY_UP,this.onKeyUp,false);
    }

    private function onKeyDown(param1:KeyboardEvent) : void
    {
        param1.stopImmediatePropagation();
    }

    private function onKeyUp(param1:KeyboardEvent) : void
    {
        param1.stopImmediatePropagation();
    }
}
}

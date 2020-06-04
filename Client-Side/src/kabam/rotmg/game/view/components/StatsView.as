package kabam.rotmg.game.view.components {
import com.company.assembleegameclient.objects.Player;

import flash.display.Sprite;
import flash.events.MouseEvent;
import flash.filters.GlowFilter;

import kabam.rotmg.game.model.StatModel;
import kabam.rotmg.text.model.TextKey;

import org.osflash.signals.natives.NativeSignal;

public class StatsView extends Sprite {

    private static const statsModel:Array = [new StatModel(TextKey.STAT_MODEL_ATTACK_SHORT,TextKey.STAT_MODEL_ATTACK_LONG,TextKey.STAT_MODEL_ATTACK_DESCRIPTION,true),new StatModel(TextKey.STAT_MODEL_DEFENSE_SHORT,TextKey.STAT_MODEL_DEFENSE_LONG,TextKey.STAT_MODEL_DEFENSE_DESCRIPTION,false),new StatModel(TextKey.STAT_MODEL_SPEED_SHORT,TextKey.STAT_MODEL_SPEED_LONG,TextKey.STAT_MODEL_SPEED_DESCRIPTION,true),new StatModel(TextKey.STAT_MODEL_DEXTERITY_SHORT,TextKey.STAT_MODEL_DEXTERITY_LONG,TextKey.STAT_MODEL_DEXTERITY_DESCRIPTION,true),new StatModel(TextKey.STAT_MODEL_VITALITY_SHORT,TextKey.STAT_MODEL_VITALITY_LONG,TextKey.STAT_MODEL_VITALITY_DESCRIPTION,true),new StatModel(TextKey.STAT_MODEL_WISDOM_SHORT,TextKey.STAT_MODEL_WISDOM_LONG,TextKey.STAT_MODEL_WISDOM_DESCRIPTION,true)];
    public static const ATTACK:int = 0;
    public static const DEFENSE:int = 1;
    public static const SPEED:int = 2;
    public static const DEXTERITY:int = 3;
    public static const VITALITY:int = 4;
    public static const WISDOM:int = 5;
    public static const STATE_UNDOCKED:String = "state_undocked";
    public static const STATE_DOCKED:String = "state_docked";
    public static const STATE_DEFAULT:String = STATE_DOCKED;


    private const WIDTH:int = 188.0;

    private const HEIGHT:int = 45;

    private var background:Sprite;

    public var stats_:Vector.<StatView>;

    public var containerSprite:Sprite;

    public var mouseDown:NativeSignal;

    public var currentState:String = "state_docked";

    public function StatsView()
    {
        this.background = this.createBackground();
        this.stats_ = new Vector.<StatView>();
        this.containerSprite = new Sprite();
        super();
        addChild(this.background);
        addChild(this.containerSprite);
        this.createStats();
        mouseChildren = false;
        this.mouseDown = new NativeSignal(this,MouseEvent.MOUSE_DOWN,MouseEvent);
    }

    private function createStats() : void
    {
        var _loc3_:StatView = null;
        var _loc1_:int = 0;
        var _loc2_:int = 0;
        while(_loc2_ < statsModel.length)
        {
            _loc3_ = this.createStat(_loc2_,_loc1_);
            this.stats_.push(_loc3_);
            this.containerSprite.addChild(_loc3_);
            _loc1_ = _loc1_ + _loc2_ % 2;
            _loc2_++;
        }
    }

    private function createStat(param1:int, param2:int) : StatView
    {
        var _loc4_:StatView = null;
        var _loc3_:StatModel = statsModel[param1];
        _loc4_ = new StatView(_loc3_.name,_loc3_.abbreviation,_loc3_.description,_loc3_.redOnZero);
        _loc4_.x = param1 % 2 * this.WIDTH / 2;
        _loc4_.y = param2 * (this.HEIGHT / 3);
        return _loc4_;
    }

    public function draw(param1:Player, param2:Boolean = true) : void
    {
        if(param1)
        {
            this.setBackgroundVisibility();
            this.drawStats(param1);
        }
        if(param2)
        {
            this.containerSprite.x = (this.WIDTH - this.containerSprite.width) / 2;
        }
    }

    private function drawStats(param1:Player) : void
    {
        this.stats_[ATTACK].draw(param1.attack_,param1.attackBoost_,param1.attackMax_,param1.level_);
        this.stats_[DEFENSE].draw(param1.defense_,param1.defenseBoost_,param1.defenseMax_,param1.level_);
        this.stats_[SPEED].draw(param1.speed_,param1.speedBoost_,param1.speedMax_,param1.level_);
        this.stats_[DEXTERITY].draw(param1.dexterity_,param1.dexterityBoost_,param1.dexterityMax_,param1.level_);
        this.stats_[VITALITY].draw(param1.vitality_,param1.vitalityBoost_,param1.vitalityMax_,param1.level_);
        this.stats_[WISDOM].draw(param1.wisdom_,param1.wisdomBoost_,param1.wisdomMax_,param1.level_);
    }

    public function dock() : void
    {
        this.currentState = STATE_DOCKED;
    }

    public function undock() : void
    {
        this.currentState = STATE_UNDOCKED;
    }

    private function createBackground() : Sprite
    {
        this.background = new Sprite();
        this.background.graphics.clear();
        this.background.graphics.beginFill(3552822);
        this.background.graphics.lineStyle(2,16777215);
        this.background.graphics.drawRoundRect(-5,-5,this.WIDTH + 10,this.HEIGHT + 13,10);
        this.background.filters = [new GlowFilter(0,1,10,10,1,3)];
        return this.background;
    }

    private function setBackgroundVisibility() : void
    {
        if(this.currentState == STATE_UNDOCKED)
        {
            this.background.alpha = 1;
        }
        else if(this.currentState == STATE_DOCKED)
        {
            this.background.alpha = 0;
        }
    }


}
}

﻿package kabam.rotmg.ui.view {
import com.company.assembleegameclient.objects.Player;
import com.company.assembleegameclient.ui.ExperienceBoostTimerPopup;
import com.company.assembleegameclient.ui.StatusBar;

import flash.display.Sprite;
import flash.events.Event;

import kabam.rotmg.text.model.TextKey;

public class StatMetersView extends Sprite {

    private var expBar_:StatusBar;
    private var fameBar_:StatusBar;
    private var hpBar_:StatusBar;
    private var mpBar_:StatusBar;
    private var areTempXpListenersAdded:Boolean;
    private var curXPBoost:int;
    private var expTimer:ExperienceBoostTimerPopup;

    public function StatMetersView() {
        this.expBar_ = new StatusBar(90, 16, 5931045, 0x545454, TextKey.EXP_BAR_LEVEL);
        this.fameBar_ = new StatusBar(90, 16, 0xE25F00, 0x545454, TextKey.CURRENCY_FAME);
        this.hpBar_ = new StatusBar(235, 16, 14693428, 0x545454, TextKey.STATUS_BAR_HEALTH_POINTS);
        this.mpBar_ = new StatusBar(136, 16, 6325472, 0x545454, TextKey.STATUS_BAR_MANA_POINTS);
        this.hpBar_.y = 27;
        this.hpBar_.x = 35;
        this.mpBar_.y = 51;
        this.mpBar_.x = 35;
        this.expBar_.x = mpBar_.width + 180;
        this.expBar_.y = mpBar_.y;
        this.fameBar_.x = mpBar_.width + 180;
        this.fameBar_.y = mpBar_.y;
        this.expBar_.visible = true;
        this.fameBar_.visible = false;
        addChild(this.expBar_);
        addChild(this.fameBar_);
        addChild(this.hpBar_);
        addChild(this.mpBar_);
    }

    public function update(_arg1:Player):void {
        this.expBar_.setLabelText(TextKey.EXP_BAR_LEVEL, {"level": _arg1.level_});
        if (_arg1.level_ != 20) {
            if (this.expTimer) {
                this.expTimer.update(_arg1.xpTimer);
            }
            if (!this.expBar_.visible) {
                this.expBar_.visible = true;
                this.fameBar_.visible = false;
            }
            this.expBar_.draw(_arg1.exp_, _arg1.nextLevelExp_, 0);
            if (this.curXPBoost != _arg1.xpBoost_) {
                this.curXPBoost = _arg1.xpBoost_;
                if (this.curXPBoost) {
                    this.expBar_.showMultiplierText();
                }
                else {
                    this.expBar_.hideMultiplierText();
                }
            }
            if (_arg1.xpTimer) {
                if (!this.areTempXpListenersAdded) {
                    this.expBar_.addEventListener("MULTIPLIER_OVER", this.onExpBarOver);
                    this.expBar_.addEventListener("MULTIPLIER_OUT", this.onExpBarOut);
                    this.areTempXpListenersAdded = true;
                }
            }
            else {
                if (this.areTempXpListenersAdded) {
                    this.expBar_.removeEventListener("MULTIPLIER_OVER", this.onExpBarOver);
                    this.expBar_.removeEventListener("MULTIPLIER_OUT", this.onExpBarOut);
                    this.areTempXpListenersAdded = false;
                }
                if (((this.expTimer) && (this.expTimer.parent))) {
                    removeChild(this.expTimer);
                    this.expTimer = null;
                }
            }
        }
        else {
            if (!this.fameBar_.visible) {
                this.fameBar_.visible = true;
                this.expBar_.visible = false;
            }
            this.fameBar_.draw(_arg1.currFame_, _arg1.nextClassQuestFame_, 0);
        }
        this.hpBar_.draw(_arg1.hp_, _arg1.maxHP_, _arg1.maxHPBoost_, _arg1.maxHPMax_);
        this.mpBar_.draw(_arg1.mp_, _arg1.maxMP_, _arg1.maxMPBoost_, _arg1.maxMPMax_);
    }

    private function onExpBarOver(_arg1:Event):void {
        addChild((this.expTimer = new ExperienceBoostTimerPopup()));
    }

    private function onExpBarOut(_arg1:Event):void {
        if (((this.expTimer) && (this.expTimer.parent))) {
            removeChild(this.expTimer);
            this.expTimer = null;
        }
    }


}
}

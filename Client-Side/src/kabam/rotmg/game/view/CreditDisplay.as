package kabam.rotmg.game.view {
import com.company.assembleegameclient.game.GameSprite;
import com.company.assembleegameclient.parameters.Parameters;
import com.company.assembleegameclient.util.FameUtil;
import com.company.assembleegameclient.util.TextureRedrawer;
import com.company.assembleegameclient.util.TimeUtil;
import com.company.util.AssetLibrary;

import flash.display.Bitmap;
import flash.display.BitmapData;
import flash.display.Sprite;
import flash.events.MouseEvent;
import flash.filters.DropShadowFilter;

import kabam.rotmg.assets.services.IconFactory;
import kabam.rotmg.text.view.TextFieldDisplayConcrete;
import kabam.rotmg.text.view.stringBuilder.StaticStringBuilder;
import kabam.rotmg.ui.view.SignalWaiter;

import org.osflash.signals.Signal;

public class CreditDisplay extends Sprite {

    private static const FONT_SIZE:int = 18;
    public static const IMAGE_NAME:String = "lofiObj3";
    public static const IMAGE_ID:int = 225;
    public static const waiter:SignalWaiter = new SignalWaiter();

    private var creditsText_:TextFieldDisplayConcrete;
    private var fameText_:TextFieldDisplayConcrete;
    private var prestigeText_:TextFieldDisplayConcrete;
    private var coinIcon_:Bitmap;
    private var fameIcon_:Bitmap;
    private var prestigeIcon_:Bitmap;
    private var credits_:int = -1;
    private var fame_:int = -1;
    private var prestige_:int = -1;
    private var displayFame_:Boolean = true;
    private var gs:GameSprite;
    public var openAccountDialog:Signal;

    public function CreditDisplay(_arg1:GameSprite = null, _arg2:Boolean = true,_arg4:Number = 0) {
        this.openAccountDialog = new Signal();
        super();
        this.displayFame_ = _arg2;
        this.gs = _arg1;
        this.creditsText_ = this.makeTextField();
        waiter.push(this.creditsText_.textChanged);
        addChild(this.creditsText_);
        this.prestigeText_ = this.makeTextField();
        waiter.push(this.prestigeText_.textChanged);
        addChild(this.prestigeText_);
        this.prestigeIcon_ = new Bitmap(IconFactory.makePrestige());
        addChild(this.prestigeIcon_);
        var _local5:BitmapData = AssetLibrary.getImageFromSet(IMAGE_NAME, IMAGE_ID);
        _local5 = TextureRedrawer.redraw(_local5, 40, true, 0);
        this.coinIcon_ = new Bitmap(_local5);
        addChild(this.coinIcon_);
        if (this.displayFame_) {
            this.fameText_ = this.makeTextField();
            waiter.push(this.fameText_.textChanged);
            addChild(this.fameText_);
            this.fameIcon_ = new Bitmap(FameUtil.getFameIcon());
            addChild(this.fameIcon_);
        }
        this.draw(0, 0, 0,0);
        mouseEnabled = true;
        doubleClickEnabled = true;
        addEventListener(MouseEvent.DOUBLE_CLICK, this.onDoubleClick, false, 0, true);
        waiter.complete.add(this.onAlignHorizontal);
    }

    private function onAlignHorizontal():void {
            this.coinIcon_.x = -(this.coinIcon_.width);
            this.creditsText_.x = ((this.coinIcon_.x - this.creditsText_.width) + 8);
            this.creditsText_.y = ((this.coinIcon_.y + (this.coinIcon_.height / 2)) - (this.creditsText_.height / 2));

            this.prestigeIcon_.x = (this.coinIcon_.x + 10);
            this.prestigeIcon_.y = (this.coinIcon_.y + 30);
            this.prestigeText_.x = ((this.prestigeIcon_.x - this.prestigeText_.width) - 2);
            this.prestigeText_.y = ((this.prestigeIcon_.y + (this.prestigeIcon_.height / 2)) - (this.prestigeText_.height / 2));
        if (this.displayFame_) {
            this.fameIcon_.x = (this.creditsText_.x - this.fameIcon_.width);
            this.fameText_.x = ((this.fameIcon_.x - this.fameText_.width) + 8);
            this.fameText_.y = ((this.fameIcon_.y + (this.fameIcon_.height / 2)) - (this.fameText_.height / 2));
        }
    }

    private function onDoubleClick(_arg1:MouseEvent):void {
        if (((((!(this.gs)) || (this.gs.evalIsNotInCombatMapArea()))) || ((Parameters.data_.clickForGold == true)))) {
            this.openAccountDialog.dispatch();
        }
    }

    public function makeTextField(_arg1:uint = 0xFFFFFF):TextFieldDisplayConcrete {
        var _local2:TextFieldDisplayConcrete = new TextFieldDisplayConcrete().setSize(FONT_SIZE).setColor(_arg1).setTextHeight(16);
        _local2.filters = [new DropShadowFilter(0, 0, 0, 1, 4, 4, 2)];
        return (_local2);
    }

    public function draw(_arg1:int, _arg2:int, _arg3:int, _arg4:int = 0) : void
    {
        if ((((((_arg1 == this.credits_)) && (((this.displayFame_) && ((_arg2 == this.fame_)))))) && ((_arg4 == this.prestige_)) )) {
            return;
        }
        this.credits_ = _arg1;
        this.creditsText_.setStringBuilder(new StaticStringBuilder(this.credits_.toString()));
        this.prestige_ = _arg4;
        this.prestigeText_.setStringBuilder(new StaticStringBuilder(this.prestige_.toString()));
        if (this.displayFame_) {
            this.fame_ = _arg2;
            this.fameText_.setStringBuilder(new StaticStringBuilder(this.fame_.toString()));
        }
        if (waiter.isEmpty()) {
            this.onAlignHorizontal();
        }
    }


}
}

package com.company.assembleegameclient.screens
{
import com.company.assembleegameclient.ui.Scrollbar;
import flash.display.Graphics;
import flash.display.Shape;
import flash.display.Sprite;
import flash.events.Event;
import flash.filters.DropShadowFilter;
import kabam.rotmg.servers.api.Server;
import kabam.rotmg.text.model.TextKey;
import kabam.rotmg.text.view.TextFieldDisplayConcrete;
import kabam.rotmg.text.view.stringBuilder.LineBuilder;
import kabam.rotmg.ui.view.ButtonFactory;
import kabam.rotmg.ui.view.components.MenuOptionsBar;
import kabam.rotmg.ui.view.components.ScreenBase;
import org.osflash.signals.Signal;

public class ServersScreen extends Sprite
{


    public var gotoTitle:Signal;

    private var selectServerText_:TextFieldDisplayConcrete;

    private var lines_:Shape;

    private var content_:Sprite;

    private var serverBoxes_:ServerBoxes;

    private var scrollBar_:Scrollbar;

    private var servers:Vector.<Server>;

    public function ServersScreen()
    {
        super();
        addChild(new ScreenBase());
        this.gotoTitle = new Signal();
        addChild(new ScreenBase());
        addChild(new AccountScreen());
    }

    public function initialize(param1:Vector.<Server>) : void
    {
        this.servers = param1;
        this.makeSelectServerText();
        this.makeLines();
        this.makeContainer();
        this.makeServerBoxes();
        this.serverBoxes_.height > 400 && this.makeScrollbar();
        this.makeMenuBar();
    }

    private function makeMenuBar() : void
    {
        var _loc1_:MenuOptionsBar = new MenuOptionsBar();
        var _loc2_:TitleMenuOption = ButtonFactory.getDoneButton();
        _loc1_.addButton(_loc2_,MenuOptionsBar.CENTER);
        _loc2_.clicked.add(this.onDone);
        addChild(_loc1_);
    }

    private function makeScrollbar() : void
    {
        this.scrollBar_ = new Scrollbar(16,400);
        this.scrollBar_.x = 800 - this.scrollBar_.width - 4;
        this.scrollBar_.y = 104;
        this.scrollBar_.setIndicatorSize(400,this.serverBoxes_.height);
        this.scrollBar_.addEventListener(Event.CHANGE,this.onScrollBarChange);
        addChild(this.scrollBar_);
    }

    private function makeServerBoxes() : void
    {
        this.serverBoxes_ = new ServerBoxes(this.servers);
        this.serverBoxes_.y = 8;
        this.serverBoxes_.addEventListener(Event.COMPLETE,this.onDone);
        this.content_.addChild(this.serverBoxes_);
    }

    private function makeContainer() : void
    {
        this.content_ = new Sprite();
        this.content_.x = 4;
        this.content_.y = 100;
        var _loc1_:Shape = new Shape();
        _loc1_.graphics.beginFill(16777215);
        _loc1_.graphics.drawRect(0,0,776,430);
        _loc1_.graphics.endFill();
        this.content_.addChild(_loc1_);
        this.content_.mask = _loc1_;
        addChild(this.content_);
    }

    private function makeLines() : void
    {
        this.lines_ = new Shape();
        var _loc1_:Graphics = this.lines_.graphics;
        _loc1_.clear();
        _loc1_.lineStyle(2,5526612);
        _loc1_.moveTo(0,100);
        _loc1_.lineTo(800,100);
        _loc1_.lineStyle();
        addChild(this.lines_);
    }

    private function makeSelectServerText() : void
    {
        this.selectServerText_ = new TextFieldDisplayConcrete().setSize(18).setColor(11776947).setBold(true);
        this.selectServerText_.setStringBuilder(new LineBuilder().setParams(TextKey.SERVERS_SELECT));
        this.selectServerText_.filters = [new DropShadowFilter(0,0,0,1,8,8)];
        this.selectServerText_.x = 18;
        this.selectServerText_.y = 72;
        addChild(this.selectServerText_);
    }

    private function onDone() : void
    {
        this.gotoTitle.dispatch();
    }

    private function onScrollBarChange(param1:Event) : void
    {
        this.serverBoxes_.y = 8 - this.scrollBar_.pos() * (this.serverBoxes_.height - 400);
    }
}
}

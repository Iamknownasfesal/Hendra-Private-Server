package com.company.assembleegameclient.ui.menu
{
import com.company.assembleegameclient.game.GameSprite;
import com.company.assembleegameclient.parameters.Parameters;
import com.company.util.GraphicsUtil;
import com.company.util.RectangleUtil;
import flash.display.CapsStyle;
import flash.display.DisplayObjectContainer;
import flash.display.GraphicsPath;
import flash.display.GraphicsSolidFill;
import flash.display.GraphicsStroke;
import flash.display.IGraphicsData;
import flash.display.JointStyle;
import flash.display.LineScaleMode;
import flash.display.Sprite;
import flash.events.Event;
import flash.events.MouseEvent;
import flash.filters.DropShadowFilter;
import flash.geom.Rectangle;
import kabam.rotmg.ui.view.UnFocusAble;

public class Menu extends Sprite implements UnFocusAble
{


    protected var yOffset:int;

    private var backgroundFill_:GraphicsSolidFill;

    private var outlineFill_:GraphicsSolidFill;

    private var lineStyle_:GraphicsStroke;

    private var path_:GraphicsPath;

    private var graphicsData_:Vector.<IGraphicsData>;

    private var background_:uint;

    private var outline_:uint;

    public function Menu(param1:uint, param2:uint)
    {
        this.backgroundFill_ = new GraphicsSolidFill(0,1);
        this.outlineFill_ = new GraphicsSolidFill(0,1);
        this.lineStyle_ = new GraphicsStroke(1,false,LineScaleMode.NORMAL,CapsStyle.NONE,JointStyle.ROUND,3,this.outlineFill_);
        this.path_ = new GraphicsPath(new Vector.<int>(),new Vector.<Number>());
        this.graphicsData_ = new <IGraphicsData>[this.lineStyle_,this.backgroundFill_,this.path_,GraphicsUtil.END_FILL,GraphicsUtil.END_STROKE];
        super();
        this.background_ = param1;
        this.outline_ = param2;
        this.yOffset = 40;
        filters = [new DropShadowFilter(0,0,0,1,16,16)];
        addEventListener(Event.ADDED_TO_STAGE,this.onAddedToStage);
        addEventListener(Event.REMOVED_FROM_STAGE,this.onRemovedFromStage);
    }

    public function remove() : void
    {
        if(parent != null)
        {
            parent.removeChild(this);
        }
    }

    public function scaleParent(param1:Boolean) : void
    {
        var _loc2_:DisplayObjectContainer = null;
        if(this.parent is GameSprite)
        {
            _loc2_ = this;
        }
        else
        {
            _loc2_ = this.parent;
        }
        var _loc3_:Number = 800 / stage.stageWidth;
        var _loc4_:Number = 600 / stage.stageHeight;
        if(param1 == true)
        {
            _loc2_.scaleX = _loc3_ / _loc4_;
            _loc2_.scaleY = 1;
        }
        else
        {
            _loc2_.scaleX = _loc3_;
            _loc2_.scaleY = _loc4_;
        }
    }

    public function positionFixed() : void
    {
        var _loc1_:Number = NaN;
        var _loc2_:Boolean = Parameters.data_.uiscale;
        var _loc3_:Number = (stage.stageWidth - 800) / 2 + stage.mouseX;
        _loc1_ = (stage.stageHeight - 600) / 2 + stage.mouseY;
        var _loc4_:Number = 600 / stage.stageHeight;
        this.scaleParent(_loc2_);
        if(_loc2_)
        {
            _loc3_ = _loc3_ * _loc4_;
            _loc1_ = _loc1_ * _loc4_;
        }
        if(stage == null)
        {
            return;
        }
        if(stage.mouseX + 0.5 * stage.stageWidth - 400 < stage.stageWidth / 2)
        {
            x = _loc3_ + 12;
        }
        else
        {
            x = _loc3_ - width - 1;
        }
        if(x < 12)
        {
            x = 12;
        }
        if(stage.mouseY + 0.5 * stage.stageHeight - 300 < stage.stageHeight / 3)
        {
            y = _loc1_ + 12;
        }
        else
        {
            y = _loc1_ - height - 1;
        }
        if(y < 12)
        {
            y = 12;
        }
    }

    protected function addOption(param1:MenuOption) : void
    {
        param1.x = 8;
        param1.y = this.yOffset;
        addChild(param1);
        this.yOffset = this.yOffset + 28;
    }

    protected function draw() : void
    {
        this.backgroundFill_.color = this.background_;
        this.outlineFill_.color = this.outline_;
        graphics.clear();
        GraphicsUtil.clearPath(this.path_);
        GraphicsUtil.drawCutEdgeRect(-6,-6,Math.max(154,width + 12),height + 12,4,[1,1,1,1],this.path_);
        graphics.drawGraphicsData(this.graphicsData_);
    }

    private function position() : void
    {
        this.positionFixed();
    }

    protected function onAddedToStage(param1:Event) : void
    {
        this.draw();
        this.position();
        addEventListener(Event.ENTER_FRAME,this.onEnterFrame);
        addEventListener(MouseEvent.ROLL_OUT,this.onRollOut);
    }

    protected function onRemovedFromStage(param1:Event) : void
    {
        this.parent.scaleX = 1;
        this.parent.scaleY = 1;
        removeEventListener(Event.ENTER_FRAME,this.onEnterFrame);
        removeEventListener(MouseEvent.ROLL_OUT,this.onRollOut);
    }

    protected function onEnterFrame(param1:Event) : void
    {
        if(stage == null)
        {
            return;
        }
        this.scaleParent(Parameters.data_.uiscale);
        var _loc2_:Rectangle = getRect(stage);
        var _loc3_:Number = RectangleUtil.pointDist(_loc2_,stage.mouseX,stage.mouseY);
        if(_loc3_ > 40)
        {
            this.remove();
        }
    }

    protected function onRollOut(param1:Event) : void
    {
        this.remove();
    }
}
}

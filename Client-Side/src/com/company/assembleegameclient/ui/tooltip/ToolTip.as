package com.company.assembleegameclient.ui.tooltip
{
import com.company.assembleegameclient.map.partyoverlay.PlayerArrow;
import com.company.assembleegameclient.parameters.Parameters;
import com.company.assembleegameclient.ui.options.Options;
import com.company.util.GraphicsUtil;
import flash.display.CapsStyle;
import flash.display.DisplayObject;
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
import kabam.rotmg.tooltips.view.TooltipsView;
import kabam.rotmg.ui.view.SignalWaiter;

public class ToolTip extends Sprite
{


    protected const waiter:SignalWaiter = new SignalWaiter();

    public var contentWidth_:int;

    public var contentHeight_:int;

    private var background_:uint;

    private var backgroundAlpha_:Number;

    private var outline_:uint;

    private var outlineAlpha_:Number;

    private var followMouse_:Boolean;

    private var forcePositionLeft_:Boolean = false;

    private var forcePositionRight_:Boolean = false;

    private var targetObj:DisplayObject;

    private var backgroundFill_:GraphicsSolidFill;

    private var outlineFill_:GraphicsSolidFill;

    private var lineStyle_:GraphicsStroke;

    private var path_:GraphicsPath;

    private var graphicsData_:Vector.<IGraphicsData>;

    public function ToolTip(param1:uint, param2:Number, param3:uint, param4:Number, param5:Boolean = true)
    {
        this.backgroundFill_ = new GraphicsSolidFill(0,1);
        this.outlineFill_ = new GraphicsSolidFill(16777215,1);
        this.lineStyle_ = new GraphicsStroke(1,false,LineScaleMode.NORMAL,CapsStyle.NONE,JointStyle.ROUND,3,this.outlineFill_);
        this.path_ = new GraphicsPath(new Vector.<int>(),new Vector.<Number>());
        this.graphicsData_ = new <IGraphicsData>[this.lineStyle_,this.backgroundFill_,this.path_,GraphicsUtil.END_FILL,GraphicsUtil.END_STROKE];
        super();
        this.background_ = param1;
        this.backgroundAlpha_ = param2;
        this.outline_ = param3;
        this.outlineAlpha_ = param4;
        this.followMouse_ = param5;
        mouseEnabled = false;
        mouseChildren = false;
        filters = [new DropShadowFilter(0,0,0,1,16,16)];
        addEventListener(Event.ADDED_TO_STAGE,this.onAddedToStage);
        addEventListener(Event.REMOVED_FROM_STAGE,this.onRemovedFromStage);
        this.waiter.complete.add(this.alignUIAndDraw);
    }

    public function attachToTarget(param1:DisplayObject) : void
    {
        if(param1)
        {
            this.targetObj = param1;
            this.targetObj.addEventListener(MouseEvent.ROLL_OUT,this.onLeaveTarget);
        }
    }

    public function detachFromTarget() : void
    {
        if(this.targetObj)
        {
            this.targetObj.removeEventListener(MouseEvent.ROLL_OUT,this.onLeaveTarget);
            if(parent)
            {
                parent.removeChild(this);
            }
            this.targetObj = null;
        }
    }

    public function forcePostionLeft() : void
    {
        this.forcePositionLeft_ = true;
        this.forcePositionRight_ = false;
    }

    public function forcePostionRight() : void
    {
        this.forcePositionRight_ = true;
        this.forcePositionLeft_ = false;
    }

    public function draw() : void
    {
        this.backgroundFill_.color = this.background_;
        this.backgroundFill_.alpha = this.backgroundAlpha_;
        this.outlineFill_.color = this.outline_;
        this.outlineFill_.alpha = this.outlineAlpha_;
        graphics.clear();
        this.contentWidth_ = width;
        this.contentHeight_ = height;
        GraphicsUtil.clearPath(this.path_);
        GraphicsUtil.drawCutEdgeRect(-6,-6,this.contentWidth_ + 12,this.contentHeight_ + 12,4,[1,1,1,1],this.path_);
        graphics.drawGraphicsData(this.graphicsData_);
    }

    public function positionFixed() : void
    {
        var _loc1_:Number = NaN;
        var _loc2_:Number = NaN;
        var _loc3_:Number = 800 / stage.stageWidth;
        var _loc4_:Number = 600 / stage.stageHeight;
        if(this.parent is Options)
        {
            _loc1_ = (stage.mouseX + stage.stageWidth / 2 - 400) / stage.stageWidth * 800;
            _loc2_ = (stage.mouseY + stage.stageHeight / 2 - 300) / stage.stageHeight * 600;
        }
        else
        {
            _loc1_ = (stage.stageWidth - 800) / 2 + stage.mouseX;
            _loc2_ = (stage.stageHeight - 600) / 2 + stage.mouseY;
            if(this.parent is TooltipsView || this is PlayerGroupToolTip && !(this.parent is PlayerArrow))
            {
                if(Parameters.data_.uiscale)
                {
                    this.parent.scaleX = _loc3_ / _loc4_;
                    this.parent.scaleY = 1;
                    _loc1_ = _loc1_ * _loc4_;
                    _loc2_ = _loc2_ * _loc4_;
                }
                else
                {
                    this.parent.scaleX = _loc3_;
                    this.parent.scaleY = _loc4_;
                }
            }
        }
        if(stage == null)
        {
            return;
        }
        if(stage.mouseX + 0.5 * stage.stageWidth - 400 < stage.stageWidth / 2)
        {
            x = _loc1_ + 12;
        }
        else
        {
            x = _loc1_ - width - 1;
        }
        if(x < 12)
        {
            x = 12;
        }
        if(stage.mouseY + 0.5 * stage.stageHeight - 300 < stage.stageHeight / 3)
        {
            y = _loc2_ + 12;
        }
        else
        {
            y = _loc2_ - height - 1;
        }
        if(y < 12)
        {
            y = 12;
        }
    }

    protected function alignUI() : void
    {
    }

    protected function position() : void
    {
        this.positionFixed();
    }

    private function alignUIAndDraw() : void
    {
        this.alignUI();
        this.draw();
        this.position();
    }

    public function drawOnEnterFrame(param1:Event) : void
    {
        this.draw();
    }

    private function onLeaveTarget(param1:MouseEvent) : void
    {
        this.detachFromTarget();
    }

    private function onAddedToStage(param1:Event) : void
    {
        if(this.waiter.isEmpty())
        {
            this.draw();
        }
        if(this.followMouse_)
        {
            this.position();
            addEventListener(Event.ENTER_FRAME,this.onEnterFrame);
        }
        if(!this.followMouse_ && Parameters.data_.QuestToolTipMod)
        {
            addEventListener(Event.ENTER_FRAME,this.drawOnEnterFrame);
        }
    }

    private function onRemovedFromStage(param1:Event) : void
    {
        if(this.followMouse_)
        {
            removeEventListener(Event.ENTER_FRAME,this.onEnterFrame);
        }
        else
        {
            removeEventListener(Event.ENTER_FRAME,this.drawOnEnterFrame);
        }
    }

    private function onEnterFrame(param1:Event) : void
    {
        this.position();
    }
}
}

﻿package com.company.assembleegameclient.mapeditor
{
import com.company.assembleegameclient.map.AnimateProperties;
import com.company.assembleegameclient.map.Camera;
import com.company.assembleegameclient.map.GroundLibrary;
import com.company.assembleegameclient.map.SquareFace;
import com.company.assembleegameclient.ui.tooltip.ToolTip;
import flash.display.BitmapData;
import flash.display.IGraphicsData;
import flash.display.Shape;
import flash.geom.Rectangle;

class GroundElement extends Element
{

    private static const VIN:Vector.<Number> = new <Number>[0,0,0,1,0,0,1,1,0,0,1,0];

    private static const SCALE:Number = 0.6;


    public var groundXML_:XML;

    private var tileShape_:Shape;

    function GroundElement(_arg1:XML)
    {
        super(int(_arg1.@type));
        this.groundXML_ = _arg1;
        var _local2:Vector.<IGraphicsData> = new Vector.<IGraphicsData>();
        var _local3:Camera = new Camera();
        _local3.configure(0.5,0.5,12,Math.PI / 4,new Rectangle(-100,-100,200,200));
        var _local4:BitmapData = GroundLibrary.getBitmapData(type_);
        var _local5:SquareFace = new SquareFace(_local4,VIN,0,0,AnimateProperties.NO_ANIMATE,0,0);
        _local5.draw(_local2,_local3,0);
        this.tileShape_ = new Shape();
        this.tileShape_.graphics.drawGraphicsData(_local2);
        this.tileShape_.scaleX = this.tileShape_.scaleY = SCALE;
        this.tileShape_.x = WIDTH / 2;
        this.tileShape_.y = HEIGHT / 2;
        addChild(this.tileShape_);
    }

    override protected function getToolTip() : ToolTip
    {
        return new GroundTypeToolTip(this.groundXML_);
    }
}
}

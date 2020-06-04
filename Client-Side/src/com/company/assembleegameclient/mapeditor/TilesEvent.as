﻿package com.company.assembleegameclient.mapeditor {
import com.company.util.IntPoint;

import flash.events.Event;

class TilesEvent extends Event {

    public static const TILES_EVENT:String = "TILES_EVENT";

    public var tiles_:Vector.<IntPoint>;

    public function TilesEvent(_arg1:Vector.<IntPoint>) {
        super(TILES_EVENT);
        this.tiles_ = _arg1;
    }

}
}

﻿package kabam.rotmg.messaging.impl.incoming.arena {
import flash.utils.IDataInput;

import kabam.rotmg.messaging.impl.incoming.IncomingMessage;

public class ArenaDeath extends IncomingMessage {

    public var cost:int;

    public function ArenaDeath(_arg1:uint, _arg2:Function) {
        super(_arg1, _arg2);
    }

    override public function parseFromInput(_arg1:IDataInput):void {
        this.cost = _arg1.readInt();
    }

    override public function toString():String {
        return (formatToString("ARENADEATH", "cost"));
    }


}
}

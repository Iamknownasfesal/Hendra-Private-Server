﻿package kabam.rotmg.messaging.impl.data {
import flash.utils.IDataInput;

public class ObjectData {

    public var objectType_:int;
    public var status_:ObjectStatusData;

    public function ObjectData() {
        this.status_ = new ObjectStatusData();
        super();
    }

    public function parseFromInput(_arg1:IDataInput):void {
        this.objectType_ = _arg1.readUnsignedShort();
        this.status_.parseFromInput(_arg1);
    }

    public function toString():String {
        return (((("objectType_: " + this.objectType_) + " status_: ") + this.status_));
    }


}
}

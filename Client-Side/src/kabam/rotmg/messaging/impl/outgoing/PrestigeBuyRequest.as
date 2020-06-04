package kabam.rotmg.messaging.impl.outgoing {
import flash.utils.IDataOutput;


public class PrestigeBuyRequest extends OutgoingMessage {
    public var BuyId_:int

    public function PrestigeBuyRequest(_arg_1:uint, _arg_2:Function) {
        super(_arg_1, _arg_2);
    }

    override public function writeToOutput(_arg1:IDataOutput):void {
        _arg1.writeInt(BuyId_);
    }

    override public function toString():String {
        return formatToString("PRESTIGEBUYREQUEST", "BuyId_");
    }
}
}

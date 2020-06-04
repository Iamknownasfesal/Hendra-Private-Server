//kabam.rotmg.chat.control.ParseChatMessageCommand

package kabam.rotmg.chat.control{
import com.company.assembleegameclient.parameters.Parameters;

import flash.display.DisplayObject;
import flash.display.StageScaleMode;
import flash.events.Event;

import kabam.rotmg.chat.model.ChatMessage;
import kabam.rotmg.game.signals.AddTextLineSignal;
import kabam.rotmg.text.model.TextKey;
import kabam.rotmg.ui.model.HUDModel;


public class ParseChatMessageCommand {

    [Inject]
    public var data:String;
    [Inject]
    public var hudModel:HUDModel;
    [Inject]
    public var addTextLine:AddTextLineSignal;

    public function execute():void{
        var _local1:Array;
        var _local2:Number;
        var _local3:Number;
        if (this.fsCommands(this.data))
        {
            return;
        }
        if (this.data.charAt(0) == "/"){
            _local1 = this.data.substr(1, this.data.length).split(" ");
            switch (_local1[0]){
                case "help":
                    this.addTextLine.dispatch(ChatMessage.make(Parameters.HELP_CHAT_NAME, TextKey.HELP_COMMAND));
                    return;
            };
        };
        this.hudModel.gameSprite.gsc_.playerText(this.data);
    }

    private function fsCommands(_arg_1:String):Boolean
    {
        _arg_1 = _arg_1.toLowerCase();
        var _local_4:* = undefined;
        var _local_2:DisplayObject = Parameters.root;
        if (_arg_1 == "/fs")
        {
            if (_local_2.stage.scaleMode == StageScaleMode.EXACT_FIT)
            {
                _local_2.stage.scaleMode = StageScaleMode.NO_SCALE;
                Parameters.data_.stageScale = StageScaleMode.NO_SCALE;
                this.addTextLine.dispatch(ChatMessage.make(Parameters.HELP_CHAT_NAME, "Fullscreen: On"));
            }
            else
            {
                _local_2.stage.scaleMode = StageScaleMode.EXACT_FIT;
                Parameters.data_.stageScale = StageScaleMode.EXACT_FIT;
                this.addTextLine.dispatch(ChatMessage.make(Parameters.HELP_CHAT_NAME, "Fullscreen: Off"));
            }
            Parameters.save();
            _local_2.dispatchEvent(new Event(Event.RESIZE));
            return (true);
        }
        if (_arg_1 == "/mscale")
        {
            this.addTextLine.dispatch(ChatMessage.make(Parameters.HELP_CHAT_NAME, (("Map Scale: " + Parameters.data_.mscale) + " - Usage: /mscale <any decimal number>.")));
            return (true);
        }
        var _local_3:Array = _arg_1.match("^/mscale (\\d*\\.*\\d+)$");
        if (_local_3 != null)
        {
            _local_4 = Number(_local_3[1]);
            _local_4 = _local_4 > 0.5? _local_4: 0.5;
            _local_4 = _local_4 < 5? _local_4: 5;
            Parameters.data_["mscale"] = _local_4;
            Parameters.save();
            _local_2.dispatchEvent(new Event(Event.RESIZE));
            this.addTextLine.dispatch(ChatMessage.make(Parameters.HELP_CHAT_NAME, ("Map Scale: " + _local_3[1])));
            return (true);
        }
        return (false);
    }

}
}//package kabam.rotmg.chat.control
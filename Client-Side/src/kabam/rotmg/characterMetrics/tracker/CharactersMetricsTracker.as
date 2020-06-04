package kabam.rotmg.characterMetrics.tracker {
import com.hurlant.util.Base64;
import flash.utils.Dictionary;
import flash.utils.IDataInput;
import kabam.rotmg.characterMetrics.data.CharacterMetricsData;

public class CharactersMetricsTracker
{

    public static const STATS_SIZE:int = 5;


    private var charactersStats:Dictionary;

    private var _lastUpdate:Date;

    public function CharactersMetricsTracker()
    {
        super();
    }

    public function setBinaryStringData(param1:int, param2:String) : void
    {
        var _loc3_:RegExp = /-/g;
        var _loc4_:RegExp = /_/g;
        var _loc5_:int = 4 - param2.length % 4;
        while(_loc5_--)
        {
            param2 = param2 + "=";
        }
        param2 = param2.replace(_loc3_,"+").replace(_loc4_,"/");
        this.setBinaryData(param1,Base64.decodeToByteArray(param2));
    }

    public function setBinaryData(param1:int, param2:IDataInput) : void
    {
        var _loc3_:int = 0;
        var _loc4_:int = 0;
        if(!this.charactersStats)
        {
            this.charactersStats = new Dictionary();
        }
        if(!this.charactersStats[param1])
        {
            this.charactersStats[param1] = new CharacterMetricsData();
        }
        while(param2.bytesAvailable >= STATS_SIZE)
        {
            _loc3_ = param2.readByte();
            _loc4_ = param2.readInt();
            this.charactersStats[param1].setStat(_loc3_,_loc4_);
        }
        this._lastUpdate = new Date();
    }

    public function get lastUpdate() : Date
    {
        return this._lastUpdate;
    }

    public function getCharacterStat(param1:int, param2:int) : int
    {
        if(!this.charactersStats)
        {
            this.charactersStats = new Dictionary();
        }
        if(!this.charactersStats[param1])
        {
            return 0;
        }
        return this.charactersStats[param1].getStat(param2);
    }

    public function parseCharListData(param1:XML) : void
    {
        var _loc2_:XML = null;
        for each(_loc2_ in param1.Char)
        {
            this.setBinaryStringData(int(_loc2_.@id),_loc2_.PCStats);
        }
    }
}
}

package kabam.rotmg.fame.data {
import flash.utils.Dictionary;
import kabam.rotmg.fame.data.bonus.FameBonus;

public class TotalFame
{


    private var _bonuses:Vector.<FameBonus>;

    private var _baseFame:Number;

    private var _currentFame:Number;

    public function TotalFame(param1:Number)
    {
        this._bonuses = new Vector.<FameBonus>();
        super();
        this._baseFame = param1;
        this._currentFame = param1;
    }

    public function addBonus(param1:FameBonus) : void
    {
        if(param1 != null)
        {
            this._bonuses.push(param1);
            this._currentFame = this._currentFame + param1.fameAdded;
        }
    }

    public function get bonuses() : Dictionary
    {
        var _loc2_:FameBonus = null;
        var _loc1_:Dictionary = new Dictionary();
        for each(_loc2_ in this._bonuses)
        {
            _loc1_[_loc2_.id] = _loc2_;
        }
        return _loc1_;
    }

    public function get baseFame() : int
    {
        return this._baseFame;
    }

    public function get currentFame() : int
    {
        return this._currentFame;
    }
}
}

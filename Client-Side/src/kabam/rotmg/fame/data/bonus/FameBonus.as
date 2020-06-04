package kabam.rotmg.fame.data.bonus {
public class FameBonus
{


    private var _added:int;

    private var _numAdded:int;

    private var _level:int;

    private var _fameAdded:int;

    private var _id:int;

    private var _name:String;

    private var _tooltip:String;

    public function FameBonus(param1:int, param2:int, param3:int, param4:int, param5:String, param6:String)
    {
        super();
        this._added = param2;
        this._numAdded = param3;
        this._level = param4;
        this._id = param1;
        this._name = param5;
        this._tooltip = param6;
    }

    public function get added() : int
    {
        return this._added;
    }

    public function get numAdded() : int
    {
        return this._numAdded;
    }

    public function get level() : int
    {
        return this._level;
    }

    public function get fameAdded() : int
    {
        return this._fameAdded;
    }

    public function set fameAdded(param1:int) : void
    {
        this._fameAdded = param1;
    }

    public function get id() : int
    {
        return this._id;
    }

    public function get name() : String
    {
        return this._name;
    }

    public function get tooltip() : String
    {
        return this._tooltip;
    }
}
}

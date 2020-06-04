package kabam.rotmg.util.date {
import mx.utils.StringUtil;

public class TimeSpan
{

    public static const MILLISECONDS_IN_DAY:Number = 86400000;

    public static const MILLISECONDS_IN_HOUR:Number = 3600000;

    public static const MILLISECONDS_IN_MINUTE:Number = 60000;

    public static const MILLISECONDS_IN_SECOND:Number = 1000;


    private var _totalMilliseconds:Number;

    public function TimeSpan(param1:Number)
    {
        super();
        this._totalMilliseconds = Math.floor(param1);
    }

    public static function distanceOfTimeInWords(param1:Date, param2:Date, param3:Boolean = false) : String
    {
        var _loc4_:TimeSpan = TimeSpan.fromDates(param1,param2);
        if(_loc4_.totalMinutes < 1)
        {
            if(param3)
            {
                if(_loc4_.totalSeconds == 0 || _loc4_.totalSeconds == 1)
                {
                    return "now";
                }
                return StringUtil.substitute("{0} seconds ago",[Math.round(_loc4_.totalSeconds)]);
            }
            return "minute ago";
        }
        if(_loc4_.totalMinutes >= 1 && _loc4_.totalMinutes < 2)
        {
            return "1 minute ago";
        }
        if(_loc4_.totalMinutes < 60)
        {
            return StringUtil.substitute("{0} minutes ago",[Math.round(_loc4_.totalMinutes)]);
        }
        if(_loc4_.totalHours >= 1 && _loc4_.totalHours < 2)
        {
            return "1 hour ago";
        }
        return StringUtil.substitute("{0} hours ago",[Math.round(_loc4_.totalHours)]);
    }

    public static function fromDates(param1:Date, param2:Date) : TimeSpan
    {
        return new TimeSpan(param2.time - param1.time);
    }

    public static function fromMilliseconds(param1:Number) : TimeSpan
    {
        return new TimeSpan(param1);
    }

    public static function fromSeconds(param1:Number) : TimeSpan
    {
        return new TimeSpan(param1 * MILLISECONDS_IN_SECOND);
    }

    public static function fromMinutes(param1:Number) : TimeSpan
    {
        return new TimeSpan(param1 * MILLISECONDS_IN_MINUTE);
    }

    public static function fromHours(param1:Number) : TimeSpan
    {
        return new TimeSpan(param1 * MILLISECONDS_IN_HOUR);
    }

    public static function fromDays(param1:Number) : TimeSpan
    {
        return new TimeSpan(param1 * MILLISECONDS_IN_DAY);
    }

    public function get days() : int
    {
        return int(this._totalMilliseconds / MILLISECONDS_IN_DAY);
    }

    public function get hours() : int
    {
        return int(this._totalMilliseconds / MILLISECONDS_IN_HOUR) % 24;
    }

    public function get minutes() : int
    {
        return int(this._totalMilliseconds / MILLISECONDS_IN_MINUTE) % 60;
    }

    public function get seconds() : int
    {
        return int(this._totalMilliseconds / MILLISECONDS_IN_SECOND) % 60;
    }

    public function get milliseconds() : int
    {
        return int(this._totalMilliseconds) % 1000;
    }

    public function get totalDays() : Number
    {
        return this._totalMilliseconds / MILLISECONDS_IN_DAY;
    }

    public function get totalHours() : Number
    {
        return this._totalMilliseconds / MILLISECONDS_IN_HOUR;
    }

    public function get totalMinutes() : Number
    {
        return this._totalMilliseconds / MILLISECONDS_IN_MINUTE;
    }

    public function get totalSeconds() : Number
    {
        return this._totalMilliseconds / MILLISECONDS_IN_SECOND;
    }

    public function get totalMilliseconds() : Number
    {
        return this._totalMilliseconds;
    }

    public function add(param1:Date) : Date
    {
        var _loc2_:Date = new Date(param1.time);
        _loc2_.milliseconds = _loc2_.milliseconds + this.totalMilliseconds;
        return _loc2_;
    }
}
}

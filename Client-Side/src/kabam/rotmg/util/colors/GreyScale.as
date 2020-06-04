package kabam.rotmg.util.colors {
import flash.display.BitmapData;
import flash.display.DisplayObject;
import flash.filters.ColorMatrixFilter;
import flash.geom.Point;
import flash.geom.Rectangle;

public class GreyScale
{


    public function GreyScale()
    {
        super();
    }

    public static function setGreyScale(param1:BitmapData) : BitmapData
    {
        var _loc5_:Array = [0.2225,0.7169,0.0606,0,0,0.2225,0.7169,0.0606,0,0,0.2225,0.7169,0.0606,0,0,0,0,0,1,0];
        var _loc6_:ColorMatrixFilter = new ColorMatrixFilter(_loc5_);
        param1.applyFilter(param1,new Rectangle(0,0,param1.width,param1.height),new Point(0,0),_loc6_);
        return param1;
    }

    public static function greyScaleToDisplayObject(param1:DisplayObject, param2:Boolean) : void
    {
        var _loc6_:Array = [0.2225,0.7169,0.0606,0,0,0.2225,0.7169,0.0606,0,0,0.2225,0.7169,0.0606,0,0,0,0,0,1,0];
        var _loc7_:ColorMatrixFilter = new ColorMatrixFilter(_loc6_);
        if(param2)
        {
            param1.filters = [_loc7_];
        }
        else
        {
            param1.filters = [];
        }
    }
}
}

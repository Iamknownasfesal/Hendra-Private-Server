package kabam.rotmg.fame {
import com.company.assembleegameclient.objects.ObjectLibrary;
import com.company.assembleegameclient.objects.TextureDataConcrete;
import com.company.assembleegameclient.util.TextureRedrawer;
import flash.display.Bitmap;
import flash.display.BitmapData;

public class DungeonLine extends StatsLine
{


    private var dungeonTextureName:String;

    private var dungeonBitmap:Bitmap;

    public function DungeonLine(param1:String, param2:String, param3:String)
    {
        this.dungeonTextureName = param2;
        super(param1,param3,"",StatsLine.TYPE_STAT);
    }

    override protected function setLabelsPosition() : void
    {
        var _loc2_:BitmapData = null;
        var _loc1_:TextureDataConcrete = ObjectLibrary.dungeonToPortalTextureData_[this.dungeonTextureName];
        if(_loc1_)
        {
            _loc2_ = _loc1_.getTexture();
            _loc2_ = TextureRedrawer.redraw(_loc2_,40,true,0,false);
            this.dungeonBitmap = new Bitmap(_loc2_);
            this.dungeonBitmap.x = -Math.round(_loc2_.width / 2) + 13;
            this.dungeonBitmap.y = -Math.round(_loc2_.height / 2) + 11;
            addChild(this.dungeonBitmap);
        }
        label.y = 4;
        label.x = 24;
        lineHeight = 25;
        if(fameValue)
        {
            fameValue.y = 4;
        }
        if(lock)
        {
            lock.y = -6;
        }
    }

    override public function clean() : void
    {
        super.clean();
        if(this.dungeonBitmap)
        {
            this.dungeonBitmap.bitmapData.dispose();
        }
    }
}
}

package kabam.rotmg.game.model
{
import com.company.assembleegameclient.objects.GameObject;
import com.company.assembleegameclient.objects.Player;
import flash.utils.Dictionary;

public class GameModel
{


    public var player:Player;

    public var gameObjects:Dictionary;

    public function GameModel()
    {
        super();
    }

    public function getGameObject(param1:int) : GameObject
    {
        var _loc2_:GameObject = this.gameObjects[param1];
        if(!_loc2_ && this.player.objectId_ == param1)
        {
            _loc2_ = this.player;
        }
        return _loc2_;
    }
}
}

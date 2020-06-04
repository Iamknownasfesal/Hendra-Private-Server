package kabam.rotmg.fame.data {
import com.company.assembleegameclient.appengine.SavedCharacter;
import kabam.rotmg.characterMetrics.data.MetricsID;
import kabam.rotmg.characterMetrics.tracker.CharactersMetricsTracker;
import kabam.rotmg.fame.data.bonus.FameBonus;
import kabam.rotmg.fame.data.bonus.FameBonusConfig;
import kabam.rotmg.fame.data.bonus.FameBonusID;
import kabam.rotmg.core.model.PlayerModel;
import kabam.rotmg.ui.model.HUDModel;

public class FameTracker
{


    [Inject]
    public var metrics:CharactersMetricsTracker;

    [Inject]
    public var hudModel:HUDModel;

    [Inject]
    public var player:PlayerModel;

    public function FameTracker()
    {
        super();
    }

    private function getFameBonus(param1:int, param2:int, param3:int) : FameBonus
    {
        var _loc4_:FameBonus = FameBonusConfig.getBonus(param2);
        var _loc5_:int = this.getCharacterLevel(param1);
        if(_loc5_ < _loc4_.level)
        {
            return null;
        }
        _loc4_.fameAdded = (int)(_loc4_.added / 100.0 * param3) + _loc4_.numAdded;
        return _loc4_;
    }

    private function getWellEquippedBonus(param1:int, param2:int) : FameBonus
    {
        var _loc3_:FameBonus = FameBonusConfig.getBonus(FameBonusID.WELL_EQUIPPED);
        _loc3_.fameAdded = Math.floor(param1 * param2 / 100);
        return _loc3_;
    }

    public function getCurrentTotalFame(param1:int) : TotalFame
    {
        var _loc2_:TotalFame = new TotalFame(this.currentFame(param1));
        var _loc3_:int = this.getCharacterLevel(param1);
        var _loc4_:int = this.getCharacterType(param1);
        if(this.player.getTotalFame() == 0)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.ANCESTOR,_loc2_.currentFame));
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.POTIONS_DRUNK) == 0)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.THIRSTY,_loc2_.currentFame));
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.SHOTS_THAT_DAMAGE) == 0)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.PACIFIST,_loc2_.currentFame));
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.SPECIAL_ABILITY_USES) == 0)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.MUNDANE,_loc2_.currentFame));
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.TELEPORTS) == 0)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.BOOTS_ON_THE_GROUND,_loc2_.currentFame));
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.PIRATE_CAVES_COMPLETED) > 0 && this.metrics.getCharacterStat(param1,MetricsID.UNDEAD_LAIRS_COMPLETED) > 0 && this.metrics.getCharacterStat(param1,MetricsID.ABYSS_OF_DEMONS_COMPLETED) > 0 && this.metrics.getCharacterStat(param1,MetricsID.SNAKE_PITS_COMPLETED) > 0 && this.metrics.getCharacterStat(param1,MetricsID.SPIDER_DENS_COMPLETED) > 0 && this.metrics.getCharacterStat(param1,MetricsID.SPRITE_WORLDS_COMPLETED) > 0 && this.metrics.getCharacterStat(param1,MetricsID.TOMBS_COMPLETED) > 0 && this.metrics.getCharacterStat(param1,MetricsID.TRENCHES_COMPLETED) > 0 && this.metrics.getCharacterStat(param1,MetricsID.JUNGLES_COMPLETED) > 0 && this.metrics.getCharacterStat(param1,MetricsID.MANORS_COMPLETED) > 0)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.TUNNEL_RAT,_loc2_.currentFame));
        }
        var _loc5_:int = this.metrics.getCharacterStat(param1,MetricsID.MONSTER_KILLS);
        var _loc6_:int = this.metrics.getCharacterStat(param1,MetricsID.GOD_KILLS);
        if(_loc5_ + _loc6_ > 0)
        {
            if(_loc6_ / (_loc5_ + _loc6_) > 0.1)
            {
                _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.ENEMY_OF_THE_GODS,_loc2_.currentFame));
            }
            if(_loc6_ / (_loc5_ + _loc6_) > 0.5)
            {
                _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.SLAYER_OF_THE_GODS,_loc2_.currentFame));
            }
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.ORYX_KILLS) > 0)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.ORYX_SLAYER,_loc2_.currentFame));
        }
        var _loc7_:int = this.metrics.getCharacterStat(param1,MetricsID.SHOTS);
        var _loc8_:int = this.metrics.getCharacterStat(param1,MetricsID.SHOTS_THAT_DAMAGE);
        if(_loc8_ > 0 && _loc7_ > 0)
        {
            if(_loc8_ / _loc7_ > 0.25)
            {
                _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.ACCURATE,_loc2_.currentFame));
            }
            if(_loc8_ / _loc7_ > 0.5)
            {
                _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.SHARPSHOOTER,_loc2_.currentFame));
            }
            if(_loc8_ / _loc7_ > 0.75)
            {
                _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.SNIPER,_loc2_.currentFame));
            }
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.TILES_UNCOVERED) > 1000000)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.EXPLORER,_loc2_.currentFame));
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.TILES_UNCOVERED) > 4000000)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.CARTOGRAPHER,_loc2_.currentFame));
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.CUBE_KILLS) == 0)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.FRIEND_OF_THE_CUBES,_loc2_.currentFame));
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.LEVEL_UP_ASSISTS) > 100)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.TEAM_PLAYER,_loc2_.currentFame));
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.LEVEL_UP_ASSISTS) > 1000)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.LEADER_OF_MEN,_loc2_.currentFame));
        }
        if(this.metrics.getCharacterStat(param1,MetricsID.QUESTS_COMPLETED) > 1000)
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.DOER_OF_DEEDS,_loc2_.currentFame));
        }
        _loc2_.addBonus(this.getWellEquippedBonus(this.getCharacterFameBonus(param1),_loc2_.currentFame));
        if(_loc2_.currentFame > this.player.getBestCharFame())
        {
            _loc2_.addBonus(this.getFameBonus(param1,FameBonusID.FIRST_BORN,_loc2_.currentFame));
        }
        return _loc2_;
    }

    private function hasMapPlayer() : Boolean
    {
        return this.hudModel.gameSprite && this.hudModel.gameSprite.map && this.hudModel.gameSprite.map.player_;
    }

    private function getSavedCharacter(param1:int) : SavedCharacter
    {
        return this.player.getCharacterById(param1);
    }

    private function getCharacterExp(param1:int) : int
    {
        if(this.hasMapPlayer())
        {
            return this.hudModel.gameSprite.map.player_.exp_;
        }
        return this.getSavedCharacter(param1).xp();
    }

    private function getCharacterLevel(param1:int) : int
    {
        if(this.hasMapPlayer())
        {
            return this.hudModel.gameSprite.map.player_.level_;
        }
        return this.getSavedCharacter(param1).level();
    }

    private function getCharacterType(param1:int) : int
    {
        if(this.hasMapPlayer())
        {
            return this.hudModel.gameSprite.map.player_.objectType_;
        }
        return this.getSavedCharacter(param1).objectType();
    }

    private function getCharacterFameBonus(param1:int) : int
    {
        if(this.hasMapPlayer())
        {
            return this.hudModel.gameSprite.map.player_.getFameBonus();
        }
        return this.getSavedCharacter(param1).fameBonus();
    }

    public function currentFame(param1:int) : int
    {
        var _loc2_:int = this.metrics.getCharacterStat(param1,MetricsID.MINUTES_ACTIVE);
        var _loc3_:int = this.getCharacterExp(param1);
        var _loc4_:int = this.getCharacterLevel(param1);
        if(this.hasMapPlayer())
        {
            _loc3_ = _loc3_ + (_loc4_ - 1) * (_loc4_ - 1) * 50;
        }
        return this.calculateBaseFame(_loc3_,_loc2_);
    }

    public function calculateBaseFame(param1:int, param2:int) : int
    {
        var _loc3_:Number = 0;
        _loc3_ = _loc3_ + Math.max(0,Math.min(20000,param1)) * 0.001;
        _loc3_ = _loc3_ + Math.max(0,Math.min(45200,param1) - 20000) * 0.002;
        _loc3_ = _loc3_ + Math.max(0,Math.min(80000,param1) - 45200) * 0.003;
        _loc3_ = _loc3_ + Math.max(0,Math.min(101200,param1) - 80000) * 0.002;
        _loc3_ = _loc3_ + Math.max(0,param1 - 101200) * 0.0005;
        _loc3_ = _loc3_ + Math.min(Math.floor(param2 / 6),30);
        return Math.floor(_loc3_);
    }
}
}

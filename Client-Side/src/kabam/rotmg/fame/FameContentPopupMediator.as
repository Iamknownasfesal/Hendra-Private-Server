package kabam.rotmg.fame {
import com.company.assembleegameclient.appengine.SavedCharacter;
import com.company.assembleegameclient.objects.ObjectLibrary;
import com.company.assembleegameclient.objects.Player;
import com.company.assembleegameclient.parameters.Parameters;
import com.company.assembleegameclient.ui.tooltip.TextToolTip;

import flash.globalization.DateTimeFormatter;
import flash.utils.Dictionary;
import kabam.rotmg.characterMetrics.data.MetricsID;
import kabam.rotmg.characterMetrics.tracker.CharactersMetricsTracker;
import kabam.rotmg.dialogs.control.CloseDialogsSignal;
import kabam.rotmg.fame.data.FameTracker;
import kabam.rotmg.fame.data.TotalFame;
import kabam.rotmg.fame.data.bonus.FameBonus;
import kabam.rotmg.fame.data.bonus.FameBonusConfig;
import kabam.rotmg.ui.buttons.BaseButton;
import kabam.rotmg.ui.buttons.SliceScalingButton;
import kabam.rotmg.ui.popups.header.PopupHeader;
import kabam.rotmg.ui.texture.TextureParser;
import kabam.rotmg.util.date.TimeSpan;
import kabam.rotmg.core.model.PlayerModel;
import kabam.rotmg.core.signals.HideTooltipsSignal;
import kabam.rotmg.core.signals.ShowTooltipSignal;
import kabam.rotmg.text.view.stringBuilder.LineBuilder;
import kabam.rotmg.text.view.stringBuilder.StaticStringBuilder;
import kabam.rotmg.text.view.stringBuilder.StringBuilder;
import kabam.rotmg.tooltips.HoverTooltipDelegate;
import kabam.rotmg.ui.model.HUDModel;
import robotlegs.bender.bundles.mvcs.Mediator;

public class FameContentPopupMediator extends Mediator
{


    [Inject]
    public var view:FameContentPopup;

    [Inject]
    public var closePopupSignal:CloseDialogsSignal;

    private var closeButton:SliceScalingButton;

    [Inject]
    public var showTooltipSignal:ShowTooltipSignal;

    [Inject]
    public var hideTooltipSignal:HideTooltipsSignal;

    [Inject]
    public var fameTracker:FameTracker;

    [Inject]
    public var player:PlayerModel;

    [Inject]
    public var metrics:CharactersMetricsTracker;

    [Inject]
    public var hudModel:HUDModel;

    private var toolTip:TextToolTip = null;

    private var hoverTooltipDelegate:HoverTooltipDelegate;

    private var totalFame:TotalFame;

    private var bonuses:Dictionary;

    private var bonusesList:Vector.<FameBonus>;

    private var characterID:int;

    public function FameContentPopupMediator()
    {
        super();
    }

    override public function initialize() : void
    {
        var _loc2_:DateTimeFormatter = null;
        var _loc3_:Player = null;
        var _loc4_:SavedCharacter = null;
        this.closeButton = new SliceScalingButton(TextureParser.instance.getSliceScalingBitmap("UI","close_button"));
        this.closeButton.clickSignal.addOnce(this.onClose);
        this.view.header.addButton(this.closeButton,PopupHeader.RIGHT_BUTTON);
        this.characterID = this.view.characterId == -1?int(this.hudModel.gameSprite.gsc_.charId_):int(this.view.characterId);
        this.totalFame = this.fameTracker.getCurrentTotalFame(this.characterID);
        this.bonuses = this.totalFame.bonuses;
        this.bonusesList = new Vector.<FameBonus>();
        var _loc1_:String = "";
        if(!this.player.getCharacterById(this.characterID))
        {
            _loc2_ = new DateTimeFormatter("en-US");
            _loc2_.setDateTimePattern("MMMM dd yyyy");
            _loc1_ = _loc2_.format(new Date());
        }
        else
        {
            _loc1_ = this.player.getCharacterById(this.characterID).bornOn();
        }
        this.showInfo();
        this.view.fameOnDeath = this.totalFame.currentFame;
        if(this.view.characterId == -1)
        {
            _loc3_ = this.hudModel.gameSprite.map.player_;
            this.view.setCharacterData(this.totalFame.baseFame,_loc3_.name_,_loc3_.level_,ObjectLibrary.typeToDisplayId_[_loc3_.objectType_],_loc1_,_loc3_.getFamePortrait(200));
        }
        else
        {
            _loc4_ = this.player.getCharacterById(this.characterID);
            this.view.setCharacterData(this.totalFame.baseFame,_loc4_.name(),_loc4_.level(),ObjectLibrary.typeToDisplayId_[_loc4_.objectType()],_loc1_,_loc4_.getIcon(100));
        }
        this.toolTip = new TextToolTip(3552822,10197915,"Fame calculation","Refreshes when returning to the main menu.",230);
        this.hoverTooltipDelegate = new HoverTooltipDelegate();
        this.hoverTooltipDelegate.setShowToolTipSignal(this.showTooltipSignal);
        this.hoverTooltipDelegate.setHideToolTipsSignal(this.hideTooltipSignal);
        this.hoverTooltipDelegate.setDisplayObject(this.view.infoButton);
        this.hoverTooltipDelegate.tooltip = this.toolTip;
        this.showTooltipSignal.add(this.onShowTooltip);
        Parameters.data_["clicked_on_fame_ui"] = true;
    }

    private function onShowTooltip(param1:TextToolTip) : void
    {
        var _loc2_:StringBuilder = param1.titleText_.getStringBuilder();
        if(this.fameTracker.metrics.lastUpdate && _loc2_ is LineBuilder && LineBuilder(_loc2_).key == "Fame calculation")
        {
            param1.setTitle(new StaticStringBuilder("Updated " + TimeSpan.distanceOfTimeInWords(this.fameTracker.metrics.lastUpdate,new Date(),true) + "."));
        }
    }

    override public function destroy() : void
    {
        this.closeButton.dispose();
        this.hoverTooltipDelegate = null;
        this.toolTip = null;
        this.showTooltipSignal.remove(this.onShowTooltip);
    }

    private function onClose(param1:BaseButton) : void
    {
        this.closePopupSignal.dispatch();
    }

    private function getBonusValue(param1:int) : int
    {
        if(!this.totalFame.bonuses[param1])
        {
            return 0;
        }
        return this.totalFame.bonuses[param1].fameAdded;
    }

    private function showCompletedDungeons() : void
    {
        var _loc2_:StatsLine = null;
        var _loc1_:Vector.<StatsLine> = new Vector.<StatsLine>();
        this.view.addDungeonLine(new StatsLine("Dungeons","","",StatsLine.TYPE_TITLE));
        _loc1_.push(new DungeonLine("Pirate Caves","Pirate Cave",this.metrics.getCharacterStat(this.characterID,MetricsID.PIRATE_CAVES_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Undead Lairs","Undead Lair",this.metrics.getCharacterStat(this.characterID,MetricsID.UNDEAD_LAIRS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Abysses of Demons","Abyss of Demons",this.metrics.getCharacterStat(this.characterID,MetricsID.ABYSS_OF_DEMONS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Snake Pits","Snake Pit",this.metrics.getCharacterStat(this.characterID,MetricsID.SNAKE_PITS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Spider Den","Spider Den",this.metrics.getCharacterStat(this.characterID,MetricsID.SPIDER_DENS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Sprite World","Sprite World",this.metrics.getCharacterStat(this.characterID,MetricsID.SPRITE_WORLDS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Tomb of the Ancients","Tomb of the Ancients",this.metrics.getCharacterStat(this.characterID,MetricsID.TOMBS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Ocean Trench","Ocean Trench",this.metrics.getCharacterStat(this.characterID,MetricsID.TRENCHES_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Forbidden Jungle","Forbidden Jungle",this.metrics.getCharacterStat(this.characterID,MetricsID.JUNGLES_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Manor of the Immortals","Manor of the Immortals",this.metrics.getCharacterStat(this.characterID,MetricsID.MANORS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Forest Maze","Forest Maze",this.metrics.getCharacterStat(this.characterID,MetricsID.FOREST_MAZE_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Lair of Draconis","Lair of Draconis",this.metrics.getCharacterStat(this.characterID,MetricsID.LAIR_OF_DRACONIS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Candyland Hunting Grounds","Candyland Hunting Grounds",this.metrics.getCharacterStat(this.characterID,MetricsID.CANDY_LAND_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Haunted Cemetery","Haunted Cemetery",this.metrics.getCharacterStat(this.characterID,MetricsID.HAUNTED_CEMETERY_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Cave of a Thousand Treasures","Cave of A Thousand Treasures",this.metrics.getCharacterStat(this.characterID,MetricsID.CAVE_OF_A_THOUSAND_TREASURES_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Mad Lab","Mad Lab",this.metrics.getCharacterStat(this.characterID,MetricsID.MAD_LAB_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Davy Jones\' Locker","Davy Jones\' Locker",this.metrics.getCharacterStat(this.characterID,MetricsID.DAVY_JONES_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Ice Cave","Ice Cave",this.metrics.getCharacterStat(this.characterID,MetricsID.ICE_CAVE_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Deadwater Docks","Deadwater Docks",this.metrics.getCharacterStat(this.characterID,MetricsID.DEADWATER_DOCKS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("The Crawling Depths","The Crawling Depths",this.metrics.getCharacterStat(this.characterID,MetricsID.CRAWLING_DEPTH_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Woodland Labyrinth","Woodland Labyrinth",this.metrics.getCharacterStat(this.characterID,MetricsID.WOODLAND_LAB_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Battle for the Nexus","Battle for the Nexus",this.metrics.getCharacterStat(this.characterID,MetricsID.BATTLE_NEXUS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("The Shatters","The Shatters",this.metrics.getCharacterStat(this.characterID,MetricsID.THE_SHATTERS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Belladonna’s Garden","Belladonna\'s Garden",this.metrics.getCharacterStat(this.characterID,MetricsID.BELLADONNA_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("The Puppet Master’s Theatre","Puppet Master\'s Theatre",this.metrics.getCharacterStat(this.characterID,MetricsID.PUPPET_MASTER_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Toxic Sewers","Toxic Sewers",this.metrics.getCharacterStat(this.characterID,MetricsID.TOXIC_SEWERS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("The Hive","The Hive",this.metrics.getCharacterStat(this.characterID,MetricsID.THE_HIVE_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Mountain Temple","Mountain Temple",this.metrics.getCharacterStat(this.characterID,MetricsID.MOUNTAIN_TEMPLE_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Lair of Draconis (HM)","Lair of Draconis",this.metrics.getCharacterStat(this.characterID,MetricsID.LAIR_OF_DRACONIS_HM_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Lost Halls","Lost Halls",this.metrics.getCharacterStat(this.characterID,MetricsID.LOST_HALLS_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("The Void","The Void",this.metrics.getCharacterStat(this.characterID,MetricsID.THE_VOID_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Puppet Master’s Encore","Puppet Master\'s Encore",this.metrics.getCharacterStat(this.characterID,MetricsID.PUPPET_ENCORE_COMPLETED) + ""));
        _loc1_.push(new DungeonLine("Lair of Shaitan","Lair of Shaitan",this.metrics.getCharacterStat(this.characterID,MetricsID.LAIR_OF_SHAITAN_COMPLETED) + ""));
        _loc1_ = _loc1_.sort(this.dungeonNameSort);
        for each(_loc2_ in _loc1_)
        {
            this.view.addDungeonLine(_loc2_);
        }
    }

    private function dungeonNameSort(param1:StatsLine, param2:StatsLine) : int
    {
        if(param1.labelText > param2.labelText)
        {
            return 1;
        }
        return -1;
    }

    private function showStats() : void
    {
        var _loc1_:Number = 0;
        if(this.metrics.getCharacterStat(this.characterID,MetricsID.SHOTS_THAT_DAMAGE) > 0 && this.metrics.getCharacterStat(this.characterID,MetricsID.SHOTS) > 0)
        {
            _loc1_ = this.metrics.getCharacterStat(this.characterID,MetricsID.SHOTS_THAT_DAMAGE) / this.metrics.getCharacterStat(this.characterID,MetricsID.SHOTS) * 100;
        }
        this.view.addStatLine(new StatsLine("Statistics","","",StatsLine.TYPE_TITLE));
        this.view.addStatLine(new StatsLine("Shots Fired",this.metrics.getCharacterStat(this.characterID,MetricsID.SHOTS).toString(),"The total number of shots fired by this character.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Shots Hit",this.metrics.getCharacterStat(this.characterID,MetricsID.SHOTS_THAT_DAMAGE).toString(),"The total number of enemy hitting shots fired by this character.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Potions Drunk",this.metrics.getCharacterStat(this.characterID,MetricsID.POTIONS_DRUNK).toString(),"The number of potions this character has consumed.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Abilities Used",this.metrics.getCharacterStat(this.characterID,MetricsID.SPECIAL_ABILITY_USES).toString(),"The number of times this character used their abilities.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Teleported",this.metrics.getCharacterStat(this.characterID,MetricsID.TELEPORTS).toString(),"The number of times this character has teleported.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Monster Kills",this.metrics.getCharacterStat(this.characterID,MetricsID.MONSTER_KILLS).toString(),"Total number of monsters killed by this character.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("God Kills",this.metrics.getCharacterStat(this.characterID,MetricsID.GOD_KILLS).toString(),"Total number of Gods killed by this character.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Oryx Kills",this.metrics.getCharacterStat(this.characterID,MetricsID.ORYX_KILLS).toString(),"Total number of Oryx kills for this character.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Monster Assists",this.metrics.getCharacterStat(this.characterID,MetricsID.MONSTER_ASSISTS).toString(),"Total number of monster kills assisted by this character.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("God Assists",this.metrics.getCharacterStat(this.characterID,MetricsID.GOD_ASSISTS).toString(),"Total number of God kills assisted by this character.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Party Level Ups",this.metrics.getCharacterStat(this.characterID,MetricsID.LEVEL_UP_ASSISTS).toString(),"Total number of level ups assisted by this character.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Quests Completed",this.metrics.getCharacterStat(this.characterID,MetricsID.QUESTS_COMPLETED).toString(),"Total number of quests completed by this character.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Cube Kills",this.metrics.getCharacterStat(this.characterID,MetricsID.CUBE_KILLS).toString(),"Total number of Cube Enemies killed by this character.",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Accuracy",_loc1_.toFixed(2) + "%","",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Tiles Seen",this.metrics.getCharacterStat(this.characterID,MetricsID.TILES_UNCOVERED).toString(),"",StatsLine.TYPE_STAT));
        this.view.addStatLine(new StatsLine("Minutes Active",this.metrics.getCharacterStat(this.characterID,MetricsID.MINUTES_ACTIVE).toString(),"Time spent actively defeating Oryx\'s minions.",StatsLine.TYPE_STAT));
    }

    private function sortBonusesByLevel(param1:FameBonus, param2:FameBonus) : int
    {
        if(param1.level > param2.level)
        {
            return 1;
        }
        return -1;
    }

    private function sortBonusesByFame(param1:FameBonus, param2:FameBonus) : int
    {
        if(param1.fameAdded > param2.fameAdded)
        {
            return -1;
        }
        return 1;
    }

    private function showBonuses() : void
    {
        var bonusConfig:FameBonus = null;
        var level:int = 0;
        var bonus:FameBonus = null;
        var i:int = 1;
        while(i <= 14)
        {
            bonusConfig = this.totalFame.bonuses[i];
            if(bonusConfig == null)
            {
                this.bonusesList.push(FameBonusConfig.getBonus(i));
            }
            else
            {
                this.bonusesList.push(bonusConfig);
            }
            i++;
        }
        bonusConfig = this.totalFame.bonuses[16];
        if(bonusConfig == null)
        {
            this.bonusesList.push(FameBonusConfig.getBonus(16));
        }
        else
        {
            this.bonusesList.push(bonusConfig);
        }
        i = 18;
        while(i <= 22)
        {
            bonusConfig = this.totalFame.bonuses[i];
            if(bonusConfig == null)
            {
                this.bonusesList.push(FameBonusConfig.getBonus(i));
            }
            else
            {
                this.bonusesList.push(bonusConfig);
            }
            i++;
        }
        if(this.view.characterId == -1)
        {
            level = this.hudModel.gameSprite.map.player_.level_;
        }
        else
        {
            level = this.player.getCharacterById(this.characterID).level();
        }
        this.bonusesList = this.bonusesList.sort(this.sortBonusesByLevel);
        var unlocked:Vector.<FameBonus> = this.bonusesList.filter(function(param1:FameBonus, param2:int, param3:Vector.<FameBonus>):Boolean
        {
            return level >= param1.level;
        });
        unlocked = unlocked.sort(this.sortBonusesByFame);
        var locked:Vector.<FameBonus> = this.bonusesList.filter(function(param1:FameBonus, param2:int, param3:Vector.<FameBonus>):Boolean
        {
            return level < param1.level;
        });
        this.bonusesList = unlocked.concat(locked);
        this.view.addStatLine(new StatsLine("Bonuses","","",StatsLine.TYPE_TITLE));
        for each(bonus in this.bonusesList)
        {
            this.view.addStatLine(new StatsLine(LineBuilder.getLocalizedStringFromKey("FameBonus." + bonus.name),bonus.fameAdded.toString(),LineBuilder.getLocalizedStringFromKey("FameBonus." + bonus.name + "Description") + "\n" + LineBuilder.getLocalizedStringFromKey("FameBonus.LevelRequirement",{"level":bonus.level}),StatsLine.TYPE_BONUS,level < bonus.level));
        }
    }

    private function showInfo() : void
    {
        this.showStats();
        this.showBonuses();
        this.showCompletedDungeons();
    }
}
}

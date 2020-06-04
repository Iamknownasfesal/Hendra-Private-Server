package com.company.assembleegameclient.game
{
import com.company.assembleegameclient.game.events.MoneyChangedEvent;
import com.company.assembleegameclient.map.Map;
import com.company.assembleegameclient.objects.GameObject;
import com.company.assembleegameclient.objects.IInteractiveObject;
import com.company.assembleegameclient.objects.Player;
import com.company.assembleegameclient.objects.Projectile;
import com.company.assembleegameclient.parameters.Parameters;
import com.company.assembleegameclient.tutorial.Tutorial;
import com.company.assembleegameclient.ui.GuildText;
import com.company.assembleegameclient.ui.RankText;
import com.company.assembleegameclient.ui.menu.PlayerMenu;
import com.company.assembleegameclient.util.TextureRedrawer;
import com.company.util.CachingColorTransformer;
import com.company.util.GraphicsUtil;
import com.company.util.MoreColorUtil;
import com.company.util.MoreObjectUtil;
import com.company.util.PointUtil;
import flash.display.DisplayObject;
import flash.display.GraphicsPath;
import flash.display.GraphicsSolidFill;
import flash.display.IGraphicsData;
import flash.display.Sprite;
import flash.display.StageScaleMode;
import flash.events.Event;
import flash.events.MouseEvent;
import flash.external.ExternalInterface;
import flash.filters.ColorMatrixFilter;
import flash.utils.ByteArray;
import flash.utils.getTimer;
import kabam.lib.loopedprocs.LoopedCallback;
import kabam.lib.loopedprocs.LoopedProcess;
import kabam.rotmg.account.core.Account;
import kabam.rotmg.appengine.api.AppEngineClient;
import kabam.rotmg.chat.view.Chat;
import kabam.rotmg.constants.GeneralConstants;
import kabam.rotmg.core.StaticInjectorContext;
import kabam.rotmg.core.model.MapModel;
import kabam.rotmg.core.model.PlayerModel;
import kabam.rotmg.core.view.Layers;
import kabam.rotmg.dialogs.control.OpenDialogSignal;
import kabam.rotmg.game.view.CreditDisplay;
import kabam.rotmg.game.view.GiftStatusDisplay;
import kabam.rotmg.game.view.components.TabConstants;
import kabam.rotmg.maploading.signals.HideMapLoadingSignal;
import kabam.rotmg.maploading.signals.MapLoadedSignal;
import kabam.rotmg.messaging.impl.GameServerConnectionConcrete;
import kabam.rotmg.messaging.impl.incoming.MapInfo;
import kabam.rotmg.servers.api.Server;
import kabam.rotmg.stage3D.Renderer;
import kabam.rotmg.ui.UIUtils;
import kabam.rotmg.ui.view.CharacterDetailsView;
import kabam.rotmg.ui.view.HUDView;
import kabam.rotmg.ui.view.StatMetersView;

import org.osflash.signals.Signal;

public class GameSprite extends AGameSprite
{

    protected static const PAUSED_FILTER:ColorMatrixFilter = new ColorMatrixFilter(MoreColorUtil.greyscaleFilterMatrix);


    public const monitor:Signal = new Signal(String,int);

    public const modelInitialized:Signal = new Signal();

    public const drawCharacterWindow:Signal = new Signal(Player);

    public var chatBox_:Chat;

    public var isNexus_:Boolean = false;

    public var idleWatcher_:IdleWatcher;

    public var rankText_:RankText;

    public var guildText_:GuildText;

    public var creditDisplay_:CreditDisplay;

    public var giftStatusDisplay:GiftStatusDisplay;

    public var mapModel:MapModel;

    public var openDialog:OpenDialogSignal;

    public var showPackage:Signal;

    public var chatPlayerMenu:PlayerMenu;

    private var focus:GameObject;

    private var frameTimeSum_:int = 0;

    private var frameTimeCount_:int = 0;

    private var isGameStarted:Boolean;

    private var statMeters:StatMetersView;

    private var displaysPosY:uint = 4;

    private var characterDetails:CharacterDetailsView;

    private var currentPackage:DisplayObject;

    private var packageY:Number;

    private const bgY:Sprite = new Sprite();

    public function GameSprite(param1:Server, param2:int, param3:Boolean, param4:int, param5:int, param6:ByteArray, param7:PlayerModel, param8:String, param9:Boolean)
    {
        this.showPackage = new Signal();
        this.currentPackage = new Sprite();
        super();
        this.model = param7;
        map = new Map(this);
        addChild(map);
        gsc_ = new GameServerConnectionConcrete(this,param1,param2,param3,param4,param5,param6,param8,param9);
        mui_ = new MapUserInput(this);
        this.chatBox_ = new Chat();
        this.chatBox_.list.addEventListener(MouseEvent.MOUSE_DOWN,this.onChatDown);
        this.chatBox_.list.addEventListener(MouseEvent.MOUSE_UP,this.onChatUp);
        addChild(this.chatBox_);
        this.idleWatcher_ = new IdleWatcher();
    }

    public static function dispatchMapLoaded(param1:MapInfo) : void
    {
        var _loc2_:MapLoadedSignal = StaticInjectorContext.getInjector().getInstance(MapLoadedSignal);
        _loc2_ && _loc2_.dispatch(param1);
    }

    private static function hidePreloader() : void
    {
        var _loc1_:HideMapLoadingSignal = StaticInjectorContext.getInjector().getInstance(HideMapLoadingSignal);
        _loc1_ && _loc1_.dispatch();
    }

    override public function setFocus(param1:GameObject) : void
    {
        param1 = param1 || map.player_;
        this.focus = param1;
    }

    override public function applyMapInfo(param1:MapInfo) : void
    {
        map.setProps(param1.width_,param1.height_,param1.name_,param1.background_,param1.allowPlayerTeleport_,param1.showDisplays_);
        dispatchMapLoaded(param1);
    }

    override public function initialize() : void
    {
        var _loc1_:Account = null;
        _loc1_ = null;
        map.initialize();
        this.modelInitialized.dispatch();
        if(this.evalIsNotInCombatMapArea())
        {
            this.showSafeAreaDisplays();
        }
        _loc1_ = StaticInjectorContext.getInjector().getInstance(Account);
        this.isNexus_ = map.name_ == Map.NEXUS;
        if(this.isNexus_ || map.name_ == Map.DAILY_QUEST_ROOM)
        {
            this.creditDisplay_ = new CreditDisplay(this,true);
        }
        else
        {
            this.creditDisplay_ = new CreditDisplay(this);
        }
        this.creditDisplay_.x = 594;
        this.creditDisplay_.y = 0;
        addChild(this.creditDisplay_);
        var _loc3_:AppEngineClient = StaticInjectorContext.getInjector().getInstance(AppEngineClient);
        var _loc4_:Object = {
            "game_net_user_id":_loc1_.gameNetworkUserId(),
            "game_net":_loc1_.gameNetwork(),
            "play_platform":_loc1_.playPlatform()
        };
        MoreObjectUtil.addToObject(_loc4_,_loc1_.getCredentials());
        if(map.name_ != "Kitchen" && map.name_ != "Tutorial" && map.name_ != "Nexus Explanation" && Parameters.data_.watchForTutorialExit == true)
        {
            Parameters.data_.watchForTutorialExit = false;
            this.callTracking("rotmg.Marketing.track(\"tutorialComplete\")");
            _loc4_["fteStepCompleted"] = 9900;
            _loc3_.sendRequest("/log/logFteStep",_loc4_);
        }
        if(map.name_ == "Kitchen")
        {
            _loc4_["fteStepCompleted"] = 200;
            _loc3_.sendRequest("/log/logFteStep",_loc4_);
        }
        if(map.name_ == "Tutorial")
        {
            if(Parameters.data_.needsTutorial == true)
            {
                Parameters.data_.watchForTutorialExit = true;
                this.callTracking("rotmg.Marketing.track(\"install\")");
                _loc4_["fteStepCompleted"] = 100;
                _loc3_.sendRequest("/log/logFteStep",_loc4_);
            }
            this.startTutorial();
        }
        Parameters.save();
        hidePreloader();
        stage.dispatchEvent(new Event(Event.RESIZE));
        this.parent.parent.setChildIndex((this.parent.parent as Layers).top,2);
    }

    override public function evalIsNotInCombatMapArea() : Boolean
    {
        return map.name_ == Map.NEXUS || map.name_ == Map.VAULT || map.name_ == Map.GUILD_HALL || map.name_ == Map.CLOTH_BAZAAR || map.name_ == Map.NEXUS_EXPLANATION || map.name_ == Map.DAILY_QUEST_ROOM;
    }

    public function chatMenuPositionFixed() : void
    {
        var _loc2_:Number = NaN;
        var _loc1_:Number = (stage.mouseX + stage.stageWidth / 2 - 400) / stage.stageWidth * 800;
        _loc2_ = (stage.mouseY + stage.stageHeight / 2 - 300) / stage.stageHeight * 600;
        this.chatPlayerMenu.x = _loc1_;
        this.chatPlayerMenu.y = _loc2_ - this.chatPlayerMenu.height;
    }

    public function addChatPlayerMenu(param1:Player, param2:Number, param3:Number, param4:String = null, param5:Boolean = false, param6:Boolean = false) : void
    {
        this.removeChatPlayerMenu();
        this.chatPlayerMenu = new PlayerMenu();
        if(param4 == null)
        {
            this.chatPlayerMenu.init(this,param1);
        }
        else if(param6)
        {
            this.chatPlayerMenu.initDifferentServer(this,param4,param5,param6);
        }
        else
        {
            if(param4.length > 0 && (param4.charAt(0) == "#" || param4.charAt(0) == "*" || param4.charAt(0) == "@"))
            {
                return;
            }
            this.chatPlayerMenu.initDifferentServer(this,param4,param5);
        }
        addChild(this.chatPlayerMenu);
        this.chatMenuPositionFixed();
    }

    public function removeChatPlayerMenu() : void
    {
        if(this.chatPlayerMenu != null && this.chatPlayerMenu.parent != null)
        {
            removeChild(this.chatPlayerMenu);
            this.chatPlayerMenu = null;
        }
    }

    public function hudModelInitialized() : void
    {
        hudView = new HUDView();
        hudView.x = 600;
        addChild(hudView);
    }

    public function connect() : void
    {
        if(!this.isGameStarted)
        {
            this.isGameStarted = true;
            Renderer.inGame = true;
            gsc_.connect();
            this.idleWatcher_.start(this);
            lastUpdate_ = getTimer();
            stage.addEventListener(MoneyChangedEvent.MONEY_CHANGED,this.onMoneyChanged);
            stage.addEventListener(Event.ENTER_FRAME,this.onEnterFrame);
            this.parent.parent.setChildIndex((this.parent.parent as Layers).top,0);
            if(Parameters.data_.mscale == undefined)
            {
                Parameters.data_.mscale = "1.0";
                Parameters.save();
            }
            if(Parameters.data_.stageScale == undefined)
            {
                Parameters.data_.stageScale = StageScaleMode.NO_SCALE;
                Parameters.save();
            }
            if(Parameters.data_.uiscale == undefined)
            {
                Parameters.data_.uiscale = true;
                Parameters.save();
            }
            stage.scaleMode = Parameters.data_.stageScale;
            stage.addEventListener(Event.RESIZE,this.onScreenResize);
            stage.dispatchEvent(new Event(Event.RESIZE));
            LoopedProcess.addProcess(new LoopedCallback(100,this.updateNearestInteractive));
        }
    }

    public function disconnect() : void
    {
        if(this.isGameStarted)
        {
            this.isGameStarted = false;
            Renderer.inGame = false;
            this.idleWatcher_.stop();
            stage.removeEventListener(MoneyChangedEvent.MONEY_CHANGED,this.onMoneyChanged);
            stage.removeEventListener(Event.ENTER_FRAME,this.onEnterFrame);
            stage.removeEventListener(Event.RESIZE,this.onScreenResize);
            stage.scaleMode = StageScaleMode.EXACT_FIT;
            stage.dispatchEvent(new Event(Event.RESIZE));
            LoopedProcess.destroyAll();
            contains(map) && removeChild(map);
            map.dispose();
            CachingColorTransformer.clear();
            TextureRedrawer.clearCache();
            Projectile.dispose();
            gsc_.disconnect();
        }
    }

    private function showSafeAreaDisplays() : void
    {
        this.showUI();
        this.showGuildText();
        this.setYAndPositionPackage();
        this.showGiftStatusDisplay();
    }

    private function showGiftStatusDisplay() : void
    {
        this.giftStatusDisplay = new GiftStatusDisplay();
        this.giftStatusDisplay.x = 6;
        this.giftStatusDisplay.y = this.displaysPosY + 2;
        this.displaysPosY = this.displaysPosY + UIUtils.NOTIFICATION_SPACE;
        addChild(this.giftStatusDisplay);
    }


    private function setYAndPositionPackage() : void
    {
        this.packageY = this.displaysPosY + 2;
        this.displaysPosY = this.displaysPosY + UIUtils.NOTIFICATION_SPACE;
        this.positionPackage();
    }

    private function positionPackage() : void
    {
        this.currentPackage.x = 6;
        this.currentPackage.y = this.packageY;
    }

    private function addAndPositionPackage(param1:DisplayObject) : void
    {
        this.currentPackage = param1;
        addChild(this.currentPackage);
        this.positionPackage();
    }

    private function showGuildText() : void
    {
        this.guildText_ = new GuildText("",-1);
        this.guildText_.x = 64;
        this.guildText_.y = 6;
        addChild(this.guildText_);
    }

    private function showUI() : void
    {
        var _local1:GraphicsSolidFill = new GraphicsSolidFill(TabConstants.BACKGROUND_COLOR, 1);
        var _local2:GraphicsPath = new GraphicsPath(new Vector.<int>(), new Vector.<Number>());
        var _local3:Vector.<IGraphicsData> = new <IGraphicsData>[_local1, _local2, GraphicsUtil.END_FILL];
        GraphicsUtil.drawCutEdgeRect(0, 0, 280, (75), 6, [1, 1, 1, 1], _local2);
        this.bgY.graphics.drawGraphicsData(_local3);
        this.bgY.x = 3;
        this.bgY.y = TabConstants.TAB_TOP_OFFSET;
        addChild(this.bgY);
        this.bgY.y = this.displaysPosY;
        this.displaysPosY = this.displaysPosY + UIUtils.NOTIFICATION_SPACE;
        this.characterDetails = new CharacterDetailsView();
        this.characterDetails.x = 0;
        this.characterDetails.y = 4;
        this.statMeters = new StatMetersView();
        addChild(this.bgY);
        addChild(this.characterDetails);
        addChild(this.statMeters);
    }

    private function callTracking(param1:String) : void
    {
        if(ExternalInterface.available == false)
        {
            return;
        }
        try
        {
            ExternalInterface.call(param1);
            return;
        }
        catch(err:Error)
        {
            return;
        }
    }

    private function startTutorial() : void
    {
        tutorial_ = new Tutorial(this);
        addChild(tutorial_);
    }

    private function updateNearestInteractive() : void
    {
        var dist:Number = NaN;
        var go:GameObject = null;
        var iObj:IInteractiveObject = null;
        if(!this.map || !this.map.player_)
        {
            return;
        }
        var player:Player = this.map.player_;
        var minDist:Number = GeneralConstants.MAXIMUM_INTERACTION_DISTANCE;
        var closestInteractive:IInteractiveObject = null;
        var playerX:Number = player.x_;
        var playerY:Number = player.y_;
        for each(go in this.map.goDict_)
        {
            iObj = go as IInteractiveObject;
            if(iObj)
            {
                if(Math.abs(playerX - go.x_) < GeneralConstants.MAXIMUM_INTERACTION_DISTANCE || Math.abs(playerY - go.y_) < GeneralConstants.MAXIMUM_INTERACTION_DISTANCE)
                {
                    dist = PointUtil.distanceXY(go.x_,go.y_,playerX,playerY);
                    if(dist < GeneralConstants.MAXIMUM_INTERACTION_DISTANCE && dist < minDist)
                    {
                        minDist = dist;
                        closestInteractive = iObj;
                    }
                }
            }
        }
        this.mapModel.currentInteractiveTarget = closestInteractive;
    }

    public function onScreenResize(param1:Event) : void
    {
        var _loc2_:Boolean = Parameters.data_.uiscale;
        var _loc12_:Number = 800 / stage.stageWidth;
        var _loc13_:Number = 600 / stage.stageHeight;
        this.map.scaleX = _loc12_ *  (stage.scaleMode != StageScaleMode.EXACT_FIT?Parameters.data_.mscale:1);
        this.map.scaleY = _loc13_ * (stage.scaleMode != StageScaleMode.EXACT_FIT?Parameters.data_.mscale:1);
        if(this.hudView != null)
        {
            if(_loc2_)
            {
                this.hudView.scaleX = _loc12_ / _loc13_;
                this.hudView.scaleY = 1;
                this.hudView.y = 0;
            }
            else
            {
                this.hudView.scaleX = _loc12_;
                this.hudView.scaleY = _loc13_;
                this.hudView.y = 300 * (1 - _loc13_);
            }
            this.hudView.x = 800 - 200 * this.hudView.scaleX;
            if(this.creditDisplay_ != null)
            {
                this.creditDisplay_.x = this.hudView.x - 6 * this.creditDisplay_.scaleX;
            }
        }
        if(this.chatBox_ != null)
        {
            if(_loc2_)
            {
                this.chatBox_.scaleX = _loc12_ / _loc13_;
                this.chatBox_.scaleY = 1;
            }
            else
            {
                this.chatBox_.scaleX = _loc12_;
                this.chatBox_.scaleY = _loc13_;
            }
            this.chatBox_.y = 300 + 300 * (1 - this.chatBox_.scaleY);
        }
        if(this.rankText_ != null)
        {
            if(_loc2_)
            {
                this.rankText_.scaleX = _loc12_ / _loc13_;
                this.rankText_.scaleY = 1;
            }
            else
            {
                this.rankText_.scaleX = _loc12_;
                this.rankText_.scaleY = _loc13_;
            }
            this.rankText_.x = 8 * this.rankText_.scaleX;
            this.rankText_.y = 4 * this.rankText_.scaleY;
        }
        if(this.guildText_ != null)
        {
            if(_loc2_)
            {
                this.guildText_.scaleX = _loc12_ / _loc13_;
                this.guildText_.scaleY = 1;
            }
            else
            {
                this.guildText_.scaleX = _loc12_;
                this.guildText_.scaleY = _loc13_;
            }
            this.guildText_.x = 64 * this.guildText_.scaleX;
            this.guildText_.y = 6 * this.guildText_.scaleY;
        }
        if(this.creditDisplay_ != null)
        {
            if(_loc2_)
            {
                this.creditDisplay_.scaleX = _loc12_ / _loc13_;
                this.creditDisplay_.scaleY = 1;
            }
            else
            {
                this.creditDisplay_.scaleX = _loc12_;
                this.creditDisplay_.scaleY = _loc13_;
            }
        }
        if(this.giftStatusDisplay != null)
        {
            if(_loc2_)
            {
                this.giftStatusDisplay.scaleX = _loc12_ / _loc13_;
                this.giftStatusDisplay.scaleY = 1;
            }
            else
            {
                this.giftStatusDisplay.scaleX = _loc12_;
                this.giftStatusDisplay.scaleY = _loc13_;
            }
            this.giftStatusDisplay.x = 6 * this.giftStatusDisplay.scaleX;
            this.giftStatusDisplay.y = 62 * this.giftStatusDisplay.scaleY;
        }
    }

    public function onChatDown(param1:MouseEvent) : void
    {
        if(this.chatPlayerMenu != null)
        {
            this.removeChatPlayerMenu();
        }
        mui_.onMouseDown(param1);
    }

    public function onChatUp(param1:MouseEvent) : void
    {
        mui_.onMouseUp(param1);
    }

    private function onMoneyChanged(param1:Event) : void
    {
        gsc_.checkCredits();
    }

    private function onEnterFrame(param1:Event) : void
    {
        var _loc2_:Number = NaN;
        var _loc3_:int = getTimer();
        var _loc4_:int = _loc3_ - lastUpdate_;
        if(this.idleWatcher_.update(_loc4_))
        {
            closed.dispatch();
            return;
        }
        LoopedProcess.runProcesses(_loc3_);
        this.frameTimeSum_ = this.frameTimeSum_ + _loc4_;
        this.frameTimeCount_ = this.frameTimeCount_ + 1;
        if(this.frameTimeSum_ > 300000)
        {
            _loc2_ = int(Math.round(1000 * this.frameTimeCount_ / this.frameTimeSum_));
            this.frameTimeCount_ = 0;
            this.frameTimeSum_ = 0;
        }
        var _loc5_:int = getTimer();
        map.update(_loc3_,_loc4_);
        this.monitor.dispatch("Map.update",getTimer() - _loc5_);
        camera_.update(_loc4_);
        var _loc6_:Player = map.player_;
        if(this.focus)
        {
            camera_.configureCamera(this.focus,!!_loc6_?Boolean(Boolean(_loc6_.isHallucinating())):false);
            map.draw(camera_,_loc3_);
        }
        if(_loc6_ != null)
        {
            this.creditDisplay_.draw(_loc6_.credits_,_loc6_.fame_,_loc6_.tokens_,_loc6_.prestige_);
            this.drawCharacterWindow.dispatch(_loc6_);
            if(this.evalIsNotInCombatMapArea())
            {
                this.guildText_.draw(_loc6_.guildName_,_loc6_.guildRank_);
                this.guildText_.x = 190 + 16 * this.guildText_.scaleX;
            }
            if(_loc6_.isPaused())
            {
                map.filters = [PAUSED_FILTER];
                hudView.filters = [PAUSED_FILTER];
                map.mouseEnabled = false;
                map.mouseChildren = false;
                hudView.mouseEnabled = false;
                hudView.mouseChildren = false;
            }
            else if(map.filters.length > 0)
            {
                map.filters = [];
                hudView.filters = [];
                map.mouseEnabled = true;
                map.mouseChildren = true;
                hudView.mouseEnabled = true;
                hudView.mouseChildren = true;
            }
            moveRecords_.addRecord(_loc3_,_loc6_.x_,_loc6_.y_);
        }
        lastUpdate_ = _loc3_;
        var _loc7_:int = getTimer() - _loc3_;
        this.monitor.dispatch("GameSprite.loop",_loc7_);
    }
}
}

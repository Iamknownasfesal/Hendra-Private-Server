package kabam.rotmg.minimap.view {
import com.company.assembleegameclient.map.AbstractMap;
import com.company.assembleegameclient.map.GroundLibrary;
import com.company.assembleegameclient.objects.Character;
import com.company.assembleegameclient.objects.GameObject;
import com.company.assembleegameclient.objects.GuildHallPortal;
import com.company.assembleegameclient.objects.Player;
import com.company.assembleegameclient.objects.Portal;
import com.company.assembleegameclient.parameters.Parameters;
import com.company.assembleegameclient.ui.menu.PlayerGroupMenu;
import com.company.assembleegameclient.ui.tooltip.PlayerGroupToolTip;
import com.company.util.AssetLibrary;
import com.company.util.PointUtil;
import com.company.util.RectangleUtil;
import flash.display.BitmapData;
import flash.display.Graphics;
import flash.display.Shape;
import flash.events.Event;
import flash.events.MouseEvent;
import flash.geom.Matrix;
import flash.geom.Point;
import flash.utils.Dictionary;

public final class MiniMapImp extends MiniMap
{

    public static const MOUSE_DIST_SQ:int = 25;

    private static var objectTypeColorDict_:Dictionary = new Dictionary();


    public var _width:int;

    public var _height:int;

    public var zoomIndex:int = 0;

    public var active:Boolean = true;

    public var miniMapData_:BitmapData;

    public var zoomLevels:Vector.<Number>;

    public var groundLayer_:Shape;

    public var characterLayer_:Shape;

    public var w:int = 800;

    public var h:int = 600;

    private var focus:GameObject;

    private var isMouseOver:Boolean = false;

    private var tooltip:PlayerGroupToolTip = null;

    private var menu:PlayerGroupMenu = null;

    private var mapMatrix_:Matrix;

    private var players:Vector.<Player>;

    private var tempPoint:Point;

    public function MiniMapImp(param1:int, param2:int)
    {
        this.zoomLevels = new Vector.<Number>();
        this.mapMatrix_ = new Matrix();
        this.players = new Vector.<Player>();
        this.tempPoint = new Point();
        super();
        this._width = param1;
        this._height = param2;
        this.makeVisualLayers();
        this.addMouseListeners();
    }

    public static function gameObjectToColor(param1:GameObject) : uint
    {
        var _loc2_:int = param1.objectType_;
        if(!objectTypeColorDict_.hasOwnProperty(_loc2_.toString()))
        {
            objectTypeColorDict_[_loc2_] = param1.getColor();
        }
        return objectTypeColorDict_[_loc2_];
    }

    override public function setMap(param1:AbstractMap) : void
    {
        this.map = param1;
        this.makeViewModel();
    }

    override public function setFocus(param1:GameObject) : void
    {
        this.focus = param1;
    }

    private final function makeViewModel() : void
    {
        var _local2_:int = 0;
        this.miniMapData_ = new BitmapData(this.map.width_ + 2,map.height_ + 2,false,0);
        var _loc1_:Number = Math.max(this._width/ (map.width_ * 1.5),this._height / (map.height_ * 1.5));
        _local2_ = 4;
        while(_local2_ > _loc1_)
        {
            this.zoomLevels.push(_local2_);
            _local2_ = _local2_ / 2;
        }
        this.zoomLevels.push(_loc1_);
    }

    private final function makeVisualLayers() : void
    {
        graphics.clear();
        graphics.beginFill(2368548);
        graphics.lineStyle(3,7829367);
        graphics.drawCircle(0,0,85);
        graphics.endFill();
        this.groundLayer_ = new Shape();
        this.groundLayer_.x = -105;
        this.groundLayer_.y = -105;
        addChild(this.groundLayer_);
        this.characterLayer_ = new Shape();
        this.characterLayer_.x = -105;
        this.characterLayer_.y = -105;
        addChild(this.characterLayer_);
    }

    private final function addMouseListeners() : void
    {
        addEventListener("mouseOver",this.onMouseOver);
        addEventListener("mouseOut",this.onMouseOut);
        addEventListener("click",this.onMapClick);
        addEventListener("removedFromStage",this.onRemovedFromStage);
    }

    private final function onRemovedFromStage(param1:Event) : void
    {
        this.active = false;
        this.removeDecorations();
    }

    public final function dispose() : void
    {
        this.miniMapData_.dispose();
        this.miniMapData_ = null;
        this.removeDecorations();
    }

    private final function onMouseOver(param1:MouseEvent) : void
    {
        this.isMouseOver= true;
    }

    private final function onMouseOut(param1:MouseEvent) : void
    {
        this.isMouseOver = false;
    }

    private final function onMapClick(param1:MouseEvent) : void
    {
        if(this.tooltip == null || this.tooltip.parent == null || this.tooltip.players_ == null || this.tooltip.players_.length == 0)
        {
            return;
        }
        this.removeMenu();
        this.addMenu();
        this.removeTooltip();
    }

    private final function addMenu() : void
    {
        this.menu = new PlayerGroupMenu(map,this.tooltip.players_);
        this.menu.x = this.tooltip.x + 12;
        this.menu.y = this.tooltip.y;
        menuLayer.addChild(this.menu);
    }

    override public function setGroundTile(param1:int, param2:int, param3:uint) : void
    {
        var _loc4_:uint =  GroundLibrary.getColor(param3);
        this.miniMapData_.setPixel(param1 + 1,param2 + 1,_loc4_);
    }

    override public function setGameObjectTile(param1:int, param2:int, param3:GameObject) : void
    {
        var _loc4_:uint = gameObjectToColor(param3);
        this.miniMapData_.setPixel(param1 + 1,param2 + 1,_loc4_);
    }

    private final function removeDecorations() : void
    {
        this.removeTooltip();
        this.removeMenu();
    }

    private final function removeTooltip() : void
    {
        if(this.tooltip != null)
        {
            if(this.tooltip.parent != null)
            {
                this.tooltip.parent.removeChild(this.tooltip);
            }
            this.tooltip = null;
        }
    }

    private final function removeMenu() : void
    {
        if(this.menu != null)
        {
            if(this.menu.parent != null)
            {
                this.menu.parent.removeChild(this.menu);
            }
            this.menu = null;
        }
    }

    override public function draw() : void
    {
        var _loc6_:* = null;
        var _loc5_:* = null;
        var _loc3_:* = 0;
        var _loc15_:* = null;
        var _loc7_:* = NaN;
        var _loc8_:* = NaN;
        var _loc4_:Number = NaN;
        var _loc13_:Number = NaN;
        var _loc2_:Number = NaN;
        var _loc14_:Number = NaN;
        this.groundLayer_.graphics.clear();
        this.characterLayer_.graphics.clear();
        if(!this.focus || !this.active)
        {
            return;
        }
        var _loc1_:Number = this.zoomLevels[this.zoomIndex];
        var _loc11_:int = (this.w - 800) / 10.35;
        var _loc12_:int = (this.h - 600) / 7;
        this.mapMatrix_.identity();
        this.mapMatrix_.translate(-this.focus.x_,-this.focus.y_);
        this.mapMatrix_.scale(_loc1_ - 0.2,_loc1_ - 0.2);
        this.mapMatrix_.translate(this.x - _loc11_ + 1,this.y + _loc12_ + 1);
        _loc6_ = this.groundLayer_.graphics;
        _loc6_.beginBitmapFill(this.miniMapData_,this.mapMatrix_,false);
        _loc6_.drawCircle(this.x - _loc11_,this.y + _loc12_,85);
        _loc6_.endFill();
        _loc6_ = this.characterLayer_.graphics;
        var _loc9_:Number = mouseX + 70;
        var _loc10_:Number = mouseY + 95;
        this.players.length = 0;
        var _loc18_:int = 0;
        var _loc17_:* = map.goDict_;
        for each(_loc5_ in _loc17_)
        {
            if(!_loc5_.props_.noMiniMap_ || _loc5_ != this.focus)
            {
                _loc15_ = _loc5_ as Player;
                if(_loc15_ != null)
                {
                    if(_loc15_.isPaused)
                    {
                        _loc3_ = 8355711;
                    }
                    else if(_loc15_.isFellowGuild_)
                    {
                        _loc3_ = 65280;
                    }
                    else if(Parameters.data_.newMiniMapColors && !_loc15_.nameChosen_ && _loc15_.starred_)
                    {
                        _loc3_ = 16777215;
                    }
                    else if(Parameters.data_.newMiniMapColors && !_loc15_.nameChosen_)
                    {
                        _loc3_ = 13619151;
                    }
                    else if(Parameters.data_.newMiniMapColors && !_loc15_.starred_)
                    {
                        _loc3_ = 13618944;
                    }
                    else
                    {
                        _loc3_ = 16776960;
                    }
                }
                else if(_loc5_ is Portal || _loc5_ is GuildHallPortal)
                {
                    _loc3_ = 255;
                }
                else
                {
                    if(_loc5_ is Portal)
                    {
                        _loc3_ = uint(139);
                    }
                    if(_loc5_ is Character)
                    {
                        if(_loc5_.props_.isEnemy_)
                        {
                            _loc3_ = uint(16711680);
                        }
                        else
                        {
                            _loc3_ = uint(gameObjectToColor(_loc5_));
                        }
                    }
                    else
                    {
                        continue;
                    }
                }
                _loc7_ = Number(this.mapMatrix_.a * _loc5_.x_ + this.mapMatrix_.c * _loc5_.x_ + this.mapMatrix_.tx + 0.25 * _loc1_);
                _loc8_ = Number(this.mapMatrix_.b * _loc5_.y_ + this.mapMatrix_.d * _loc5_.y_ + this.mapMatrix_.ty + 0.25 * _loc1_);
                if(Math.pow(_loc7_ - (this.x - _loc11_),2) + Math.pow(_loc8_ - (this.y + _loc12_),2) > 5625)
                {
                    continue;
                }
                if(_loc15_ != null && _loc15_ != this.map.player_.map_.player_ && this.isMouseOver && (this.menu == null || this.menu.parent == null))
                {
                    _loc4_ = _loc9_ - _loc7_;
                    _loc13_ = _loc10_ - _loc8_;
                    _loc2_ = _loc4_ * _loc4_ + _loc13_ * _loc13_;
                    if(_loc2_ < 25)
                    {
                        this.players.push(_loc15_);
                    }
                }
                _loc6_.beginFill(_loc3_);
                _loc6_.drawRect(_loc7_,_loc8_,4,4);
                _loc6_.endFill();
            }
        }
        if(this.players.length != 0)
        {
            if(this.tooltip == null)
            {
                this.tooltip = new PlayerGroupToolTip(this.players);
                menuLayer.addChild(this.tooltip);
            }
            else if(!this.areSamePlayers(this.tooltip.players_,this.players))
            {
                this.tooltip.setPlayers(this.players);
            }
        }
        else if(this.tooltip != null)
        {
            if(this.tooltip.parent != null)
            {
                this.tooltip.parent.removeChild(this.tooltip);
            }
            this.tooltip = null;
        }
    }

    private final function areSamePlayers(param1:Vector.<Player>, param2:Vector.<Player>) : Boolean
    {
        var _loc4_:int = 0;
        var _loc3_:int = param1.length;
        if(_loc3_ != param2.length)
        {
            return false;
        }
        _loc4_ = 0;
        while(_loc4_ < _loc3_)
        {
            if(param1[_loc4_] != param2[_loc4_])
            {
                return false;
            }
            _loc4_++;
        }
        return true;
    }

    override public function zoomIn() : void
    {
        this.zoomIndex = Math.max(0,this.zoomIndex - 1);
    }

    override public function zoomOut() : void
    {
        this.zoomIndex = Math.min(this.zoomLevels.length - 1,this.zoomIndex + 1);
    }

    override public function deactivate() : void
    {
    }
}
}
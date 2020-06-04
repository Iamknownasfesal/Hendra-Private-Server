package kabam.rotmg.ui.view
{
import com.company.assembleegameclient.game.AGameSprite;
import com.company.assembleegameclient.game.GameSprite;
import com.company.assembleegameclient.objects.Player;
import com.company.assembleegameclient.objects.GameObject;
import com.company.assembleegameclient.ui.TradePanel;
import com.company.assembleegameclient.ui.panels.InteractPanel;
import com.company.assembleegameclient.ui.panels.itemgrids.EquippedGrid;
import com.company.util.GraphicsUtil;
import com.company.util.SpriteUtil;
import flash.display.GraphicsPath;
import flash.display.GraphicsSolidFill;
import flash.display.IGraphicsData;
import flash.display.Sprite;
import flash.events.Event;
import flash.geom.Point;

import kabam.rotmg.game.view.components.TabConstants;
import kabam.rotmg.game.view.components.TabStripView;
import kabam.rotmg.messaging.impl.incoming.TradeAccepted;
import kabam.rotmg.messaging.impl.incoming.TradeChanged;
import kabam.rotmg.messaging.impl.incoming.TradeStart;
import kabam.rotmg.minimap.view.MiniMapImp;

public class HUDView extends Sprite implements UnFocusAble
{

    private const MAP_POSITION:Point = new Point(105,105);

    private const EQUIPMENT_INVENTORY_POSITION:Point = new Point(7,250);

    private const TAB_STRIP_POSITION:Point = new Point(7,346);

    private const INTERACT_PANEL_POSITION:Point = new Point(0,500);

    public var tabStrip:TabStripView;

    public var interactPanel:InteractPanel;

    public var tradePanel:TradePanel;

    private var miniMap:MiniMapImp;

    public var equippedGrid:EquippedGrid;

    public var equippedGridBG:Sprite;

    private var player:Player;

    public var BG:Sprite;

    public function HUDView()
    {
        super();
        this.createAssets();
        this.addAssets();
        this.positionAssets();
    }

    public function setPlayerDependentAssets(param1:GameSprite) : void
    {
        this.player = param1.map.player_;
        this.createEquippedGridBackground();
        this.createEquippedGrid();
        this.createInteractPanel(param1);
    }

    public function draw() : void
    {
        if(this.equippedGrid)
        {
            this.equippedGrid.draw();
        }
        if(this.interactPanel)
        {
            this.interactPanel.draw();
        }
    }

    public function startTrade(param1:GameSprite, param2:TradeStart) : void
    {
        if(!this.tradePanel)
        {
            this.tradePanel = new TradePanel(param1,param2);
            this.tradePanel.y = 200;
            this.tradePanel.addEventListener(Event.CANCEL,this.onTradeCancel);
            addChild(this.tradePanel);
            this.setNonTradePanelAssetsVisible(false);
        }
    }

    public function tradeDone() : void
    {
        this.removeTradePanel();
    }

    public function tradeChanged(param1:TradeChanged) : void
    {
        if(this.tradePanel)
        {
            this.tradePanel.setYourOffer(param1.offer_);
        }
    }

    public function tradeAccepted(param1:TradeAccepted) : void
    {
        if(this.tradePanel)
        {
            this.tradePanel.youAccepted(param1.myOffer_,param1.yourOffer_);
        }
    }

    private function createAssets() : void
    {
        this.miniMap = new MiniMapImp(192,192);
        this.tabStrip = new TabStripView();
    }

    private function addAssets() : void
    {
        addChild(this.miniMap);
        addChild(this.tabStrip);
    }

    private function positionAssets() : void
    {
        this.miniMap.x = this.MAP_POSITION.x;
        this.miniMap.y = this.MAP_POSITION.y;
        this.tabStrip.x = this.TAB_STRIP_POSITION.x;
        this.tabStrip.y = this.TAB_STRIP_POSITION.y;
    }

    private function createInteractPanel(param1:GameSprite) : void
    {
        var _loc1_:Vector.<IGraphicsData> = null;
        _loc1_ = null;
        var _loc2_:GraphicsSolidFill = new GraphicsSolidFill(TabConstants.BACKGROUND_COLOR,1);
        var _loc3_:GraphicsPath = new GraphicsPath(new Vector.<int>(),new Vector.<Number>());
        _loc1_ = new <IGraphicsData>[_loc2_,_loc3_,GraphicsUtil.END_FILL];
        GraphicsUtil.drawCutEdgeRect(0,0,186,92,6,[1,1,1,1],_loc3_);
        this.BG = new Sprite();
        this.BG.x = this.EQUIPMENT_INVENTORY_POSITION.x -3;
        this.BG.y = this.INTERACT_PANEL_POSITION.y + 3;
        this.BG.graphics.drawGraphicsData(_loc1_);
        addChild(this.BG);
        this.BG.visible = false;
        this.interactPanel = new InteractPanel(param1,this.player,200,100);
        this.interactPanel.x = this.INTERACT_PANEL_POSITION.x;
        this.interactPanel.y = this.INTERACT_PANEL_POSITION.y;
        addChild(this.interactPanel);
    }

    private function createEquippedGrid() : void
    {
        this.equippedGrid = new EquippedGrid(this.player);
        this.equippedGrid.x = this.EQUIPMENT_INVENTORY_POSITION.x + 4;
        this.equippedGrid.y = this.EQUIPMENT_INVENTORY_POSITION.y;
        addChild(this.equippedGrid);
    }

    private function createEquippedGridBackground() : void
    {
        var _loc1_:Vector.<IGraphicsData> = null;
        _loc1_ = null;
        var _loc2_:GraphicsSolidFill = new GraphicsSolidFill(TabConstants.BACKGROUND_COLOR,1);
        var _loc3_:GraphicsPath = new GraphicsPath(new Vector.<int>(),new Vector.<Number>());
        _loc1_ = new <IGraphicsData>[_loc2_,_loc3_,GraphicsUtil.END_FILL];
        GraphicsUtil.drawCutEdgeRect(0,0,186,92,6,[1,1,1,1],_loc3_);
        this.equippedGridBG = new Sprite();
        this.equippedGridBG.x = this.EQUIPMENT_INVENTORY_POSITION.x - 3;
        this.equippedGridBG.y = this.EQUIPMENT_INVENTORY_POSITION.y - 3;
        this.equippedGridBG.graphics.drawGraphicsData(_loc1_);
        addChild(this.equippedGridBG);
    }

    private function setNonTradePanelAssetsVisible(param1:Boolean) : void
    {
        this.tabStrip.visible = param1;
        this.equippedGrid.visible = param1;
        this.equippedGridBG.visible = param1;
        this.interactPanel.visible = param1;
    }

    private function removeTradePanel() : void
    {
        if(this.tradePanel)
        {
            SpriteUtil.safeRemoveChild(this,this.tradePanel);
            this.tradePanel.removeEventListener(Event.CANCEL,this.onTradeCancel);
            this.tradePanel = null;
            this.setNonTradePanelAssetsVisible(true);
        }
    }

    private function onTradeCancel(param1:Event) : void
    {
        this.removeTradePanel();
    }

    public function setMiniMapFocus(object:GameObject) : void {
        this.miniMap.setFocus(object);
    }
}
}

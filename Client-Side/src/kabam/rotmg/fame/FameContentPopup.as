package kabam.rotmg.fame {
import flash.display.Bitmap;
import flash.display.BitmapData;
import flash.display.Sprite;
import flash.geom.Rectangle;
import kabam.rotmg.ui.buttons.SliceScalingButton;
import kabam.rotmg.ui.defaults.DefaultLabelFormat;
import kabam.rotmg.ui.labels.UILabel;
import kabam.rotmg.ui.popups.header.PopupHeader;
import kabam.rotmg.ui.popups.modal.ModalPopup;
import kabam.rotmg.ui.scroll.UIScrollbar;
import kabam.rotmg.ui.sliceScaling.SliceScalingBitmap;
import kabam.rotmg.ui.tabs.UITab;
import kabam.rotmg.ui.tabs.UITabs;
import kabam.rotmg.ui.texture.TextureParser;
import kabam.rotmg.assets.services.IconFactory;

public class FameContentPopup extends ModalPopup
{


    private var characterDecorationBG:SliceScalingBitmap;

    private var contentTabs:SliceScalingBitmap;

    private var contentInset:SliceScalingBitmap;

    private var fameBitmap:Bitmap;

    private var fameOnDeathTitle:UILabel;

    private var fameOnDeathLabel:UILabel;

    private var statsLinesPosition:int = 0;

    private var dungeonLinesPosition:int = 0;

    private var statsContainer:Sprite;

    private var dungeonContainer:Sprite;

    private var totalFameBitmap:Bitmap;

    private var totalFame:UILabel;

    private var characterNameLabel:UILabel;

    private var characterInfoLabel:UILabel;

    private var characterDateLabel:UILabel;

    public var infoButton:SliceScalingButton;

    public var characterId:int;

    private var tabs:UITabs;

    public function FameContentPopup(param1:int = -1)
    {
        super(340,505,"Fame Overview",DefaultLabelFormat.defaultSmallPopupTitle,new Rectangle(0,0,340,565));
        this.characterId = param1;
        this.infoButton = new SliceScalingButton(TextureParser.instance.getSliceScalingBitmap("UI","info_button"));
        _header.addButton(this.infoButton,PopupHeader.LEFT_BUTTON);
        this.characterNameLabel = new UILabel();
        this.characterInfoLabel = new UILabel();
        this.characterDateLabel = new UILabel();
        DefaultLabelFormat.characterFameNameLabel(this.characterNameLabel);
        DefaultLabelFormat.characterFameInfoLabel(this.characterInfoLabel);
        DefaultLabelFormat.characterFameInfoLabel(this.characterDateLabel);
        this.characterNameLabel.x = 75;
        this.characterNameLabel.y = 8;
        this.characterInfoLabel.x = 75;
        this.characterInfoLabel.y = 30;
        this.characterDateLabel.x = 75;
        this.characterDateLabel.y = 42;
        addChild(this.characterNameLabel);
        addChild(this.characterInfoLabel);
        addChild(this.characterDateLabel);
        this.totalFame = new UILabel();
        DefaultLabelFormat.currentFameLabel(this.totalFame);
        addChild(this.totalFame);
        var _loc2_:BitmapData = IconFactory.makeFame();
        this.fameBitmap = new Bitmap(_loc2_);
        addChild(this.fameBitmap);
        var _loc3_:BitmapData = IconFactory.makeFame();
        this.totalFameBitmap = new Bitmap(_loc3_);
        addChild(this.totalFameBitmap);
        this.characterDecorationBG = TextureParser.instance.getSliceScalingBitmap("UI","popup_content_decoration",69);
        addChild(this.characterDecorationBG);
        this.characterDecorationBG.height = 80;
        this.characterDecorationBG.x = 0;
        this.characterDecorationBG.y = 5;
        this.contentTabs = TextureParser.instance.getSliceScalingBitmap("UI","tab_inset_content_background",340);
        addChild(this.contentTabs);
        this.contentTabs.height = 45;
        this.contentTabs.x = 0;
        this.contentTabs.y = 90;
        this.contentInset = TextureParser.instance.getSliceScalingBitmap("UI","popup_content_inset",340);
        addChild(this.contentInset);
        this.contentInset.height = 353;
        this.contentInset.x = 0;
        this.contentInset.y = 125;
        this.tabs = new UITabs(340,true);
        this.tabs.addTab(this.createStatsTab(),true);
        this.tabs.addTab(this.createDungeonTab());
        this.tabs.y = 91;
        this.tabs.x = 0;
        addChild(this.tabs);
        this.fameOnDeathTitle = new UILabel();
        DefaultLabelFormat.deathFameLabel(this.fameOnDeathTitle);
        addChild(this.fameOnDeathTitle);
        this.fameOnDeathLabel = new UILabel();
        DefaultLabelFormat.deathFameCount(this.fameOnDeathLabel);
        addChild(this.fameOnDeathLabel);
    }

    private function createStatsTab() : UITab
    {
        var _loc1_:UITab = null;
        var _loc2_:Sprite = null;
        var _loc4_:Sprite = null;
        _loc1_ = new UITab("Statistics",true);
        _loc2_ = new Sprite();
        this.statsContainer = new Sprite();
        this.statsContainer.x = this.contentInset.x;
        this.statsContainer.y = 2;
        _loc2_.addChild(this.statsContainer);
        var _loc3_:UIScrollbar = new UIScrollbar(338);
        _loc3_.mouseRollSpeedFactor = 1;
        _loc3_.scrollObject = _loc1_;
        _loc3_.content = this.statsContainer;
        _loc2_.addChild(_loc3_);
        _loc3_.x = this.contentInset.x + this.contentInset.width - 25;
        _loc3_.y = 7;
        _loc4_ = new Sprite();
        _loc4_.graphics.beginFill(0);
        _loc4_.graphics.drawRect(0,0,340,342);
        _loc4_.x = this.statsContainer.x;
        _loc4_.y = this.statsContainer.y;
        this.statsContainer.mask = _loc4_;
        _loc2_.addChild(_loc4_);
        _loc1_.addContent(_loc2_);
        return _loc1_;
    }

    private function createDungeonTab() : UITab
    {
        var _loc1_:UITab = new UITab("Dungeons",true);
        var _loc2_:Sprite = new Sprite();
        this.dungeonContainer = new Sprite();
        this.dungeonContainer.x = this.contentInset.x;
        this.dungeonContainer.y = 2;
        _loc2_.addChild(this.dungeonContainer);
        var _loc3_:UIScrollbar = new UIScrollbar(338);
        _loc3_.mouseRollSpeedFactor = 1;
        _loc3_.scrollObject = _loc1_;
        _loc3_.content = this.dungeonContainer;
        _loc2_.addChild(_loc3_);
        _loc3_.x = this.contentInset.x + this.contentInset.width - 25;
        _loc3_.y = 7;
        var _loc4_:Sprite = new Sprite();
        _loc4_.graphics.beginFill(0);
        _loc4_.graphics.drawRect(0,0,340,342);
        _loc4_.x = this.dungeonContainer.x;
        _loc4_.y = this.dungeonContainer.y;
        this.dungeonContainer.mask = _loc4_;
        _loc2_.addChild(_loc4_);
        _loc1_.addContent(_loc2_);
        return _loc1_;
    }

    public function set fameOnDeath(param1:int) : void
    {
        this.fameOnDeathTitle.text = "Fame on Death:";
        this.fameOnDeathTitle.x = 0;
        this.fameOnDeathTitle.y = 485;
        this.fameOnDeathLabel.text = param1.toString();
        this.fameOnDeathLabel.x = 330 - this.fameOnDeathLabel.textWidth - this.fameBitmap.width;
        this.fameOnDeathLabel.y = 485;
        this.fameBitmap.x = this.fameOnDeathLabel.x + this.fameOnDeathLabel.textWidth + 3;
        this.fameBitmap.y = this.fameOnDeathLabel.y;
    }

    public function setCharacterData(param1:int, param2:String, param3:int, param4:String, param5:String, param6:BitmapData) : void
    {
        this.totalFame.text = param1.toString();
        this.totalFame.x = 75;
        this.totalFame.y = 60;
        this.totalFameBitmap.x = this.totalFame.x + this.totalFame.textWidth + 3;
        this.totalFameBitmap.y = this.totalFame.y + 1;
        this.characterNameLabel.text = param2;
        this.characterInfoLabel.text = "Level " + param3 + ", " + param4;
        this.characterDateLabel.text = "Created on " + param5;
        var _loc7_:Bitmap = new Bitmap(param6);
        _loc7_.x = Math.round(this.characterDecorationBG.x + (68 - _loc7_.width) / 2);
        _loc7_.y = Math.round(this.characterDecorationBG.y + (80 - _loc7_.height) / 2);
        addChild(_loc7_);
    }

    public function addDungeonLine(param1:StatsLine) : void
    {
        var _loc2_:int = 0;
        if(this.dungeonLinesPosition >= 1)
        {
            _loc2_ = 5;
        }
        else
        {
            _loc2_ = 0;
        }
        param1.x = 6;
        param1.y = this.dungeonLinesPosition * 27 - _loc2_;
        this.dungeonContainer.addChild(param1);
        if(this.dungeonLinesPosition % 2 == 1)
        {
            param1.drawBrightBackground();
        }
        this.dungeonLinesPosition++;
    }

    public function addStatLine(param1:StatsLine) : void
    {
        param1.x = 6;
        param1.y = this.statsLinesPosition * 22;
        this.statsContainer.addChild(param1);
        if(this.statsLinesPosition % 2 == 1)
        {
            param1.drawBrightBackground();
        }
        this.statsLinesPosition++;
    }
}
}

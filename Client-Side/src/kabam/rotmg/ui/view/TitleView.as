package kabam.rotmg.ui.view
{
import com.company.assembleegameclient.screens.AccountScreen;
import com.company.assembleegameclient.screens.TitleMenuOption;
import com.company.assembleegameclient.ui.SoundIcon;
import flash.display.Sprite;
import flash.events.MouseEvent;
import flash.external.ExternalInterface;
import flash.filters.DropShadowFilter;
import flash.text.TextFieldAutoSize;
import kabam.rotmg.account.transfer.view.KabamLoginView;
import kabam.rotmg.application.model.PlatformModel;
import kabam.rotmg.application.model.PlatformType;
import kabam.rotmg.core.StaticInjectorContext;
import kabam.rotmg.dialogs.control.OpenDialogSignal;
import kabam.rotmg.text.model.TextKey;
import kabam.rotmg.text.view.TextFieldDisplayConcrete;
import kabam.rotmg.text.view.stringBuilder.LineBuilder;
import kabam.rotmg.text.view.stringBuilder.StaticStringBuilder;
import kabam.rotmg.ui.model.EnvironmentData;
import kabam.rotmg.ui.view.components.DarkLayer;
import kabam.rotmg.ui.view.components.MapBackground;
import kabam.rotmg.ui.view.components.MenuOptionsBar;
import org.osflash.signals.Signal;
import org.osflash.signals.natives.NativeMappedSignal;

public class TitleView extends Sprite
{

    static var TitleScreenGraphic:Class = TitleView_TitleScreenGraphic;

    public static const MIDDLE_OF_BOTTOM_BAND:Number = 589.45;

    public static var queueEmailConfirmation:Boolean = false;

    public static var queuePasswordPrompt:Boolean = false;

    public static var queuePasswordPromptFull:Boolean = false;

    public static var queueRegistrationPrompt:Boolean = false;

    public static var kabammigrateOpened:Boolean = false;


    private var versionText:TextFieldDisplayConcrete;

    private var copyrightText:TextFieldDisplayConcrete;

    private var menuOptionsBar:MenuOptionsBar;

    private var data:EnvironmentData;

    public var playClicked:Signal;

    public var serversClicked:Signal;

    public var accountClicked:Signal;

    public var languagesClicked:Signal;

    public var kabamTransferClicked:Signal;

    public var editorClicked:Signal;

    public var quitClicked:Signal;

    public var optionalButtonsAdded:Signal;

    private var migrateButton:TitleMenuOption;

    public function TitleView()
    {
        var _loc1_:String = null;
        this.menuOptionsBar = this.makeMenuOptionsBar();
        this.optionalButtonsAdded = new Signal();
        super();
        addChild(new MapBackground());
        addChild(new DarkLayer());
        addChild(new TitleScreenGraphic());
        addChild(this.menuOptionsBar);
        this.makeChildren();
        var _loc2_:PlatformModel = StaticInjectorContext.getInjector().getInstance(PlatformModel);
        if(_loc2_.getPlatform() == PlatformType.WEB)
        {
            _loc1_ = "";
            try
            {
                _loc1_ = ExternalInterface.call("window.location.search.substring",1);
            }
            catch(err:Error)
            {
            }
            if(!kabammigrateOpened && _loc1_ && _loc1_ == "kabammigrate")
            {
                kabammigrateOpened = true;
            }
        }
        else if(_loc2_.getPlatform() == PlatformType.KABAM)
        {
        }
    }

    private function makeMenuOptionsBar() : MenuOptionsBar
    {
        var _loc1_:TitleMenuOption = ButtonFactory.getPlayButton();
        var _loc2_:TitleMenuOption = ButtonFactory.getServersButton();
        var _loc3_:TitleMenuOption = ButtonFactory.getAccountButton();
        this.playClicked = _loc1_.clicked;
        this.serversClicked = _loc2_.clicked;
        this.accountClicked = _loc3_.clicked;
        var _loc6_:MenuOptionsBar = new MenuOptionsBar();
        _loc6_.addButton(_loc1_,MenuOptionsBar.CENTER);
        _loc6_.addButton(_loc2_,MenuOptionsBar.LEFT);
        _loc6_.addButton(_loc3_,MenuOptionsBar.RIGHT);
        return _loc6_;
    }

    private function makeChildren() : void
    {
        this.versionText = this.makeText().setHTML(true).setAutoSize(TextFieldAutoSize.LEFT).setVerticalAlign(TextFieldDisplayConcrete.MIDDLE);
        this.versionText.y = MIDDLE_OF_BOTTOM_BAND;
        addChild(this.versionText);
    }

    public function makeText() : TextFieldDisplayConcrete
    {
        var _loc1_:TextFieldDisplayConcrete = null;
        _loc1_ = null;
        _loc1_ = new TextFieldDisplayConcrete().setSize(12).setColor(8355711);
        _loc1_.filters = [new DropShadowFilter(0,0,0)];
        return _loc1_;
    }

    public function initialize(param1:EnvironmentData) : void
    {
        this.data = param1;
        this.updateVersionText();
        this.handleOptionalButtons();
    }

    public function putNoticeTagToOption(param1:TitleMenuOption, param2:String, param3:int = 14, param4:uint = 10092390, param5:Boolean = true) : void
    {
        param1.createNoticeTag(param2,param3,param4,param5);
    }

    private function updateVersionText() : void
    {
        this.versionText.setStringBuilder(new StaticStringBuilder(this.data.buildLabel));
    }

    private function handleOptionalButtons() : void
    {
        this.data.canMapEdit && this.createEditorButton();
        this.optionalButtonsAdded.dispatch();
    }

    private function createEditorButton() : void
    {
        var _loc1_:TitleMenuOption = ButtonFactory.getEditorButton();
        this.menuOptionsBar.addButton(_loc1_,MenuOptionsBar.RIGHT);
        this.editorClicked = _loc1_.clicked;
    }
}
}

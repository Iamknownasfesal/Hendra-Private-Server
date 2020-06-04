package kabam.rotmg.ui.view
{
import com.company.assembleegameclient.mapeditor.MapEditor;
import com.company.assembleegameclient.screens.ServersScreen;
import com.company.assembleegameclient.ui.language.LanguageOptionOverlay;
import flash.events.Event;
import flash.external.ExternalInterface;
import flash.net.URLRequest;
import flash.net.URLRequestMethod;
import flash.net.URLVariables;
import flash.net.navigateToURL;
import flash.system.Capabilities;
import kabam.rotmg.account.core.Account;
import kabam.rotmg.account.core.signals.OpenAccountInfoSignal;
import kabam.rotmg.account.securityQuestions.data.SecurityQuestionsModel;
import kabam.rotmg.account.securityQuestions.view.SecurityQuestionsInfoDialog;
import kabam.rotmg.application.DynamicSettings;
import kabam.rotmg.application.api.ApplicationSetup;
import kabam.rotmg.core.model.PlayerModel;
import kabam.rotmg.core.signals.SetScreenSignal;
import kabam.rotmg.core.signals.SetScreenWithValidDataSignal;
import kabam.rotmg.core.view.Layers;
import kabam.rotmg.dialogs.control.OpenDialogSignal;
import kabam.rotmg.ui.model.EnvironmentData;
import kabam.rotmg.ui.signals.EnterGameSignal;
import robotlegs.bender.bundles.mvcs.Mediator;
import robotlegs.bender.framework.api.ILogger;

public class TitleMediator extends Mediator
{

    private static var supportCalledBefore:Boolean = false;


    [Inject]
    public var view:TitleView;

    [Inject]
    public var account:Account;

    [Inject]
    public var playerModel:PlayerModel;

    [Inject]
    public var setScreen:SetScreenSignal;

    [Inject]
    public var setScreenWithValidData:SetScreenWithValidDataSignal;

    [Inject]
    public var enterGame:EnterGameSignal;

    [Inject]
    public var openAccountInfo:OpenAccountInfoSignal;

    [Inject]
    public var openDialog:OpenDialogSignal;

    [Inject]
    public var setup:ApplicationSetup;

    [Inject]
    public var layers:Layers;

    [Inject]
    public var securityQuestionsModel:SecurityQuestionsModel;

    [Inject]
    public var logger:ILogger;

    public function TitleMediator()
    {
        super();
    }

    override public function initialize() : void
    {
        this.view.optionalButtonsAdded.add(this.onOptionalButtonsAdded);
        this.view.initialize(this.makeEnvironmentData());
        this.view.playClicked.add(this.handleIntentionToPlay);
        this.view.serversClicked.add(this.showServersScreen);
        this.view.accountClicked.add(this.handleIntentionToReviewAccount);
        if(this.playerModel.isNewToEditing())
        {
            this.view.putNoticeTagToOption(ButtonFactory.getEditorButton(),"");
        }
        if(this.securityQuestionsModel.showSecurityQuestionsOnStartup)
        {
            this.openDialog.dispatch(new SecurityQuestionsInfoDialog());
        }
    }

    private function openSupportPage() : void
    {
        var _loc1_:URLVariables = new URLVariables();
        var _loc2_:URLRequest = new URLRequest();
        var _loc3_:Boolean = false;
        if(DynamicSettings.settingExists("SalesforceMobile") && DynamicSettings.getSettingValue("SalesforceMobile") == 1)
        {
            _loc3_ = true;
        }
        var _loc4_:String = this.playerModel.getSalesForceData();
        if(_loc4_ == "unavailable" || !_loc3_)
        {
            _loc1_.language = "en_US";
            _loc1_.game = "a0Za000000jIBFUEA4";
            _loc1_.issue = "Other_Game_Issues";
            _loc2_.url = "http://rotmg.decagames.io";
            _loc2_.method = URLRequestMethod.GET;
            _loc2_.data = _loc1_;
            navigateToURL(_loc2_,"_blank");
        }
        else if(Capabilities.playerType == "PlugIn" || Capabilities.playerType == "ActiveX")
        {
            if(!supportCalledBefore)
            {
                ExternalInterface.call("openSalesForceFirstTime",_loc4_);
                supportCalledBefore = true;
            }
            else
            {
                ExternalInterface.call("reopenSalesForce");
            }
        }
        else
        {
            _loc1_.data = _loc4_;
            _loc2_.url = "http://rotmg.decagames.io";
            _loc2_.method = URLRequestMethod.GET;
            _loc2_.data = _loc1_;
            navigateToURL(_loc2_,"_blank");
        }
    }

    private function onOptionalButtonsAdded() : void
    {
        this.view.editorClicked && this.view.editorClicked.add(this.showMapEditor);
        this.view.quitClicked && this.view.quitClicked.add(this.attemptToCloseClient);
    }

    private function showLanguagesScreen() : void
    {
        this.setScreen.dispatch(new LanguageOptionOverlay());
    }

    private function makeEnvironmentData() : EnvironmentData
    {
        var _loc1_:EnvironmentData = new EnvironmentData();
        _loc1_.isDesktop = Capabilities.playerType == "Desktop";
        _loc1_.canMapEdit = this.playerModel.isAdmin() || this.playerModel.mapEditor();
        _loc1_.buildLabel = this.setup.getBuildLabel().replace("RotMG","NillysReborn");
        return _loc1_;
    }

    override public function destroy() : void
    {
        this.view.playClicked.remove(this.handleIntentionToPlay);
        this.view.serversClicked.remove(this.showServersScreen);
        this.view.accountClicked.remove(this.handleIntentionToReviewAccount);
        this.view.optionalButtonsAdded.remove(this.onOptionalButtonsAdded);
        this.view.editorClicked && this.view.editorClicked.remove(this.showMapEditor);
        this.view.quitClicked && this.view.quitClicked.remove(this.attemptToCloseClient);
    }

    private function handleIntentionToPlay() : void
    {
        this.enterGame.dispatch();
    }

    private function showServersScreen() : void
    {
        this.setScreen.dispatch(new ServersScreen());
    }

    private function handleIntentionToReviewAccount() : void
    {
        this.openAccountInfo.dispatch(false);
    }

    private function showMapEditor() : void
    {
        this.setScreen.dispatch(new MapEditor());
    }

    private function attemptToCloseClient() : void
    {
        dispatch(new Event("APP_CLOSE_EVENT"));
    }
}
}

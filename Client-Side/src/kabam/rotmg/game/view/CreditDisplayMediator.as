package kabam.rotmg.game.view {
import kabam.rotmg.account.core.signals.OpenMoneyWindowSignal;
import kabam.rotmg.core.model.PlayerModel;

import robotlegs.bender.bundles.mvcs.Mediator;

public class CreditDisplayMediator extends Mediator {

    [Inject]
    public var view:CreditDisplay;
    [Inject]
    public var model:PlayerModel;
    [Inject]
    public var openMoneyWindow:OpenMoneyWindowSignal;


    override public function initialize():void {
        this.model.creditsChanged.add(this.onCreditsChanged);
        this.model.fameChanged.add(this.onFameChanged);
        this.model.prestigeChanged.add(this.onPrestigeChanged);
        this.view.openAccountDialog.add(this.onOpenAccountDialog);
    }

    override public function destroy():void {
        this.model.creditsChanged.remove(this.onCreditsChanged);
        this.model.fameChanged.remove(this.onFameChanged);
        this.model.prestigeChanged.remove(this.onPrestigeChanged);
        this.view.openAccountDialog.remove(this.onOpenAccountDialog);
    }

    private function onCreditsChanged(param1:int) : void
    {
        this.view.draw(this.model.getCredits(),this.model.getFame(),param1);
    }

    private function onFameChanged(param1:int) : void
    {
        this.view.draw(this.model.getCredits(),this.model.getFame(),param1);
    }

    private function onPrestigeChanged(param1:int) : void
    {
        this.view.draw(this.model.getCredits(),this.model.getFame(),this.model.getPrestige());
    }

    private function onOpenAccountDialog():void {
        this.openMoneyWindow.dispatch();
    }


}
}

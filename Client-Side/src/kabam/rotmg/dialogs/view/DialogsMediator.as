package kabam.rotmg.dialogs.view
{
import flash.display.Sprite;
import kabam.rotmg.dialogs.control.AddPopupToStartupQueueSignal;
import kabam.rotmg.dialogs.control.CloseDialogsSignal;
import kabam.rotmg.dialogs.control.FlushPopupStartupQueueSignal;
import kabam.rotmg.dialogs.control.OpenDialogNoModalSignal;
import kabam.rotmg.dialogs.control.OpenDialogSignal;
import kabam.rotmg.dialogs.control.PopDialogSignal;
import kabam.rotmg.dialogs.control.PushDialogSignal;
import kabam.rotmg.dialogs.control.ShowDialogBackgroundSignal;
import kabam.rotmg.dialogs.model.DialogsModel;
import kabam.rotmg.dialogs.model.PopupQueueEntry;
import org.osflash.signals.Signal;
import robotlegs.bender.bundles.mvcs.Mediator;

public class DialogsMediator extends Mediator
{


    [Inject]
    public var view:DialogsView;

    [Inject]
    public var openDialogNoModal:OpenDialogNoModalSignal;

    [Inject]
    public var openDialog:OpenDialogSignal;

    [Inject]
    public var closeDialog:CloseDialogsSignal;

    [Inject]
    public var showDialogBackground:ShowDialogBackgroundSignal;

    [Inject]
    public var pushDialogSignal:PushDialogSignal;

    [Inject]
    public var popDialogSignal:PopDialogSignal;

    [Inject]
    public var addToQueueSignal:AddPopupToStartupQueueSignal;

    [Inject]
    public var flushStartupQueue:FlushPopupStartupQueueSignal;

    [Inject]
    public var dialogsModel:DialogsModel;

    public function DialogsMediator()
    {
        super();
    }

    override public function initialize() : void
    {
        this.showDialogBackground.add(this.onShowDialogBackground);
        this.openDialog.add(this.onOpenDialog);
        this.openDialogNoModal.add(this.onOpenDialogNoModal);
        this.closeDialog.add(this.onCloseDialog);
        this.pushDialogSignal.add(this.onPushDialog);
        this.popDialogSignal.add(this.onPopDialog);
        this.addToQueueSignal.add(this.onAddToQueue);
        this.flushStartupQueue.add(this.onFlushQueue);
    }

    private function onFlushQueue() : void
    {
        var _loc1_:PopupQueueEntry = this.dialogsModel.flushStartupQueue();
        if(_loc1_ != null)
        {
            if(_loc1_.paramObject)
            {
                _loc1_.signal.dispatch(_loc1_.paramObject);
            }
            else
            {
                _loc1_.signal.dispatch();
            }
        }
    }

    private function onAddToQueue(param1:String, param2:Signal, param3:int, param4:Object) : void
    {
        this.dialogsModel.addPopupToStartupQueue(param1,param2,param3,param4);
    }

    private function onPushDialog(param1:Sprite) : void
    {
        this.view.push(param1);
    }

    private function onPopDialog() : void
    {
        this.view.pop();
    }

    override public function destroy() : void
    {
        this.showDialogBackground.remove(this.onShowDialogBackground);
        this.openDialog.remove(this.onOpenDialog);
        this.openDialogNoModal.remove(this.onOpenDialogNoModal);
        this.closeDialog.remove(this.onCloseDialog);
        this.pushDialogSignal.remove(this.onPushDialog);
        this.popDialogSignal.remove(this.onPopDialog);
        this.addToQueueSignal.remove(this.onAddToQueue);
        this.flushStartupQueue.remove(this.onFlushQueue);
    }

    private function onShowDialogBackground(param1:int = 1381653) : void
    {
        this.view.showBackground(param1);
    }

    private function onOpenDialog(param1:Sprite) : void
    {
        this.view.show(param1,true);
    }

    private function onOpenDialogNoModal(param1:Sprite) : void
    {
        this.view.show(param1,false);
    }

    private function onCloseDialog() : void
    {
        this.view.stage.focus = null;
        this.view.hideAll();
    }
}
}

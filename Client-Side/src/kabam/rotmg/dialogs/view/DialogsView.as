package kabam.rotmg.dialogs.view
{
import flash.display.DisplayObject;
import flash.display.DisplayObjectContainer;
import flash.display.Graphics;
import flash.display.Sprite;
import flash.events.Event;

public class DialogsView extends Sprite
{


    private var background:Sprite;

    private var container:DisplayObjectContainer;

    private var current:Sprite;

    private var pushed:DisplayObject;

    public function DialogsView()
    {
        super();
        var _loc1_:* = new Sprite();
        this.background = _loc1_;
        addChild(_loc1_);
        _loc1_ = new Sprite();
        this.container = _loc1_;
        addChild(_loc1_);
        this.background.visible = false;
        this.background.mouseEnabled = true;
    }

    public function showBackground(_arg1:int = 0x151515):void {
        var _local2:Graphics = this.background.graphics;
        _local2.clear();
        _local2.beginFill(_arg1, 0.6);
        _local2.drawRect(0, 0, 800, 600);
        _local2.endFill();
        this.background.visible = true;
    }

    public function show(param1:Sprite, param2:Boolean) : void
    {
        this.removeCurrentDialog();
        this.addDialog(param1);
        ((param2) && (this.showBackground()));
    }

    public function hideAll() : void
    {
        this.background.visible = false;
        this.removeCurrentDialog();
    }

    public function push(param1:Sprite) : void
    {
        this.current.visible = false;
        this.pushed = param1;
        addChild(param1);
        this.background.visible = true;
    }

    public function getPushed() : DisplayObject
    {
        return this.pushed;
    }

    public function pop() : void
    {
        removeChild(this.pushed);
        this.current.visible = true;
    }

    private function addDialog(param1:Sprite) : void
    {
        this.current = param1;
        param1.addEventListener("removed",this.onRemoved);
        this.container.addChild(param1);
    }

    private function onRemoved(param1:Event) : void
    {
        var _loc2_:Sprite = param1.target as Sprite;
        if(this.current == _loc2_)
        {
            this.background.visible = false;
            this.current = null;
        }
    }

    private function removeCurrentDialog() : void
    {
        if(this.current && this.container.contains(this.current))
        {
            this.current.removeEventListener("removed",this.onRemoved);
            this.container.removeChild(this.current);
            this.background.visible = false;
        }
    }
}
}

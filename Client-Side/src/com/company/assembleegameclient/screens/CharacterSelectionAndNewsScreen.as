package com.company.assembleegameclient.screens
{
import com.company.assembleegameclient.ui.DeprecatedClickableText;
import com.company.assembleegameclient.ui.Scrollbar;
import flash.display.DisplayObject;
import flash.display.Shape;
import flash.display.Sprite;
import flash.events.Event;
import flash.events.MouseEvent;
import flash.filters.DropShadowFilter;
import flash.geom.Rectangle;
import flash.text.TextFieldAutoSize;
import kabam.rotmg.core.model.PlayerModel;
import kabam.rotmg.game.view.CreditDisplay;
import kabam.rotmg.text.model.TextKey;
import kabam.rotmg.text.view.TextFieldDisplayConcrete;
import kabam.rotmg.text.view.stringBuilder.LineBuilder;
import kabam.rotmg.text.view.stringBuilder.StaticStringBuilder;
import kabam.rotmg.ui.view.ButtonFactory;
import kabam.rotmg.ui.view.components.MenuOptionsBar;
import kabam.rotmg.ui.view.components.ScreenBase;
import org.osflash.signals.Signal;

public class CharacterSelectionAndNewsScreen extends Sprite
{

    private static const TAB_UNSELECTED:uint = 11776947;

    private static const TAB_SELECTED:uint = 16777215;


    private const SCROLLBAR_REQUIREMENT_HEIGHT:Number = 400;

    private const CHARACTER_LIST_Y_POS:int = 108;

    private const CHARACTER_LIST_X_POS:int = 70;

    private const DROP_SHADOW:DropShadowFilter = new DropShadowFilter(0,0,0,1,8,8);

    public var close:Signal;

    public var showClasses:Signal;

    public var newCharacter:Signal;

    public var chooseName:Signal;

    public var playGame:Signal;

    private var model:PlayerModel;

    private var isInitialized:Boolean;

    private var nameText:TextFieldDisplayConcrete;

    private var nameChooseLink_:DeprecatedClickableText;

    private var creditDisplay:CreditDisplay;

    private var openCharactersText:TextFieldDisplayConcrete;

    private var openGraveyardText:TextFieldDisplayConcrete;

    private var characterList:CharacterList;

    private var characterListType:int = 1;

    private var characterListHeight:Number;

    private var lines:Shape;


    private var scrollBar:Scrollbar;


    private var playButton:TitleMenuOption;

    private var classesButton:TitleMenuOption;

    private var backButton:TitleMenuOption;

    private var menuOptionsBar:MenuOptionsBar;

    private var BOUNDARY_LINE_ONE_Y:int = 106;

    public function CharacterSelectionAndNewsScreen()
    {
        this.newCharacter = new Signal();
        this.chooseName = new Signal();
        this.playGame = new Signal();
        this.playButton = ButtonFactory.getPlayButton();
        this.classesButton = ButtonFactory.getClassesButton();
        this.backButton = ButtonFactory.getMainButton();
        super();
        this.close = this.backButton.clicked;
        this.showClasses = this.classesButton.clicked;
        addChild(new ScreenBase());
        addChild(new AccountScreen());
    }

    public function initialize(param1:PlayerModel) : void
    {
        if(this.isInitialized)
        {
            return;
        }
        this.isInitialized = true;
        this.model = param1;
        this.createDisplayAssets(param1);
    }

    public function getReferenceRectangle() : Rectangle
    {
        var _loc1_:Rectangle = new Rectangle();
        if(stage)
        {
            _loc1_ = new Rectangle(0,0,800,600);
        }
        return _loc1_;
    }

    public function setName(param1:String) : void
    {
        this.nameText.setStringBuilder(new StaticStringBuilder(this.model.getName()));
        this.nameText.x = (this.getReferenceRectangle().width - this.nameText.width) * 0.5;
        if(this.nameChooseLink_)
        {
            removeChild(this.nameChooseLink_);
            this.nameChooseLink_ = null;
        }
    }

    private function createDisplayAssets(param1:PlayerModel) : void
    {
        this.createNameText();
        this.createBoundaryLines();
        this.createOpenCharactersText();
        var _loc2_:Graveyard = new Graveyard(param1);
        if(_loc2_.hasCharacters())
        {
            this.openCharactersText.setColor(TAB_SELECTED);
            this.createOpenGraveyardText();
        }
        this.createCharacterListChar();
        this.makeMenuOptionsBar();
        if(!param1.isNameChosen())
        {
            this.createChooseNameLink();
        }
    }

    private function onOpenGraveyard(param1:MouseEvent) : void
    {
        if(this.characterListType != CharacterList.TYPE_GRAVE_SELECT)
        {
            this.removeCharacterList();
            this.openCharactersText.setColor(TAB_UNSELECTED);
            this.openGraveyardText.setColor(TAB_SELECTED);
            this.createCharacterListGrave();
        }
    }

    private function createOpenGraveyardText() : void
    {
        this.openGraveyardText = new TextFieldDisplayConcrete().setSize(18).setColor(TAB_UNSELECTED);
        this.openGraveyardText.setBold(true);
        this.openGraveyardText.setStringBuilder(new LineBuilder().setParams(TextKey.CHARACTER_SELECTION_GRAVEYARD));
        this.openGraveyardText.filters = [this.DROP_SHADOW];
        this.openGraveyardText.x = this.CHARACTER_LIST_X_POS + 150;
        this.openGraveyardText.y = 79;
        this.openGraveyardText.addEventListener(MouseEvent.CLICK,this.onOpenGraveyard);
        addChild(this.openGraveyardText);
    }

    private function createCharacterListGrave() : void
    {
        this.characterListType = CharacterList.TYPE_GRAVE_SELECT;
        this.characterList = new CharacterList(this.model,CharacterList.TYPE_GRAVE_SELECT);
        this.characterList.x = this.CHARACTER_LIST_X_POS;
        this.characterList.y = this.CHARACTER_LIST_Y_POS;
        this.characterListHeight = this.characterList.height;
        if(this.characterListHeight > this.SCROLLBAR_REQUIREMENT_HEIGHT)
        {
            this.createScrollbar();
        }
        addChild(this.characterList);
    }



    private function makeMenuOptionsBar() : void
    {
        this.playButton.clicked.add(this.onPlayClick);
        this.menuOptionsBar = new MenuOptionsBar();
        this.menuOptionsBar.addButton(this.playButton,MenuOptionsBar.CENTER);
        this.menuOptionsBar.addButton(this.backButton,MenuOptionsBar.LEFT);
        this.menuOptionsBar.addButton(this.classesButton,MenuOptionsBar.RIGHT);
        addChild(this.menuOptionsBar);
    }

    private function createScrollbar() : void
    {
        this.scrollBar = new Scrollbar(16,399);
        this.scrollBar.x = 758;
        this.scrollBar.y = 113;
        this.scrollBar.setIndicatorSize(399,this.characterList.height);
        this.scrollBar.addEventListener(Event.CHANGE,this.onScrollBarChange);
        addChild(this.scrollBar);
    }

    private function createCharacterListChar() : void
    {
        this.characterListType = CharacterList.TYPE_CHAR_SELECT;
        this.characterList = new CharacterList(this.model,CharacterList.TYPE_CHAR_SELECT);
        this.characterList.x = this.CHARACTER_LIST_X_POS;
        this.characterList.y = this.CHARACTER_LIST_Y_POS;
        this.characterListHeight = this.characterList.height;
        if(this.characterListHeight > this.SCROLLBAR_REQUIREMENT_HEIGHT)
        {
            this.createScrollbar();
        }
        addChild(this.characterList);
    }

    private function removeCharacterList() : void
    {
        if(this.characterList != null)
        {
            removeChild(this.characterList);
            this.characterList = null;
        }
        if(this.scrollBar != null)
        {
            removeChild(this.scrollBar);
            this.scrollBar = null;
        }
    }

    private function createOpenCharactersText() : void
    {
        this.openCharactersText = new TextFieldDisplayConcrete().setSize(18).setColor(TAB_UNSELECTED);
        this.openCharactersText.setBold(true);
        this.openCharactersText.setStringBuilder(new LineBuilder().setParams(TextKey.CHARACTER_SELECTION_CHARACTERS));
        this.openCharactersText.filters = [this.DROP_SHADOW];
        this.openCharactersText.x = this.CHARACTER_LIST_X_POS;
        this.openCharactersText.y = 79;
        this.openCharactersText.addEventListener(MouseEvent.CLICK,this.onOpenCharacters);
        addChild(this.openCharactersText);
    }

    private function createChooseNameLink() : void
    {
        this.nameChooseLink_ = new DeprecatedClickableText(16,false,TextKey.CHARACTER_SELECTION_AND_NEWS_SCREEN_CHOOSE_NAME);
        this.nameChooseLink_.y = 50;
        this.nameChooseLink_.setAutoSize(TextFieldAutoSize.CENTER);
        this.nameChooseLink_.x = this.getReferenceRectangle().width * 0.5;
        this.nameChooseLink_.addEventListener(MouseEvent.CLICK,this.onChooseName);
        addChild(this.nameChooseLink_);
    }

    private function createNameText() : void
    {
        this.nameText = new TextFieldDisplayConcrete().setSize(22).setColor(11776947);
        this.nameText.setBold(true).setAutoSize(TextFieldAutoSize.CENTER);
        this.nameText.setStringBuilder(new StaticStringBuilder(this.model.getName()));
        this.nameText.filters = [this.DROP_SHADOW];
        this.nameText.y = 24;
        this.nameText.x = (this.getReferenceRectangle().width - this.nameText.width) * 0.5;
        addChild(this.nameText);
    }

    private function createBoundaryLines() : void
    {
        this.lines = new Shape();
        this.lines.graphics.clear();
        this.lines.graphics.lineStyle(2,5526612);
        this.lines.graphics.moveTo(0,this.BOUNDARY_LINE_ONE_Y);
        this.lines.graphics.lineTo(this.getReferenceRectangle().width,this.BOUNDARY_LINE_ONE_Y);
        this.lines.graphics.lineStyle();
        addChild(this.lines);
    }

    private function removeIfAble(param1:DisplayObject) : void
    {
        if(param1 && contains(param1))
        {
            removeChild(param1);
        }
    }

    private function onPlayClick() : void
    {
        if(this.model.getCharacterCount() == 0)
        {
            this.newCharacter.dispatch();
        }
        else
        {
            this.playGame.dispatch();
        }
    }

    private function onOpenCharacters(param1:MouseEvent) : void
    {
        if(this.characterListType != CharacterList.TYPE_CHAR_SELECT)
        {
            this.removeCharacterList();
            this.openCharactersText.setColor(TAB_SELECTED);
            this.openGraveyardText.setColor(TAB_UNSELECTED);
            this.createCharacterListChar();
        }
    }


    private function onChooseName(param1:MouseEvent) : void
    {
        this.chooseName.dispatch();
    }

    private function onScrollBarChange(param1:Event) : void
    {
        if(this.characterList != null)
        {
            this.characterList.setPos(-this.scrollBar.pos() * (this.characterListHeight - 400));
        }
    }
}
}

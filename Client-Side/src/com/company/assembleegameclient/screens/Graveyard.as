package com.company.assembleegameclient.screens {
import com.company.assembleegameclient.appengine.SavedNewsItem;

import flash.display.Sprite;

import kabam.rotmg.core.model.PlayerModel;
import kabam.rotmg.death.model.DeathModel;
import kabam.rotmg.fame.model.FameModel;
import kabam.rotmg.fame.service.RequestCharacterFameTask;

public class Graveyard extends Sprite {
    private var lines_:Vector.<GraveyardLine>;
    private var hasCharacters_:Boolean = false;
    private var arg1xTut:int = 0;
    private var arg1yTut:int = 4;


    public function Graveyard(_arg1:PlayerModel) {
        var _local2:SavedNewsItem;
        this.lines_ = new Vector.<GraveyardLine>();
        super();
        for each (_local2 in _arg1.getNews()) {
            if (_local2.isCharDeath()) {
                this.addLine(new GraveyardLine(_local2.getIcon(), _local2.title_, _local2.tagline_, _local2.link_, _local2.date_, _arg1.getAccountId()));
                this.hasCharacters_ = true;
            }
        }
    }

    public function hasCharacters():Boolean {
        return (this.hasCharacters_);
    }

    public function addLine(_arg1:GraveyardLine):void {
        if(arg1xTut > 680)
        {
            arg1xTut = 0;
            arg1yTut = arg1yTut + 4 + GraveyardLine.HEIGHT;
        }
        _arg1.x = arg1xTut;
        _arg1.y = arg1yTut;
        arg1xTut = 4 + GraveyardLine.WIDTH + arg1xTut;
        this.lines_.push(_arg1);
        addChild(_arg1);
    }


}
}

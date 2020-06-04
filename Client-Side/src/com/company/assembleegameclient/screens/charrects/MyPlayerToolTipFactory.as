package com.company.assembleegameclient.screens.charrects
{
import com.company.assembleegameclient.appengine.CharacterStats;
import com.company.assembleegameclient.ui.tooltip.MyPlayerToolTip;

public class MyPlayerToolTipFactory
{


    public function MyPlayerToolTipFactory()
    {
        super();
    }

    public function create(param1:String, param2:XML, param3:CharacterStats) : MyPlayerToolTip
    {
        return new MyPlayerToolTip(param1,param2,param3);
    }
}
}

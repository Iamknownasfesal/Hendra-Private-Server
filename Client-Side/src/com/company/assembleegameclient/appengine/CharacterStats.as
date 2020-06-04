package com.company.assembleegameclient.appengine
{
import com.company.assembleegameclient.util.FameUtil;

public class CharacterStats
{


    public var charStatsXML_:XML;

    public function CharacterStats(param1:XML)
    {
        super();
        this.charStatsXML_ = param1;
    }

    public function bestLevel() : int
    {
        return this.charStatsXML_.BestLevel;
    }

    public function bestFame() : int
    {
        return this.charStatsXML_.BestFame;
    }

    public function numStars() : int
    {
        return FameUtil.numStars(int(this.charStatsXML_.BestFame));
    }
}
}

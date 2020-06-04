﻿package com.company.assembleegameclient.util {
public class Currency {

    public static const INVALID:int = -1;
    public static const GOLD:int = 0;
    public static const FAME:int = 1;
    public static const GUILD_FAME:int = 2;
    public static const PRESTIGE:int = 4;


    public static function typeToName(_arg1:int):String {
        switch (_arg1) {
            case GOLD:
                return ("Gold");
            case FAME:
                return ("Fame");
            case GUILD_FAME:
                return ("Guild Fame");
            case PRESTIGE:
                return ("Prestige");
        }
        return ("");
    }


}
}

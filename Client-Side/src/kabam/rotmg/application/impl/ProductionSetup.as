﻿package kabam.rotmg.application.impl {
import com.company.assembleegameclient.parameters.Parameters;

import kabam.rotmg.application.api.ApplicationSetup;

public class ProductionSetup implements ApplicationSetup {

    private const SERVER:String = "37.123.96.189:8888";
    private const UNENCRYPTED:String = ("http://" + SERVER);
    private const ENCRYPTED:String = ("http://" + SERVER);
    private const BUILD_LABEL:String = "{VERSION} v{MINOR}";


    public function getAppEngineUrl(_arg1:Boolean = false):String {
        return (((_arg1) ? this.UNENCRYPTED : this.ENCRYPTED));
    }

    public function getBuildLabel():String {
        return (this.BUILD_LABEL.replace("{VERSION}", Parameters.BUILD_VERSION).replace("{MINOR}", Parameters.MINOR_VERSION));
    }

    public function useLocalTextures():Boolean {
        return (false);
    }

    public function isToolingEnabled():Boolean {
        return (false);
    }

    public function isGameLoopMonitored():Boolean {
        return (false);
    }

    public function useProductionDialogs():Boolean {
        return (true);
    }

    public function areErrorsReported():Boolean {
        return (false);
    }

    public function areDeveloperHotkeysEnabled():Boolean {
        return (false);
    }

    public function isDebug():Boolean {
        return (false);
    }


}
}

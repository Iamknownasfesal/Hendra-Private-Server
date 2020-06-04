﻿package kabam.rotmg.application {
import flash.display.DisplayObjectContainer;
import flash.display.LoaderInfo;

import kabam.rotmg.application.api.ApplicationSetup;
import kabam.rotmg.application.api.DebugSetup;
import kabam.rotmg.application.impl.FixedIPSetup;
import kabam.rotmg.application.impl.LocalhostSetup;
import kabam.rotmg.application.impl.NillysRealmSetup;
import kabam.rotmg.application.impl.NillysRealmTestSetup;
import kabam.rotmg.application.impl.PrivateSetup;
import kabam.rotmg.application.impl.ProductionSetup;
import kabam.rotmg.application.impl.Testing2Setup;
import kabam.rotmg.application.impl.TestingSetup;
import kabam.rotmg.application.model.DomainModel;
import kabam.rotmg.application.model.PlatformModel;
import kabam.rotmg.build.api.BuildData;
import kabam.rotmg.build.api.BuildEnvironment;

import org.swiftsuspenders.Injector;

import robotlegs.bender.framework.api.IConfig;

public class ApplicationConfig implements IConfig {

    [Inject]
    public var injector:Injector;
    [Inject]
    public var root:DisplayObjectContainer;
    [Inject]
    public var data:BuildData;
    [Inject]
    public var loaderInfo:LoaderInfo;
    [Inject]
    public var domainModel:DomainModel;


    public function configure():void {
        var _local1:ApplicationSetup = this.makeTestingSetup();
        this.injector.map(DebugSetup).toValue(_local1);
        this.injector.map(ApplicationSetup).toValue(_local1);
        this.injector.map(PlatformModel).asSingleton();
    }

    private function makeTestingSetup():ApplicationSetup {
        return new ProductionSetup();
    }

    private function makeFixedIPSetup():FixedIPSetup {
        return (new FixedIPSetup().setAddress(this.data.getEnvironmentString()));
    }


}
}

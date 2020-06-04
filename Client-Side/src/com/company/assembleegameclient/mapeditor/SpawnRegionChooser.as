package com.company.assembleegameclient.mapeditor {
public class SpawnRegionChooser extends Chooser {

    public function SpawnRegionChooser() {
        var _local1:XML;
        var _local2:SpawnRegionElement;
        super(Layer.SPAWNREGION);
        for each (_local1 in GroupDivider.GROUPS["SpawnRegions"]) {
            _local2 = new SpawnRegionElement(_local1);
            addElement(_local2);
        }
    }

}
}

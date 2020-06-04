package kabam.rotmg.characterMetrics.data {
import flash.utils.Dictionary;

public class CharacterMetricsData
{


    private var stats:Dictionary;

    public function CharacterMetricsData()
    {
        super();
        this.stats = new Dictionary();
    }

    public function setStat(param1:int, param2:int) : void
    {
        this.stats[param1] = param2;
    }

    public function getStat(param1:int) : int
    {
        if(!this.stats[param1])
        {
            return 0;
        }
        return this.stats[param1];
    }
}
}

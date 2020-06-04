using common.resources;
using wServer.networking;

namespace wServer.realm.worlds.logic
{
    public class DonorShop : World
    {
        public DonorShop(ProtoWorld proto, Client client = null) : base(proto)
        {
        }

        protected override void Init()
        {

            base.Init();

            Manager.Market.InitMarketplace(this);
        }
    }
}

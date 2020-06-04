using wServer.realm.worlds;

namespace wServer.realm.setpieces
{
    class LordOfSky : ISetPiece
    {
        public int Size { get { return 5; } }

        public void RenderSetPiece(World world, IntPoint pos)
        {
            var proto = world.Manager.Resources.Worlds["LordOfSky"];
            SetPieces.RenderFromProto(world, pos, proto);
        }
    }
}

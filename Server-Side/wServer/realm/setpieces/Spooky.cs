using wServer.realm.worlds;

namespace wServer.realm.setpieces
{
    class Spooky : ISetPiece
    {
        public int Size { get { return 5; } }

        public void RenderSetPiece(World world, IntPoint pos)
        {
            var Spooky = Entity.Resolve(world.Manager, "LH Sentry");
            Spooky.Move(pos.X + 2.5f, pos.Y + 2.5f);
            world.EnterWorld(Spooky);
        }
    }
}

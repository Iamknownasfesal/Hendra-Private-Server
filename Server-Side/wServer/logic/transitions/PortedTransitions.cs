namespace wServer.logic.transitions
{
    /// <summary>
    /// The imported scripts' spelling of <see cref="EntityNotExistsTransition"/>.
    /// </summary>
    /// <remarks>
    /// Same arguments, same meaning, one letter apart. Subclassed rather than copied so there is
    /// only ever one implementation of the test.
    /// </remarks>
    internal class EntityNotExistTransition : EntityNotExistsTransition
    {
        public EntityNotExistTransition(string target, double dist, string targetState,
            bool checkAttackTarget = false)
            : base(target, dist, targetState, checkAttackTarget)
        {
        }
    }
}

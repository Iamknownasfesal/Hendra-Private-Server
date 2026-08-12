namespace wServer.logic.loot
{
    /// <summary>
    /// Drop bundles the imported enemy scripts reach for by name.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The scripts in <c>logic/db</c> were written against a fork with a richer loot pipeline than
    /// this one. Two of its ideas do not survive the crossing and are approximated here rather than
    /// faked.
    /// </para>
    /// <para>
    /// The first is <c>OnlyOne</c>, which picks a single entry out of a bundle. This server decides
    /// every drop independently, so the six stat potions are each given a sixth of the chance
    /// instead: the same expected number of potions, arrived at by a different route, and very
    /// occasionally two at once.
    /// </para>
    /// <para>
    /// The second is the fork's own currency, which this build has no item for. Rather than name an
    /// item that does not exist -- which would throw when the behaviour database is built, taking
    /// every enemy in the file with it -- those bundles are empty and drop nothing.
    /// </para>
    /// </remarks>
    public static class LootTemplates
    {
        /// <summary>One stat potion, near enough.</summary>
        public static MobDrops[] StatPots()
        {
            const double each = 1.0 / 6.0;

            return new MobDrops[]
            {
                new ItemLoot("Potion of Defense", each),
                new ItemLoot("Potion of Attack", each),
                new ItemLoot("Potion of Speed", each),
                new ItemLoot("Potion of Vitality", each),
                new ItemLoot("Potion of Wisdom", each),
                new ItemLoot("Potion of Dexterity", each),
            };
        }

        /// <summary>
        /// The imported fork's own currency, which this build has no item for.
        /// </summary>
        /// <remarks>
        /// Kept as named, empty bundles so the scripts calling them still compile and read the way
        /// they were written. Give this build an equivalent item and they become one line each.
        /// </remarks>
        public static MobDrops[] SorRare() => new MobDrops[0];

        public static MobDrops[] SorUncommon() => new MobDrops[0];

        public static MobDrops[] SorCommon() => new MobDrops[0];

        public static MobDrops[] Sor1Perc() => new MobDrops[0];

        public static MobDrops[] Sor2Perc() => new MobDrops[0];

        public static MobDrops[] Sor3Perc() => new MobDrops[0];

        public static MobDrops[] Sor4Perc() => new MobDrops[0];

        public static MobDrops[] Sor5Perc() => new MobDrops[0];

        /// <summary>Raid entry tokens, which are the imported fork's own content.</summary>
        public static MobDrops[] RaidTokens() => new MobDrops[0];
    }
}

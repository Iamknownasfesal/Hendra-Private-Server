using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using common;
using common.resources;
using wServer.networking;
using wServer.networking.packets.incoming;
using wServer.networking.packets.outgoing;
using wServer.realm.entities;

namespace wServer.realm
{
    /// <summary>
    /// One account's vault: every chest it owns, held as one thing.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The chests used to be entities standing in a room, one per eight slots, and moving an item
    /// meant naming the entity it belonged to. They are rows in a panel now, so they are held here
    /// instead -- as containers with no body, which is all the swap machinery ever needed of them:
    /// <see cref="IContainer"/> asks for slot types, an inventory and somewhere to persist, and
    /// none of those has anything to do with standing in a world.
    /// </para>
    /// <para>
    /// One of these per account, not per session. Two clients logged into one account is an ordinary
    /// state, and if each had its own copy of the vault they would each apply their own moves to
    /// their own idea of the contents -- which is how one stack becomes two. Everything below runs
    /// under this object's lock, and every accepted change bumps <see cref="Version"/> and is told
    /// to every session at once.
    /// </para>
    /// <para>
    /// What is persisted is unchanged and deliberately so: one Redis field per chest,
    /// <c>vault.&lt;n&gt;</c> under <c>vault.&lt;accountId&gt;</c>, holding that chest's eight item
    /// types, exactly as before the chests stopped being entities. The flat numbering the panel
    /// draws in is worked out on the way past and stored nowhere.
    /// </para>
    /// </remarks>
    public sealed class VaultState
    {
        public const int SlotsPerChest = 8;

        /// <summary>The vaults in play, by account. See <see cref="Release"/> for how they leave.</summary>
        private static readonly ConcurrentDictionary<int, VaultState> Live =
            new ConcurrentDictionary<int, VaultState>();

        /// <summary>A chest: eight slots and somewhere to save them, and nothing else at all.</summary>
        private sealed class Chest : IContainer
        {
            public Chest(DbAccount account, int index)
            {
                var link = new DbVaultSingle(account, index);

                DbLink = link;
                SlotTypes = new int[SlotsPerChest];
                Inventory = new Inventory(this, new Item[SlotsPerChest]);
                Inventory.SetItems(Eight(link.Items));

                Inventory.InventoryChanged += (sender, e) =>
                {
                    link.Items = Inventory.GetItemTypes();
                    link.FlushAsync();
                };
            }

            /// <summary>
            /// The eight slots a chest has, out of however many are stored.
            /// </summary>
            /// <remarks>
            /// What is on disk is usually twenty-four long: the chests were entities, entities get an
            /// inventory the size of a player's, and the last sixteen have always been empty. A chest
            /// is eight slots and is read as eight, and saving writes eight back -- which the old
            /// code would also have accepted, since it only ever required the array to fit.
            /// </remarks>
            private static ushort[] Eight(ushort[] stored)
            {
                var items = Enumerable.Repeat((ushort)0xffff, SlotsPerChest).ToArray();

                if (stored != null)
                    for (var i = 0; i < SlotsPerChest && i < stored.Length; i++)
                        items[i] = stored[i];

                return items;
            }

            public int[] SlotTypes { get; }
            public Inventory Inventory { get; }
            public RInventory DbLink { get; }
        }

        private readonly RealmManager _manager;
        private readonly int _accountId;
        private readonly DbAccount _account;
        private readonly List<Chest> _chests = new List<Chest>();
        private readonly object _lock = new object();

        private VaultState(RealmManager manager, DbAccount account)
        {
            _manager = manager;
            _accountId = account.AccountId;
            _account = account;

            Grant(account);

            for (var i = 0; i < account.VaultCount; i++)
                _chests.Add(new Chest(account, i));
        }

        /// <summary>
        /// Gives an account the chests everybody gets, if it does not have them yet.
        /// </summary>
        /// <remarks>
        /// Written to the database rather than faked in the count, so a granted chest is the same
        /// object as a bought one -- it persists, it is addressed the same way, and nothing later
        /// has to know which kind it was. Raising the setting hands the difference to every account
        /// the next time it opens its vault; lowering it takes nothing away, because a chest that
        /// has been given may have something in it.
        /// </remarks>
        private void Grant(DbAccount account)
        {
            var free = _manager.Resources.Settings.FreeVaultChests;
            if (account.VaultCount >= free)
                return;

            for (var i = account.VaultCount; i < free; i++)
                _manager.Database.CreateChest(account);

            account.Reload("vaultCount");
        }

        /// <summary>What the vault was at when it was last read out. Moves quote it back.</summary>
        public int Version { get; private set; }

        public int ChestCount
        {
            get { lock (_lock) return _chests.Count; }
        }

        /// <summary>
        /// This account's vault, loading it if this is the first session to ask.
        /// </summary>
        public static VaultState Of(RealmManager manager, DbAccount account)
        {
            return Live.GetOrAdd(account.AccountId, _ => new VaultState(manager, account));
        }

        /// <summary>
        /// Forgets an account's vault once nobody is logged into it.
        /// </summary>
        /// <remarks>
        /// Called as clients leave. The check is for another session on the same account rather than
        /// for none at all, because the whole point of holding one of these is that two sessions
        /// share it -- dropping it while the second is still using it would hand that session a
        /// fresh copy and undo the sharing.
        /// </remarks>
        public static void Release(RealmManager manager, int accountId)
        {
            if (manager.Clients.Keys.Any(c => c.Account != null && c.Account.AccountId == accountId))
                return;

            VaultState gone;
            Live.TryRemove(accountId, out gone);
        }

        /// <summary>The whole vault, flat, in the order the panel draws it.</summary>
        public VaultUpdate Snapshot()
        {
            lock (_lock)
            {
                var slots = new ushort[_chests.Count * SlotsPerChest];

                for (var c = 0; c < _chests.Count; c++)
                {
                    var items = _chests[c].Inventory.GetItemTypes();
                    for (var s = 0; s < SlotsPerChest; s++)
                        slots[c * SlotsPerChest + s] = items[s];
                }

                return new VaultUpdate
                {
                    Version = Version,
                    ChestCount = _chests.Count,
                    MaxChests = _manager.Resources.Settings.MaxVaultChests,
                    NextChestPrice = _manager.Resources.Settings.VaultChestCost,
                    Slots = slots,
                    Gifts = _account.Gifts ?? new ushort[0]
                };
            }
        }

        /// <summary>Tells every session on this account what the vault now is.</summary>
        public void Broadcast()
        {
            var update = Snapshot();

            foreach (var client in _manager.Clients.Keys)
                if (client.Account != null && client.Account.AccountId == _accountId)
                    client.SendPacket(update);
        }

        /// <summary>
        /// Moves an item, either within the vault or between it and the player's own inventory.
        /// </summary>
        /// <remarks>
        /// <para>
        /// A swap, always: whatever was at the destination comes back the other way, so nothing is
        /// ever removed from one place and added to another as two steps. The two steps are the
        /// thing that duplicates items when the second one fails, and there is no version of this
        /// method that does them.
        /// </para>
        /// <para>
        /// The commit is <see cref="Inventory.Execute(InventoryTransaction[])"/>, which is the
        /// server's existing primitive: it takes the transactions, checks that neither container has
        /// changed since they were opened, and applies them together or not at all.
        /// </para>
        /// </remarks>
        /// <returns>Whether the move was applied. A false is a refusal, not a failure.</returns>
        public bool TryMove(Player player, VaultMove move, out string refusal)
        {
            refusal = null;

            if (player?.Inventory == null)
            {
                refusal = "no player";
                return false;
            }

            lock (_lock)
            {
                // Stale by version: the client computed this move against a vault that has since
                // changed. Refusing and re-sending is the whole of the concurrency story -- the
                // loser is told what the vault actually is and can ask again.
                if (move.Version != Version)
                {
                    refusal = "stale";
                    return false;
                }

                // A gift is claimed rather than swapped: there is nothing to send back the other
                // way, and the destination slot has to be empty for that reason.
                if (move.FromChest == VaultMove.Gifts)
                    return TryClaimGift(player, move, out refusal);

                if (move.ToChest == VaultMove.Gifts)
                {
                    refusal = "gifts are one way";
                    return false;
                }

                IContainer from, to;
                if (!Resolve(player, move.FromChest, move.FromSlot, out from) ||
                    !Resolve(player, move.ToChest, move.ToSlot, out to))
                {
                    refusal = "out of range";
                    return false;
                }

                // The backpack is only addressable if the character has one, exactly as InvSwap has
                // it. Without this a client could reach eight slots it has not bought.
                if ((move.FromSlot >= 16 || move.ToSlot >= 16) &&
                    (from == player || to == player) && !player.HasBackpack)
                {
                    refusal = "no backpack";
                    return false;
                }

                var itemFrom = from.Inventory[move.FromSlot];
                var itemTo = to.Inventory[move.ToSlot];

                // Nothing to do, and worth saying so before opening a transaction for it.
                if (itemFrom == null && itemTo == null)
                    return true;

                // Slot types, which only ever say anything on the player's side: a vault chest takes
                // anything, and a ring does not go in a weapon slot.
                if (!to.AuditItem(itemFrom, move.ToSlot) || !from.AuditItem(itemTo, move.FromSlot))
                {
                    refusal = "wrong slot";
                    return false;
                }

                bool applied = ReferenceEquals(from, to)
                    ? Swap(from, move.FromSlot, move.ToSlot)
                    : Swap(from, move.FromSlot, to, move.ToSlot);

                if (!applied)
                {
                    refusal = "contention";
                    return false;
                }

                Version++;
                return true;
            }
        }

        /// <summary>
        /// Claims a gift into the player's inventory.
        /// </summary>
        /// <remarks>
        /// The gift leaves the account's list and the item appears in the slot together: the write
        /// to Redis is what makes the claim real, so it goes first, and the inventory is only
        /// touched once it has gone through. A failure here leaves the gift unclaimed, which is the
        /// safe direction to fail in.
        /// </remarks>
        private bool TryClaimGift(Player player, VaultMove move, out string refusal)
        {
            refusal = null;

            var gifts = _account.Gifts ?? new ushort[0];
            if (move.FromSlot < 0 || move.FromSlot >= gifts.Length)
            {
                refusal = "no such gift";
                return false;
            }

            if (move.ToChest != VaultMove.Player ||
                move.ToSlot < 0 || move.ToSlot >= player.Inventory.Length)
            {
                refusal = "gifts go to the inventory";
                return false;
            }

            if (player.Inventory[move.ToSlot] != null)
            {
                refusal = "slot taken";
                return false;
            }

            var type = gifts[move.FromSlot];
            var item = _manager.Resources.GameData.Items[type];

            if (!((IContainer)player).AuditItem(item, move.ToSlot))
            {
                refusal = "wrong slot";
                return false;
            }

            if (!_manager.Database.RemoveGift(_account, type))
            {
                refusal = "could not claim";
                return false;
            }

            var trans = player.Inventory.CreateTransaction();
            trans[move.ToSlot] = item;

            if (!Inventory.Execute(trans))
            {
                // The gift is already gone from the account, so putting it back is the only honest
                // thing to do -- an item that vanished between two writes is the failure this whole
                // design is arranged to avoid.
                _manager.Database.AddGift(_account, type);
                refusal = "contention";
                return false;
            }

            Version++;
            return true;
        }

        /// <summary>A swap inside one container, which needs one transaction rather than two.</summary>
        private static bool Swap(IContainer container, int a, int b)
        {
            var trans = container.Inventory.CreateTransaction();

            var held = trans[a];
            trans[a] = trans[b];
            trans[b] = held;

            return Inventory.Execute(trans);
        }

        private static bool Swap(IContainer from, int fromSlot, IContainer to, int toSlot)
        {
            var fromTrans = from.Inventory.CreateTransaction();
            var toTrans = to.Inventory.CreateTransaction();

            var held = fromTrans[fromSlot];
            fromTrans[fromSlot] = toTrans[toSlot];
            toTrans[toSlot] = held;

            return Inventory.Execute(fromTrans, toTrans);
        }

        /// <summary>Turns one end of a move into the container it names, or refuses it.</summary>
        private bool Resolve(Player player, short chest, short slot, out IContainer container)
        {
            container = null;

            if (chest == VaultMove.Player)
            {
                if (slot < 0 || slot >= player.Inventory.Length)
                    return false;

                container = player;
                return true;
            }

            if (chest < 0 || chest >= _chests.Count || slot < 0 || slot >= SlotsPerChest)
                return false;

            container = _chests[chest];
            return true;
        }

        /// <summary>
        /// Buys one more chest.
        /// </summary>
        /// <remarks>
        /// The count the client sent is checked against the count the server has, so a second click
        /// arriving while the first is still being paid for buys nothing. The price and the purse
        /// are read here and never sent by the client.
        /// </remarks>
        public bool TryBuy(Client client, int believedCount, out string refusal)
        {
            refusal = null;

            var account = client?.Account;
            if (account == null)
            {
                refusal = "no account";
                return false;
            }

            lock (_lock)
            {
                if (believedCount != _chests.Count)
                {
                    refusal = "already bought";
                    return false;
                }

                if (_chests.Count >= _manager.Resources.Settings.MaxVaultChests)
                {
                    refusal = "at capacity";
                    return false;
                }

                var price = _manager.Resources.Settings.VaultChestCost;
                if (account.Fame < price)
                {
                    refusal = "not enough fame";
                    return false;
                }

                var db = _manager.Database;

                // Paid for and created together. If the transaction does not go through, no chest
                // is added and nothing is charged.
                var trans = db.Conn.CreateTransaction();
                db.CreateChest(account, trans);
                db.UpdateCurrency(account, -price, CurrencyType.Fame, trans);

                if (!trans.Execute())
                {
                    refusal = "transaction failed";
                    return false;
                }

                account.Reload("vaultCount");
                account.Reload("fame");

                _chests.Add(new Chest(account, _chests.Count));
                Version++;

                if (client.Player != null)
                    client.Player.CurrentFame = account.Fame;

                return true;
            }
        }
    }
}

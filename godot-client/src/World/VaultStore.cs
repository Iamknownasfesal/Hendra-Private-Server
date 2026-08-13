using System;
using System.Collections.Generic;
using Hendra.Net;
using Hendra.Net.Packets;
using Hendra.Resources;

namespace Hendra.World;

/// <summary>How the vault is ordered on screen. Only one of these is the order it is stored in.</summary>
public enum VaultSort
{
    /// <summary>Storage order. The only mode that is the truth, and the only one you may reorder in.</summary>
    Custom,

    Name,

    FeedPower,

    Type,
}

/// <summary>
/// The player's whole vault, and the view of it currently on screen.
/// </summary>
/// <remarks>
/// <para>
/// Two things live here and the distinction between them is the single most important rule in this
/// file. <see cref="Slots"/> is storage: what the server has, in the order the server has it, one
/// chest per eight. <see cref="View"/> is an array of indices into it -- what to draw, in what
/// order -- rebuilt whenever the sort, filter or search changes.
/// </para>
/// <para>
/// Sorting never touches storage. A sort that reorders the slot array destroys the arrangement the
/// player built and cannot give it back, and it would do so as a side effect of pressing a button
/// labelled with the name of a view. Leaving A-Z restores the previous order exactly here, because
/// nothing was ever moved.
/// </para>
/// <para>
/// Moves are optimistic: applied locally at once and sent. The server answers every accepted move
/// with the whole vault, and answers a refused one with the whole vault too, so a wrong guess is
/// corrected by the next packet rather than needing to be prevented.
/// </para>
/// </remarks>
public sealed class VaultStore
{
    public const int SlotsPerChest = 8;

    /// <summary>Empty slot. The wire says 0xffff; nothing else in the client uses that.</summary>
    public const int NoItem = -1;

    private readonly GameSession _session;
    private readonly GameData _data;

    private int[] _slots = Array.Empty<int>();
    private int[] _gifts = Array.Empty<int>();
    private int[] _view = Array.Empty<int>();

    /// <summary>Bumped by anything that changes storage, so the view knows it must be rebuilt.</summary>
    private int _storageRevision;

    /// <summary>What the view was last built for. See <see cref="Rebuild"/>.</summary>
    private int _builtRevision = -1;
    private VaultSort _builtSort;
    private string _builtFilter;
    private string _builtQuery = string.Empty;

    private VaultSort _sort = VaultSort.Custom;
    private string _filter;
    private string _query = string.Empty;

    public VaultStore(GameSession session, GameData data)
    {
        _session = session;
        _data = data;
    }

    /// <summary>Raised when the contents, the capacity or the view changed.</summary>
    public event Action Changed;

    /// <summary>What the server last told us the vault was at. Quoted back on every move.</summary>
    public int Version { get; private set; }

    public int ChestCount { get; private set; }

    public int MaxChests { get; private set; }

    public int NextChestPrice { get; private set; }

    /// <summary>Whether the server has told us anything yet.</summary>
    public bool Known { get; private set; }

    /// <summary>
    /// Whether the account may not buy another chest.
    /// </summary>
    /// <remarks>
    /// False until the server has said otherwise. Nought of a nought is nought, so a vault nobody
    /// had described yet used to answer true here and the panel announced maximum capacity to a
    /// player who owned nothing.
    /// </remarks>
    public bool AtCapacity => Known && ChestCount >= MaxChests;

    /// <summary>Storage: item types by flat index, <see cref="NoItem"/> where empty.</summary>
    public IReadOnlyList<int> Slots => _slots;

    /// <summary>
    /// Gifts waiting to be claimed, which the same panel shows and the same click takes.
    /// </summary>
    /// <remarks>
    /// Dense and never empty in the middle, because a claimed gift leaves the list rather than
    /// leaving a hole. They are not storage: nothing can be put into them, and no sort or filter
    /// touches them -- they are always the first rows and always in the order they arrived.
    /// </remarks>
    public IReadOnlyList<int> Gifts => _gifts;

    public int GiftRows => (_gifts.Length + SlotsPerChest - 1) / SlotsPerChest;

    /// <summary>What to draw, as indices into <see cref="Slots"/>.</summary>
    public IReadOnlyList<int> View
    {
        get
        {
            Rebuild();
            return _view;
        }
    }

    public VaultSort Sort
    {
        get => _sort;
        set
        {
            if (_sort == value)
                return;

            _sort = value;
            Changed?.Invoke();
        }
    }

    /// <summary>The category being shown, or null for all of them.</summary>
    public string Filter
    {
        get => _filter;
        set
        {
            if (_filter == value)
                return;

            _filter = value;
            Changed?.Invoke();
        }
    }

    public string Query
    {
        get => _query;
        set
        {
            value ??= string.Empty;
            if (_query == value)
                return;

            _query = value;
            Changed?.Invoke();
        }
    }

    /// <summary>
    /// Whether displayed position still means storage position.
    /// </summary>
    /// <remarks>
    /// It does only in plain Custom with nothing filtered and nothing searched. Everything else
    /// either reorders the grid or leaves holes in it, and a drag onto the fourth square of a
    /// filtered view has no storage slot to mean. Reordering is disabled whenever this is false;
    /// taking items out is not, because a move to the player's inventory names no vault destination.
    /// </remarks>
    public bool CanReorder =>
        _sort == VaultSort.Custom && _filter == null && _query.Length == 0;

    /// <summary>Takes the server's account of the vault, which is always the whole of it.</summary>
    public void Apply(VaultUpdatePacket packet)
    {
        Version = packet.Version;
        ChestCount = packet.ChestCount;
        MaxChests = packet.MaxChests;
        NextChestPrice = packet.NextChestPrice;
        Known = true;

        if (_slots.Length != packet.Slots.Length)
            _slots = new int[packet.Slots.Length];

        for (int i = 0; i < packet.Slots.Length; i++)
            _slots[i] = packet.Slots[i] == ushort.MaxValue ? NoItem : packet.Slots[i];

        if (_gifts.Length != packet.Gifts.Length)
            _gifts = new int[packet.Gifts.Length];

        for (int i = 0; i < packet.Gifts.Length; i++)
            _gifts[i] = packet.Gifts[i] == ushort.MaxValue ? NoItem : packet.Gifts[i];

        _storageRevision++;
        Changed?.Invoke();
    }

    /// <summary>Forgets everything, for leaving the vault.</summary>
    public void Clear()
    {
        if (!Known)
            return;

        Known = false;
        ChestCount = 0;
        _slots = Array.Empty<int>();
        _gifts = Array.Empty<int>();
        _storageRevision++;
        Changed?.Invoke();
    }

    public int ItemAt(int flatIndex) =>
        flatIndex >= 0 && flatIndex < _slots.Length ? _slots[flatIndex] : NoItem;

    public int GiftAt(int index) =>
        index >= 0 && index < _gifts.Length ? _gifts[index] : NoItem;

    /// <summary>The item's description, or null for an empty slot.</summary>
    public ObjectDesc DescAt(int flatIndex) => Describe(ItemAt(flatIndex));

    public ObjectDesc DescOfGift(int index) => Describe(GiftAt(index));

    private ObjectDesc Describe(int type) =>
        type == NoItem ? null : _data?.GetObject((ushort)type);

    /// <summary>
    /// Claims a gift into a slot of the player's own inventory.
    /// </summary>
    /// <remarks>
    /// No local guess: a gift leaves the account as well as the panel, and the item only exists in
    /// one place at a time. Drawing it as taken before the server agrees would show an item the
    /// player might not get.
    /// </remarks>
    public void ClaimGift(int index, int intoSlot)
    {
        if (GiftAt(index) == NoItem)
            return;

        _session?.Send(new VaultMovePacket
        {
            Version = Version,
            FromChest = VaultMovePacket.GiftChest,
            FromSlot = (short)index,
            ToChest = VaultMovePacket.PlayerChest,
            ToSlot = (short)intoSlot,
        });
    }

    /// <summary>
    /// Moves an item from one vault slot to another, or between the vault and the inventory.
    /// </summary>
    /// <remarks>
    /// A swap in both directions, matching what the server does, so the local guess and the
    /// authoritative answer agree about the case where the destination was occupied.
    /// </remarks>
    public void Move(SlotAddress from, SlotAddress to)
    {
        if (from.Equals(to))
            return;

        short fromChest, fromSlot, toChest, toSlot;
        if (!Address(from, out fromChest, out fromSlot) || !Address(to, out toChest, out toSlot))
            return;

        // Reordering inside the vault is only meaningful while the grid is in storage order.
        if (from.Owner == SlotOwner.Vault && to.Owner == SlotOwner.Vault && !CanReorder)
            return;

        Guess(from, to);

        _session?.Send(new VaultMovePacket
        {
            Version = Version,
            FromChest = fromChest,
            FromSlot = fromSlot,
            ToChest = toChest,
            ToSlot = toSlot,
        });
    }

    /// <summary>
    /// Sends a potion from the vault straight into one of the two stacks.
    /// </summary>
    /// <remarks>
    /// Its own move rather than the ordinary one, because a stack is not a slot: there is nothing
    /// at the far end to send back, and the vault's swap is a swap all the way down. The
    /// alternative was making the player drag the potion out to a bag first and then onto the
    /// counter, which is two drags for one intention.
    /// </remarks>
    public void Stack(SlotAddress from, bool health)
    {
        short chest, slot;
        if (from.Owner != SlotOwner.Vault || !Address(from, out chest, out slot))
            return;

        // Drawn straight away on the vault's side. The count itself is a stat and arrives when the
        // server says so; this is only the square the potion left.
        if (from.Index >= 0 && from.Index < _slots.Length)
        {
            _slots[from.Index] = NoItem;
            _storageRevision++;
            Changed?.Invoke();
        }

        _session?.Send(new VaultMovePacket
        {
            Version = Version,
            FromChest = chest,
            FromSlot = slot,
            ToChest = VaultMovePacket.PotionStacks,
            ToSlot = (short)(health ? 0 : 1),
        });
    }

    public void Buy()
    {
        if (!Known || AtCapacity)
            return;

        _session?.Send(new VaultBuyPacket { ChestCount = ChestCount });
    }

    /// <summary>
    /// Draws the move before the server has agreed to it.
    /// </summary>
    /// <remarks>
    /// Only the vault's half: the player's inventory is drawn from the entity's own equipment, which
    /// the server corrects on its own schedule, and guessing at both halves here would mean two
    /// places disagreeing about the same swap.
    /// </remarks>
    private void Guess(SlotAddress from, SlotAddress to)
    {
        bool fromVault = from.Owner == SlotOwner.Vault;
        bool toVault = to.Owner == SlotOwner.Vault;

        if (fromVault && toVault)
        {
            (_slots[to.Index], _slots[from.Index]) = (_slots[from.Index], _slots[to.Index]);
        }
        else if (fromVault)
        {
            _slots[from.Index] = NoItem;
        }
        else if (toVault)
        {
            // What is arriving is not known here -- the inventory holds it -- so the slot is only
            // emptied if it held something that is now on its way out. The update that follows
            // fills it in.
            _slots[to.Index] = NoItem;
        }

        _storageRevision++;
        Changed?.Invoke();
    }

    /// <summary>Turns a slot address into the pair the wire uses.</summary>
    private bool Address(SlotAddress address, out short chest, out short slot)
    {
        if (address.Owner == SlotOwner.Player)
        {
            chest = VaultMovePacket.PlayerChest;
            slot = (short)address.Index;
            return true;
        }

        if (address.Owner != SlotOwner.Vault || address.Index < 0 || address.Index >= _slots.Length)
        {
            chest = 0;
            slot = 0;
            return false;
        }

        chest = (short)(address.Index / SlotsPerChest);
        slot = (short)(address.Index % SlotsPerChest);
        return true;
    }

    /// <summary>
    /// Rebuilds the list of what to draw, if anything it depends on has changed.
    /// </summary>
    /// <remarks>
    /// Indices rather than items throughout: the comparisons are on numbers and the array that comes
    /// out is numbers, so a vault of several hundred slots sorts without touching an object. It is
    /// memoized on the sort, the filter, the query and the storage revision, which is the whole of
    /// what it depends on.
    /// </remarks>
    private void Rebuild()
    {
        if (_builtRevision == _storageRevision &&
            _builtSort == _sort &&
            _builtFilter == _filter &&
            _builtQuery == _query)
            return;

        _builtRevision = _storageRevision;
        _builtSort = _sort;
        _builtFilter = _filter;
        _builtQuery = _query;

        bool plain = _filter == null && _query.Length == 0;

        // Nothing hidden and nothing reordered: the view is the storage, and saying so avoids
        // building and sorting an array that would come out in the order it went in.
        if (plain && _sort == VaultSort.Custom)
        {
            if (_view.Length != _slots.Length)
                _view = new int[_slots.Length];

            for (int i = 0; i < _slots.Length; i++)
                _view[i] = i;

            return;
        }

        var kept = new List<int>(_slots.Length);

        for (int i = 0; i < _slots.Length; i++)
        {
            // A filtered or searched grid is compacted, so empty slots are not carried through --
            // there is no such thing as an empty slot that matches "sword".
            if (!plain && _slots[i] == NoItem)
                continue;

            if (Matches(i))
                kept.Add(i);
        }

        if (_sort != VaultSort.Custom)
            kept.Sort(Compare);

        _view = kept.ToArray();
    }

    private bool Matches(int index)
    {
        var desc = DescAt(index);
        if (desc == null)
            return _filter == null && _query.Length == 0;

        if (_filter != null && ItemCategories.Of(desc) != _filter)
            return false;

        if (_query.Length == 0)
            return true;

        string name = desc.DisplayId ?? desc.Id ?? string.Empty;
        return name.Contains(_query, StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>
    /// Orders two slots under the current view.
    /// </summary>
    /// <remarks>
    /// Storage order is the tie-break in every mode, so a sort is stable in the way a player
    /// notices: two identical items stay in the order they were put away in.
    /// </remarks>
    private int Compare(int a, int b)
    {
        var da = DescAt(a);
        var db = DescAt(b);

        // An empty slot sorts last, which only arises in an unfiltered A-Z.
        if (da == null || db == null)
            return da == db ? a.CompareTo(b) : da == null ? 1 : -1;

        int order = _sort switch
        {
            VaultSort.Name => string.Compare(
                da.DisplayId ?? da.Id, db.DisplayId ?? db.Id, StringComparison.OrdinalIgnoreCase),

            // Best first: the question a feed power sort is asked is what to feed, not what not to.
            VaultSort.FeedPower => db.FeedPower.CompareTo(da.FeedPower),

            VaultSort.Type => string.Compare(
                ItemCategories.Of(da), ItemCategories.Of(db), StringComparison.Ordinal),

            _ => 0,
        };

        return order != 0 ? order : a.CompareTo(b);
    }
}

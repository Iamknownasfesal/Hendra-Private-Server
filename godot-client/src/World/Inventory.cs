using Hendra.Core;
using Hendra.Data;
using Hendra.Net;
using Hendra.Net.Packets;
using Hendra.Resources;

namespace Hendra.World;

/// <summary>
/// Equipping, using and swapping items.
/// </summary>
/// <remarks>
/// <para>
/// The layout is one array of 24 with three regions: 0-7 worn, 8-15 carried, 16-23 backpack. Only
/// the first four worn slots are ever used, because a player's SlotTypes declares four.
/// </para>
/// <para>
/// Changes are applied optimistically so the panel responds immediately. That is safe: the server
/// answers a rejected swap with both slots' true contents, so a wrong guess corrects itself within
/// a tick rather than needing to be prevented.
/// </para>
/// </remarks>
public sealed class Inventory
{
    /// <summary>Empty slot marker. Zero is a valid object type, so it cannot be the sentinel.</summary>
    public const int NoItem = -1;

    private const int WornSlots = 8;
    private const int CarriedFirst = 8;
    private const int CarriedLast = 15;

    /// <summary>The two potion slots, which live outside the array and are addressed by id.</summary>
    public const byte HealthPotionSlot = 254;

    public const byte MagicPotionSlot = 255;

    private readonly GameMap _map;
    private readonly GameData _data;
    private readonly GameSession _session;
    private readonly GameClock _clock;

    public Inventory(GameMap map, GameData data, GameSession session, GameClock clock)
    {
        _map = map;
        _data = data;
        _session = session;
        _clock = clock;
    }

    /// <summary>
    /// Does whatever clicking this slot should do: use a consumable, equip a carried item, or
    /// unequip a worn one.
    /// </summary>
    public void Activate(int slotIndex)
    {
        var player = _map.Player;
        if (player?.Equipment == null || slotIndex < 0 || slotIndex >= player.Equipment.Length)
            return;

        int type = player.Equipment[slotIndex];
        if (type == NoItem)
            return;

        var item = _data.GetObject((ushort)type);
        if (item == null)
            return;

        if (item.Consumable)
        {
            Use(slotIndex, type);
            return;
        }

        // A worn item comes off; a carried one goes on.
        if (slotIndex < WornSlots)
            MoveToFirstFreeCarried(player, slotIndex, type);
        else
            Equip(player, slotIndex, item, type);
    }

    /// <summary>
    /// Moves an item out of a nearby container and into the first free carried slot.
    /// </summary>
    /// <remarks>
    /// The server requires the two entities to be within one tile of each other and refuses
    /// otherwise, so this is only ever called for the container the interaction scan found.
    /// </remarks>
    public void TakeFrom(Entity container, int containerSlot)
    {
        var player = _map.Player;
        if (player?.Equipment == null || container?.Equipment == null)
            return;

        if (containerSlot < 0 || containerSlot >= container.Equipment.Length)
            return;

        int type = container.Equipment[containerSlot];
        if (type == NoItem)
            return;

        int destination = FirstFreeCarried(player);
        if (destination < 0)
            return;

        _session.Send(new InvSwapPacket
        {
            Time = _clock.FrameMs,
            Position = new WorldPos(player.X, player.Y),
            Slot1 = new SlotObject(container.ObjectId, (byte)containerSlot, type),
            Slot2 = new SlotObject(player.ObjectId, (byte)destination, NoItem),
        });

        container.Equipment[containerSlot] = NoItem;
        player.Equipment[destination] = type;
    }

    /// <summary>Moves a carried item into the first free slot of a nearby container.</summary>
    public void PutInto(Entity container, int playerSlot)
    {
        var player = _map.Player;
        if (player?.Equipment == null || container?.Equipment == null)
            return;

        if (playerSlot < 0 || playerSlot >= player.Equipment.Length)
            return;

        int type = player.Equipment[playerSlot];
        if (type == NoItem)
            return;

        int destination = -1;
        for (int i = 0; i < container.Equipment.Length; i++)
        {
            if (container.Equipment[i] != NoItem)
                continue;
            destination = i;
            break;
        }

        if (destination < 0)
            return;

        _session.Send(new InvSwapPacket
        {
            Time = _clock.FrameMs,
            Position = new WorldPos(player.X, player.Y),
            Slot1 = new SlotObject(player.ObjectId, (byte)playerSlot, type),
            Slot2 = new SlotObject(container.ObjectId, (byte)destination, NoItem),
        });

        player.Equipment[playerSlot] = NoItem;
        container.Equipment[destination] = type;
    }

    private static int FirstFreeCarried(LocalPlayer player)
    {
        for (int i = CarriedFirst; i <= CarriedLast; i++)
        {
            if (player.Equipment[i] == NoItem)
                return i;
        }
        return -1;
    }

    /// <summary>Drinks one of the two stacked potions, which are addressed by slot id, not index.</summary>
    public void UsePotion(bool health)
    {
        var player = _map.Player;
        if (player == null)
            return;

        byte slot = health ? HealthPotionSlot : MagicPotionSlot;
        _session.Send(new UseItemPacket
        {
            Time = _clock.FrameMs,
            Slot = new SlotObject(player.ObjectId, slot, NoItem),
            ItemUsePos = new WorldPos(player.X, player.Y),
            UseType = ItemUseType.Default,
        });
    }

    private void Use(int slotIndex, int type)
    {
        var player = _map.Player;
        _session.Send(new UseItemPacket
        {
            Time = _clock.FrameMs,
            Slot = new SlotObject(player.ObjectId, (byte)slotIndex, type),
            ItemUsePos = new WorldPos(player.X, player.Y),
            UseType = ItemUseType.Default,
        });

        // Consumables vanish on use; the server confirms with a stat update.
        player.Equipment[slotIndex] = NoItem;
    }

    /// <summary>Moves a carried item into whichever worn slot accepts its type.</summary>
    private void Equip(LocalPlayer player, int fromIndex, ObjectDesc item, int type)
    {
        int target = FindWornSlotFor(player, item);
        if (target < 0)
            return;

        Swap(player, fromIndex, target, type, player.Equipment[target]);
    }

    private void MoveToFirstFreeCarried(LocalPlayer player, int fromIndex, int type)
    {
        for (int i = CarriedFirst; i <= CarriedLast; i++)
        {
            if (player.Equipment[i] != NoItem)
                continue;

            Swap(player, fromIndex, i, type, NoItem);
            return;
        }
    }

    /// <summary>
    /// The worn slot whose declared type matches this item, or -1 if the class cannot wear it.
    /// </summary>
    private static int FindWornSlotFor(LocalPlayer player, ObjectDesc item)
    {
        var slotTypes = player.Desc?.SlotTypes;
        if (slotTypes == null || item.SlotType < 0)
            return -1;

        for (int i = 0; i < slotTypes.Length && i < WornSlots; i++)
        {
            if (slotTypes[i] == item.SlotType)
                return i;
        }

        return -1;
    }

    private void Swap(LocalPlayer player, int fromIndex, int toIndex, int fromType, int toType)
    {
        _session.Send(new InvSwapPacket
        {
            Time = _clock.FrameMs,
            Position = new WorldPos(player.X, player.Y),
            Slot1 = new SlotObject(player.ObjectId, (byte)fromIndex, fromType),
            Slot2 = new SlotObject(player.ObjectId, (byte)toIndex, toType),
        });

        // Applied straight away so the panel does not lag a round trip behind the click. A refusal
        // comes back with both slots' real contents attached.
        player.Equipment[fromIndex] = toType;
        player.Equipment[toIndex] = fromType;
    }
}

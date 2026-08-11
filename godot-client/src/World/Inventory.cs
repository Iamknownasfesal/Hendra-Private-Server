using System;
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

    /// <summary>Set after construction; abilities that shoot fire through it.</summary>
    private Combat _shoot;

    public Inventory(GameMap map, GameData data, GameSession session, GameClock clock)
    {
        _map = map;
        _data = data;
        _session = session;
        _clock = clock;
    }

    /// <summary>
    /// Gives the inventory somewhere to fire an ability's shots.
    /// </summary>
    /// <remarks>
    /// Set afterwards rather than taken in the constructor: combat needs the session and the map,
    /// and the two are built alongside each other.
    /// </remarks>
    public void UseCombat(Combat combat) => _shoot = combat;

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

    /// <summary>
    /// The worn slot the ability lives in.
    /// </summary>
    /// <remarks>
    /// Positional rather than looked up by slot type, because the slot type differs per class — a
    /// wizard's spell is type 11, a priest's tome type 4 — while the position does not. The server
    /// hard-codes the same index: its shot handler tests the named item against
    /// <c>player.Inventory[1]</c> to tell an ability shot from a weapon shot.
    /// </remarks>
    public const int AbilitySlot = 1;

    /// <summary>When the ability may next be used. The original's nextAltAttack_.</summary>
    private int _abilityReadyAtMs;

    /// <summary>Whether a multi-phase ability is part way through, waiting for its release.</summary>
    private bool _abilityCharging;

    /// <summary>Default cooldown for an ability whose definition names none, in milliseconds.</summary>
    private const int DefaultAbilityCooldownMs = 500;

    /// <summary>
    /// How far from the player an ability may be aimed, in tiles.
    /// </summary>
    /// <remarks>
    /// The server's own limit, and it enforces it <i>after</i> taking the magic: an ability aimed
    /// beyond this is paid for and then does nothing, with no message. So the point is pulled back
    /// to the limit here rather than sent as aimed. The original did not, and a player who clicked
    /// across a wide screen lost the cast.
    ///
    /// Half a tile inside the server's fourteen, because it compares squared distances in floats
    /// and a point clamped to exactly the limit lands on the wrong side of the comparison about
    /// half the time.
    /// </remarks>
    private const float MaxAbilityRange = 13.5f;

    /// <summary>The equipped ability, or null if the slot is empty or holds something unusable.</summary>
    public ObjectDesc EquippedAbility
    {
        get
        {
            var player = _map.Player;
            if (player?.Equipment == null || player.Equipment.Length <= AbilitySlot)
                return null;

            int type = player.Equipment[AbilitySlot];
            if (type == NoItem)
                return null;

            var item = _data.GetObject((ushort)type);
            return item is { Usable: true } ? item : null;
        }
    }

    /// <summary>Whether the ability is off cooldown and there is enough magic for it.</summary>
    public bool CanUseAbility(int nowMs)
    {
        var ability = EquippedAbility;
        return ability != null
               && nowMs >= _abilityReadyAtMs
               && _map.Player != null
               && _map.Player.Mp >= ability.MpCost;
    }

    /// <summary>
    /// Begins using the equipped ability, aimed at a point in the world.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Magic, cooldown and the effect itself are all applied server-side; the checks here only stop
    /// the client sending a use that would be thrown away, since the server answers a refused one
    /// with a bare InvResult and no explanation.
    /// </para>
    /// <para>
    /// An ability that shoots also fires locally. That is not prediction — it is the only way the
    /// shots appear at all, because the server broadcasts them to every nearby player <i>except</i>
    /// the one who fired. See <see cref="Combat.FireAbility"/> for why doing so keeps the shared
    /// random stream in step rather than breaking it.
    /// </para>
    /// </remarks>
    /// <returns>Whether the use was sent.</returns>
    public bool BeginAbility(int nowMs, float targetX, float targetY)
    {
        var player = _map.Player;
        var ability = EquippedAbility;

        if (player == null || player.IsPaused || ability == null || _abilityCharging)
            return false;

        if (nowMs < _abilityReadyAtMs || player.Mp < ability.MpCost)
            return false;

        (targetX, targetY) = ClampToRange(player, targetX, targetY);

        _abilityReadyAtMs = nowMs +
            (ability.CooldownMs > 0 ? ability.CooldownMs : DefaultAbilityCooldownMs);

        Send(player, ability, nowMs, targetX, targetY, ItemUseType.StartUse);

        if (ability.MultiPhase)
        {
            // Held down: the shot comes on release, once the end cost has been paid too.
            _abilityCharging = true;
            return true;
        }

        if (ability.ActivatesShoot)
            _shoot?.FireAbility(ability, MathF.Atan2(targetY - player.Y, targetX - player.X), nowMs);

        return true;
    }

    /// <summary>
    /// Releases a multi-phase ability. Harmless when none is charging.
    /// </summary>
    public void EndAbility(int nowMs, float targetX, float targetY)
    {
        if (!_abilityCharging)
            return;

        _abilityCharging = false;

        var player = _map.Player;
        var ability = EquippedAbility;
        if (player == null || ability == null)
            return;

        (targetX, targetY) = ClampToRange(player, targetX, targetY);
        Send(player, ability, nowMs, targetX, targetY, ItemUseType.EndUse);

        // The release only fires if the second instalment can be paid, which is checked here as
        // well as on the server so nothing is drawn that the server will not have created.
        if (ability.ActivatesShoot && player.Mp >= ability.MpEndCost)
            _shoot?.FireAbility(ability, MathF.Atan2(targetY - player.Y, targetX - player.X), nowMs);
    }

    /// <summary>Pulls an aim point back to <see cref="MaxAbilityRange"/> if it is beyond it.</summary>
    private static (float X, float Y) ClampToRange(LocalPlayer player, float targetX, float targetY)
    {
        float dx = targetX - player.X;
        float dy = targetY - player.Y;
        float distance = MathF.Sqrt(dx * dx + dy * dy);

        if (distance <= MaxAbilityRange || distance <= 0f)
            return (targetX, targetY);

        float scale = MaxAbilityRange / distance;
        return (player.X + dx * scale, player.Y + dy * scale);
    }

    private void Send(LocalPlayer player, ObjectDesc ability, int nowMs, float targetX, float targetY, ItemUseType useType) =>
        _session.Send(new UseItemPacket
        {
            Time = nowMs,
            Slot = new SlotObject(player.ObjectId, AbilitySlot, player.Equipment[AbilitySlot]),
            ItemUsePos = new WorldPos(targetX, targetY),
            UseType = useType,
        });

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

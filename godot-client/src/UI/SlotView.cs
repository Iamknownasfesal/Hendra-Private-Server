using System;
using System.Globalization;
using Godot;

namespace Hendra.UI;

/// <summary>
/// One slot: a hotbar square, a worn item, a chest's contents or a trade offer.
/// </summary>
/// <remarks>
/// <para>
/// A dark plate with a light one-pixel border. Revision one had these the other way round, near
/// white with a grey edge, and the item artwork -- dark-outlined pixel art -- was disappearing into
/// the square it was drawn on.
/// </para>
/// <para>
/// Everything in the corners is a caption on the item and not a control: the number key that uses
/// it, the mouse button that fires it, its tier. They are drawn rather than made into children
/// because a slot is redrawn only when its contents change, and three labels per slot across
/// sixteen slots is a lot of nodes to keep in step for text that never moves.
/// </para>
/// </remarks>
public sealed partial class SlotView : Control
{
    /// <summary>How long a slot's border flashes when its key is pressed.</summary>
    private const double FlashSeconds = 0.1;

    private Assets.Sprite _sprite;
    private Resources.ObjectDesc _desc;
    private Resources.GameData _data;

    private float _glow;
    private double _flashUntil = -1.0;

    private float _cooldownRemainingMs;
    private float _cooldownTotalMs;

    /// <summary>Which slot this is, so a drag can name where it came from and where it went.</summary>
    public World.SlotAddress Address { get; set; }

    /// <summary>
    /// The key that uses this slot.
    /// </summary>
    /// <remarks>
    /// The eight carried slots answer to 1 through 8. Showing the number is what turns a grid into
    /// a keyboard layout you can learn without reading the options screen. Where it is drawn
    /// depends on whether the slot is empty; see <see cref="DrawNumber"/>.
    /// </remarks>
    public string Hotkey { get; set; }

    /// <summary>The mouse button this slot is bound to, drawn in its corner. Null for most slots.</summary>
    public bool? MouseBind { get; set; }

    private bool _usable = true;

    /// <summary>
    /// Whether the character can equip what is in this slot.
    /// </summary>
    /// <remarks>
    /// False turns the plate red. It is the answer to the question you ask of every item that drops
    /// -- can I use this -- and the original answers it the same way, with the colour of the tile
    /// rather than with anything you have to hover over.
    /// </remarks>
    public bool Usable
    {
        get => _usable;
        set
        {
            if (_usable == value)
                return;

            _usable = value;
            QueueRedraw();
        }
    }

    /// <summary>
    /// Whether this slot takes part in dragging.
    /// </summary>
    /// <remarks>
    /// Off unless a slot has been given a real address. The trade screen uses the same view for its
    /// offers, and a slot there dragging under the default address would move whatever happens to
    /// be in the player's first equipment slot.
    /// </remarks>
    public bool Draggable { get; set; }

    /// <summary>Raised on a left click, whether or not the slot holds anything.</summary>
    public event Action Activated;

    /// <summary>Raised when something is dropped on this slot, with where it came from.</summary>
    public event Action<World.SlotAddress, World.SlotAddress> Dropped;

    /// <summary>Raised when this slot's item was dragged out and let go over the world.</summary>
    public event Action<World.SlotAddress> DroppedOutside;

    /// <summary>Whether a drag out of this slot is in flight.</summary>
    private bool _dragging;

    /// <summary>
    /// Notices a drag of this slot's item that nothing accepted.
    /// </summary>
    /// <remarks>
    /// Godot tells every control when a drag ends and whether anything took it. Nothing took it
    /// means it was let go over the world, which is the gesture for dropping an item on the ground
    /// -- there is no other way to say it, and every other game in this shape says it this way.
    /// </remarks>
    public override void _Notification(int what)
    {
        base._Notification(what);

        if (what != NotificationDragEnd || !_dragging)
            return;

        _dragging = false;

        if (!GetViewport().GuiIsDragSuccessful())
            DroppedOutside?.Invoke(Address);
    }

    /// <summary>Flashes the border, so a key press is visibly acknowledged.</summary>
    public void Flash()
    {
        _flashUntil = Time.GetUnixTimeFromSystem() + FlashSeconds;
        QueueRedraw();
    }

    /// <summary>
    /// Sets the cooldown wipe.
    /// </summary>
    /// <remarks>
    /// Both numbers are handed in each frame from the clock the cooldown was started against, not
    /// counted down here. A frame counter loses thirty seconds of cooldown to thirty seconds of
    /// being tabbed out; a timestamp does not notice.
    /// </remarks>
    public void SetCooldown(float remainingMs, float totalMs)
    {
        if (Mathf.IsEqualApprox(_cooldownRemainingMs, remainingMs))
            return;

        _cooldownRemainingMs = remainingMs;
        _cooldownTotalMs = totalMs;
        QueueRedraw();
    }

    public override Variant _GetDragData(Vector2 atPosition)
    {
        if (!Draggable || !_sprite.IsValid || _desc == null)
            return default;

        SetDragPreview(new DragPreview(_sprite, Size));
        _dragging = true;

        return new Godot.Collections.Dictionary
        {
            ["hendra_slot"] = true,
            ["owner"] = (int)Address.Owner,
            ["index"] = Address.Index,
        };
    }

    public override bool _CanDropData(Vector2 atPosition, Variant data) =>
        Draggable && IsSlotPayload(data, out _);

    public override void _DropData(Vector2 atPosition, Variant data)
    {
        if (IsSlotPayload(data, out var from))
            Dropped?.Invoke(from, Address);
    }

    private static bool IsSlotPayload(Variant data, out World.SlotAddress address)
    {
        address = default;

        if (data.VariantType != Variant.Type.Dictionary)
            return false;

        var payload = data.AsGodotDictionary();
        if (!payload.ContainsKey("hendra_slot"))
            return false;

        address = new World.SlotAddress(
            (World.SlotOwner)(int)payload["owner"], (int)payload["index"]);
        return true;
    }

    /// <summary>The item riding the cursor while it is being dragged.</summary>
    private sealed partial class DragPreview : Control
    {
        private readonly Assets.Sprite _sprite;
        private readonly Vector2 _size;

        public DragPreview(Assets.Sprite sprite, Vector2 size)
        {
            _sprite = sprite;
            _size = size;
            CustomMinimumSize = size;

            // Centred on the pointer, so the item sits under the finger that picked it up.
            Position = -size / 2f;
            MouseFilter = MouseFilterEnum.Ignore;
        }

        public override void _Draw()
        {
            if (_sprite.IsValid)
                DrawTextureRectRegion(_sprite.Sheet, Artwork(_size), _sprite.Region);
        }
    }

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Stop;
        FocusMode = FocusModeEnum.None;

        MouseEntered += QueueRedraw;
        MouseExited += QueueRedraw;
    }

    public override void _Process(double delta)
    {
        if (_flashUntil > 0.0 && Time.GetUnixTimeFromSystem() > _flashUntil)
        {
            _flashUntil = -1.0;
            QueueRedraw();
        }

        float target = _sprite.IsValid && GetGlobalRect().HasPoint(GetGlobalMousePosition()) ? 1f : 0f;
        float eased = Mathf.MoveToward(_glow, target, (float)delta * 8f);

        if (Mathf.IsEqualApprox(eased, _glow))
            return;

        _glow = eased;
        QueueRedraw();
    }

    public override void _GuiInput(InputEvent @event)
    {
        // On release, and only if the press did not turn into a drag. Acting on the press would
        // mean every drag also used the item it picked up. The original draws the same line: it
        // dispatches its click on mouse-up and skips it while a drag is running.
        if (@event is not InputEventMouseButton { Pressed: false, ButtonIndex: MouseButton.Left })
            return;

        if (GetViewport().GuiIsDragging())
            return;

        Activated?.Invoke();
    }

    public void SetItem(Assets.Sprite sprite, Resources.ObjectDesc desc, Resources.GameData data)
    {
        bool same = _desc == desc && _sprite.IsValid == sprite.IsValid;

        _sprite = sprite;
        _desc = desc;
        _data = data;

        // Godot only asks for a tooltip when this is non-empty, so it stands in for "there is
        // something here to describe". The text itself is never shown -- _MakeCustomTooltip
        // replaces it with the panel.
        TooltipText = desc == null ? string.Empty : " ";

        // Only when it actually changed. This is called for every slot every frame, and redrawing
        // sixteen slots sixty times a second for nothing is the layout thrash the brief warns
        // about. It is also what moves the slot's number between its two states.
        if (!same)
            QueueRedraw();
    }

    /// <summary>
    /// Builds the panel the original shows, in place of Godot's own text tooltip.
    /// </summary>
    /// <remarks>
    /// Built fresh each time rather than kept: the engine frees the control when the tooltip
    /// closes, and an item's description does not change while the pointer is over it.
    /// </remarks>
    public override Control _MakeCustomTooltip(string forText)
    {
        if (_desc == null)
            return null;

        return new ItemTooltipPanel(_desc, _sprite, _data, App.ServiceLocator.Strings);
    }

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);

        var plate = !_sprite.IsValid ? Style.Slot
            : _usable ? Style.Slot
            : Style.SlotRestricted;

        DrawRect(full, plate.Lightened(_glow * 0.12f));

        if (_sprite.IsValid)
            DrawTextureRectRegion(_sprite.Sheet, Artwork(Size), _sprite.Region);

        DrawCooldown();
        DrawNumber();
        DrawMouseBind();
        DrawTierTag();
        DrawBorder(full);
    }

    /// <summary>
    /// Where the item's artwork goes: a square, centred, whatever shape the slot is.
    /// </summary>
    /// <remarks>
    /// The slots are not square -- the hotbar's are 55 by 48 and the equipment row's 85 by 78 --
    /// and filling them edge to edge stretched every sprite sideways. Item art is square pixels;
    /// the slot is the thing that is allowed to be oblong.
    /// </remarks>
    private static Rect2 Artwork(Vector2 size)
    {
        float inset = Mathf.Max(3f, Mathf.Round(size.X * 0.09f));
        float side = Mathf.Round(Mathf.Min(size.X, size.Y) - inset * 2f);

        return new Rect2(Mathf.Round((size.X - side) / 2f), Mathf.Round((size.Y - side) / 2f), side, side);
    }

    /// <summary>
    /// The border, which is also where a key press is acknowledged.
    /// </summary>
    /// <remarks>
    /// A hundred milliseconds of a bright edge. Without it a key that fires an item nothing visible
    /// happens to -- an ability on cooldown, a potion at full health -- is indistinguishable from a
    /// key that did not register.
    /// </remarks>
    private void DrawBorder(in Rect2 full)
    {
        bool lit = _flashUntil > 0.0 || _glow > 0.5f;

        DrawRect(full, lit ? Style.SlotBorderHi : Style.SlotBorder, filled: false, width: 1f);
    }

    /// <summary>
    /// The slot's number, which moves depending on whether anything is in the slot.
    /// </summary>
    /// <remarks>
    /// Large and centred while the slot is empty, small in the top left corner once something is in
    /// it. An empty slot has nothing else to say, so the number can be the whole of it and be
    /// readable at a glance; a full one has artwork to show, and the number becomes a caption on it.
    /// </remarks>
    private void DrawNumber()
    {
        if (string.IsNullOrEmpty(Hotkey))
            return;

        if (_sprite.IsValid)
        {
            this.DrawOutlined(new Vector2(4f, Style.FontTag + 4f), Hotkey, Style.FontTag, Style.TextDim);
            return;
        }

        int size = Mathf.Max(Style.FontName, (int)(Size.Y * 0.42f));
        float width = Style.Measure(Hotkey, size);
        float baseline = Mathf.Round(
            (Size.Y + Style.Pixel.GetAscent(size) - Style.Pixel.GetDescent(size)) / 2f);

        this.DrawOutlined(
            new Vector2(Mathf.Round((Size.X - width) / 2f), baseline), Hotkey, size, Style.SlotEmptyNumber);
    }

    /// <summary>Which mouse button fires this slot, for the weapon and the ability.</summary>
    private void DrawMouseBind()
    {
        if (MouseBind is not { } right || !_sprite.IsValid)
            return;

        float height = Mathf.Min(18f, Size.Y * 0.24f);
        var box = new Rect2(Size.X - height * 0.75f - 4f, 4f, height * 0.75f, height);

        HudIcons.MouseButton(this, box, Style.TextDim, right);
    }

    /// <summary>The dark wipe over a slot that cannot be used yet, and how long is left of it.</summary>
    private void DrawCooldown()
    {
        if (_cooldownRemainingMs <= 0f || _cooldownTotalMs <= 0f)
            return;

        float fraction = Mathf.Clamp(_cooldownRemainingMs / _cooldownTotalMs, 0f, 1f);

        // A vertical wipe: the dark part drains downward as the cooldown runs out, so the slot
        // fills back up with itself. Seventy percent rather than sixty, because the plate under it
        // is already dark.
        DrawRect(new Rect2(0f, 0f, Size.X, Size.Y * fraction), new Color(0f, 0f, 0f, 0.7f));

        string remaining = (_cooldownRemainingMs / 1000f).ToString(
            _cooldownRemainingMs >= 1000f ? "0" : "0.0", CultureInfo.InvariantCulture);

        float width = Style.Measure(remaining, Style.FontBody);
        float baseline = Mathf.Round(
            (Size.Y + Style.Pixel.GetAscent(Style.FontBody) - Style.Pixel.GetDescent(Style.FontBody)) / 2f);

        this.DrawOutlined(
            new Vector2(Mathf.Round((Size.X - width) / 2f), baseline), remaining, Style.FontBody, Style.Text);
    }

    /// <summary>
    /// The item's grade, in the bottom right corner of the slot.
    /// </summary>
    /// <remarks>
    /// Straight off the item's own data -- its tier, or UT where it has none -- never a table kept
    /// in the interface. It is how you read a bag at a glance instead of hovering over every square
    /// in it, and the untiered grades are the ones worth stopping on, so they get the accent.
    /// </remarks>
    private void DrawTierTag()
    {
        string tag = ItemTooltip.TierTag(_desc);
        if (tag == null || !_sprite.IsValid)
            return;

        float width = Style.Measure(tag, Style.FontTag);

        this.DrawOutlined(
            new Vector2(Size.X - width - 3f, Size.Y - 4f), tag, Style.FontTag,
            tag == "UT" ? Style.TierSpecial : Style.TierNormal);
    }
}

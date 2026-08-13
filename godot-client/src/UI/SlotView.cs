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
/// white with a grey edge, and the item artwork was disappearing into the square it was drawn on.
/// The artwork is outlined too -- see <see cref="Style.DrawSprite"/> -- because a dark plate on its
/// own does not save a dark item, and half the weapons in the game are iron.
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

    /// <summary>How thick the border is. Two: one vanishes, and the border is what draws the grid.</summary>
    public const float Border = 2f;

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
    /// <summary>
    /// The input action this slot fires on, or null for a slot that is not bound to one.
    /// </summary>
    /// <remarks>
    /// The action rather than a picture of a button: these two are the only part of the control
    /// scheme written nowhere else, and a player who has rebound the ability to a key should see
    /// that key here rather than a mouse that lies to them.
    /// </remarks>
    public string BoundAction { get; set; }

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
    /// How many of this item the slot holds, drawn in the top left.
    /// </summary>
    /// <remarks>
    /// Zero draws nothing, which is every slot on this server: the wire carries an item type per
    /// slot and no quantity anywhere, so nothing can currently set this above zero. It is here
    /// because the corner is reserved for it -- the key number moved out of that corner to make
    /// room -- and because a stacking build should need to fill this in and nothing else.
    /// </remarks>
    public int StackCount { get; set; }

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
            this.DrawSprite(_sprite, Artwork(_size, _sprite));
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

    /// <summary>
    /// What this slot is saying about its item, beyond the artwork.
    /// </summary>
    /// <remarks>
    /// Resolved rather than stored, because the only meaning wired to it is one this slot already
    /// knows: an item the character cannot equip. A slot given a highlight for some other reason
    /// would set this instead.
    /// </remarks>
    private SlotHighlight Highlight =>
        _sprite.IsValid && !_usable ? SlotHighlight.Red : SlotHighlight.None;

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);
        var (fill, edge) = SlotHighlights.Pair(Highlight, _sprite.IsValid);

        DrawRect(full, fill.Lightened(_glow * 0.12f));

        this.DrawSprite(_sprite, Artwork(Size, _sprite));

        DrawCooldown();
        DrawStack();
        DrawNumber();
        DrawMouseBind();
        if (App.ServiceLocator.Settings is not { ShowTierLevel: false })
            DrawTierTag();
        DrawBorder(full, edge);
    }

    /// <summary>How many of it there are, in the corner reserved for the question.</summary>
    private void DrawStack()
    {
        if (StackCount <= 1 || !_sprite.IsValid)
            return;

        this.DrawToken(
            new Vector2(4f, Style.FontTag + 4f),
            StackCount.ToString(CultureInfo.InvariantCulture), Style.FontTag, Style.Text);
    }

    /// <summary>How much of the slot's short side the artwork is allowed, as a fraction.</summary>
    /// <remarks>
    /// Revision five: the item should look like it is in the slot rather than floating in the middle
    /// of one. Revision two left about six tenths and the reference is nearer nine.
    /// </remarks>
    private const float ArtworkFill = 0.88f;

    /// <summary>
    /// Where the item's artwork goes: a square, centred, whatever shape the slot is.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The slots are not square -- the hotbar's are 55 by 48 and the equipment row's 85 by 78 --
    /// and filling them edge to edge stretched every sprite sideways. Item art is square pixels;
    /// the slot is the thing that is allowed to be oblong.
    /// </para>
    /// <para>
    /// The side is a whole multiple of the source sprite, never the exact fraction above. An eight
    /// by eight sprite drawn at eighty-four pixels puts ten and a half screen pixels on each source
    /// pixel, which means half of them are eleven wide and half are ten, and which half is which
    /// changes as the slot moves -- the shimmer the snapping rule in revision two exists to
    /// prevent. Ten times eight is eighty, and every pixel in it is square.
    /// </para>
    /// </remarks>
    private static Rect2 Artwork(Vector2 size, in Assets.Sprite sprite)
    {
        float shortest = Mathf.Min(size.X, size.Y);
        float room = Mathf.Floor(shortest * ArtworkFill);

        int source = sprite.IsValid ? Mathf.Min(sprite.Region.Size.X, sprite.Region.Size.Y) : 0;

        // The whole multiple is counted in screen pixels, not in reference ones. A slot is drawn
        // through a canvas that may be at one and a half, and a sprite that is a clean ten times
        // its source in reference pixels is fifteen on the glass -- which is clean too -- while an
        // eleven times is sixteen and a half, and half its rows come out a pixel fatter than the
        // rest. Counting on the far side of the scale is what stops that.
        float scale = Style.Sharpness;
        float side = source > 0
            ? Mathf.Max(source, Mathf.Floor(room * scale / source) * source / scale)
            : room;

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
    private void DrawBorder(in Rect2 full, in Color edge)
    {
        bool lit = _flashUntil > 0.0 || _glow > 0.5f;

        // Two pixels, drawn inside the bounds rather than centred on them -- Godot straddles the
        // rectangle it is given, so a two-pixel border on the outer edge would put one pixel of
        // every slot into its neighbour's gutter and shift the grid by half a pixel. One is what
        // revision two drew and it disappears at every scale.
        DrawRect(full.Grow(-Border / 2f), lit ? Style.SlotBorderHi : edge, filled: false, width: Border);
    }

    /// <summary>
    /// The slot's number, which moves depending on whether anything is in the slot.
    /// </summary>
    /// <remarks>
    /// Large and centred while the slot is empty, small in a corner once something is in it. An
    /// empty slot has nothing else to say, so the number can be the whole of it and be readable at
    /// a glance; a full one has artwork to show, and the number becomes a caption on it.
    ///
    /// That caption sits top right rather than top left, which is where revision two had it. Top
    /// left is the stack count's corner everywhere, and a number that means "press 3" reading in
    /// the same place as a number that means "there are three" is the kind of ambiguity you only
    /// notice once and then cannot stop noticing.
    /// </remarks>
    private void DrawNumber()
    {
        if (string.IsNullOrEmpty(Hotkey))
            return;

        if (_sprite.IsValid)
        {
            float tag = Style.Measure(Hotkey, Style.FontTag);
            this.DrawToken(
                new Vector2(Size.X - tag - 4f, Style.FontTag + 4f), Hotkey, Style.FontTag, Style.TextDim);
            return;
        }

        // The scale's own figure, unless the slot is too short to hold it.
        int size = Mathf.Min(Style.FontEmptySlot, (int)(Size.Y * 0.5f));
        float width = Style.Measure(Hotkey, size);
        float baseline = Style.BaselineIn(Size.Y, size);

        this.DrawToken(
            new Vector2(Mathf.Round((Size.X - width) / 2f), baseline), Hotkey, size, Style.SlotEmptyNumber);
    }

    /// <summary>What fires this slot, for the weapon and the ability.</summary>
    private void DrawMouseBind()
    {
        if (string.IsNullOrEmpty(BoundAction) || !_sprite.IsValid)
            return;

        float height = Mathf.Min(22f, Size.Y * 0.30f);

        if (App.KeyBindings.IsMouse(BoundAction))
        {
            var which = App.KeyBindings.MouseFor(BoundAction);
            if (which == MouseButton.Middle)
                return;

            var box = new Rect2(Size.X - height * 0.75f - 4f, 4f, height * 0.75f, height);
            HudIcons.MouseButton(this, box, Style.TextDim, which == MouseButton.Right);
            return;
        }

        // Bound to a key instead. Written out, in the corner the mouse would have been in.
        string name = App.KeyBindings.BoundTo(BoundAction);
        if (name == "None")
            return;

        float width = Style.Measure(name, Style.FontTag);
        this.DrawToken(new Vector2(Size.X - width - 4f, 4f + Style.FontTag), name,
            Style.FontTag, Style.TextDim);
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
        float baseline = Style.BaselineIn(Size.Y, Style.FontBody);

        this.DrawToken(
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

        this.DrawToken(
            new Vector2(Size.X - width - 3f, Size.Y - 4f), tag, Style.FontTag, Style.TierColour(tag));
    }
}

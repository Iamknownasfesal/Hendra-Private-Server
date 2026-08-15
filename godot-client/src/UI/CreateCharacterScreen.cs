using System;
using System.Collections.Generic;
using System.Linq;
using Godot;
using Hendra.Resources;

namespace Hendra.UI;

/// <summary>
/// The page a new character is rolled on: one class at a time, with everything known about it.
/// </summary>
/// <remarks>
/// <para>
/// A full screen rather than a grid of buttons. The original gives a class the whole page because
/// the page is where the decision is made: the sprite at size, the description in the class's own
/// words, what the next fame goal asks for, and the class quests already behind you.
/// </para>
/// <para>
/// The classes, their names, their sprites and their descriptions all come out of the game's own
/// object XML — see <see cref="GameData.PlayerClasses"/>. What this server has no record of is
/// said so rather than filled in: it keeps no exaltations, no seasonal characters and no skins, so
/// those parts stand at zero.
/// </para>
/// </remarks>
public partial class CreateCharacterScreen : Control
{
    private const float Margin = 144f;
    private const float LeftRight = 1280f;
    private const float RightLeft = 1290f;
    private const float RightRight = 1775f;

    private const float TabTop = 133f;
    private const float TabHeight = 58f;
    private const float TabFoot = 11f;

    private const float BodyTop = 191f;
    private const float BodyBottom = 836f;

    private const float StripTop = 873f;
    private const float StripHeight = 108f;
    private const float CardWidth = 126f;
    private const float CardGap = 6f;

    // Measured off references/Menu/New character UI.png. The greys the page is built from are one
    // step lighter than the panels' -- it is a page, not a plate on top of the world.
    private static readonly Color Field = new("2f2f2f");
    private static readonly Color BoxHeader = new("3b3b3b");
    private static readonly Color Dim = new("7b7c7d");
    private static readonly Color Rule = new("5b5b5b");
    private static readonly Color CellEdge = new("404040");
    private static readonly Color ChipFill = new("181818");
    private static readonly Color ChipEdge = new("464646");
    private static readonly Color QuestOn = new("d46522");
    private static readonly Color QuestOff = new("1b1a1a");
    private static readonly Color CardFill = new("323232");
    private static readonly Color CardFillOn = new("474747");
    private static readonly Color CardEdge = new("444444");
    private static readonly Color CardEdgeOn = new("ececec");
    private static readonly Color TabActiveEdge = new("696969");
    private static readonly Color TabIdleEdge = new("444444");
    private static readonly Color TabIdleFill = new("373737");
    private static readonly Color TabActiveFoot = new("3e3e3e");
    private static readonly Color TabIdleFoot = new("2e2e2e");
    private static readonly Color Exalted = new("0095ff");

    private readonly List<ObjectDesc> _classes = new();

    private DiamondField _background;
    private Overview _overview;
    private ClassStrip _strip;
    private GameChip _play;
    private GameChip _close;

    private CharacterRoster _roster;
    private int _selected;

    /// <summary>Raised with the class the player chose to roll.</summary>
    public event Action<ushort> PlayRequested;

    public event Action Closed;

    public bool IsOpen => Visible;

    public override void _Ready()
    {
        MouseFilter = MouseFilterEnum.Stop;
        Visible = false;

        var data = App.ServiceLocator.Data;
        if (data != null)
            _classes.AddRange(data.PlayerClasses);

        _background = new DiamondField();
        AddChild(_background);

        var title = new Label { Text = "Create New Character", VerticalAlignment = VerticalAlignment.Center }
            .Typeset(58, Style.Text);
        title.Position = new Vector2(Margin, 34f);
        title.Size = new Vector2(900f, 60f);
        AddChild(title);

        _close = new GameChip("Close", Style.ButtonDanger, Style.ButtonDangerHigh, Style.ButtonDanger.Darkened(0.3f), 28);
        _close.Pressed += Close;
        AddChild(_close);

        AddChild(new TabHead("Overview", true) { Position = new Vector2(Margin, TabTop), Size = new Vector2(567f, TabHeight) });
        AddChild(new TabHead("Skins", false) { Position = new Vector2(714f, TabTop), Size = new Vector2(567f, TabHeight) });

        _overview = new Overview
        {
            Position = new Vector2(Margin, BodyTop),
            Size = new Vector2(LeftRight - Margin, BodyBottom - BodyTop),
        };
        AddChild(_overview);

        _questBox = new QuestBox
        {
            Position = new Vector2(RightLeft, TabTop + 2f),
            Size = new Vector2(RightRight - RightLeft, 236f),
        };
        AddChild(_questBox);

        _exaltBox = new ExaltationBox
        {
            Position = new Vector2(RightLeft, 378f),
            Size = new Vector2(RightRight - RightLeft, BodyBottom - 378f),
        };
        AddChild(_exaltBox);

        _strip = new ClassStrip(_classes)
        {
            Position = new Vector2(Margin + 9f, StripTop),
            Size = new Vector2(LeftRight - Margin - 9f, StripHeight),
        };
        _strip.Chose += Select;
        AddChild(_strip);

        _play = new GameChip("Play", Style.ButtonCommit, Style.ButtonCommitHigh, Style.ButtonCommitLow, 34);
        _play.Pressed += () =>
        {
            if (_selected >= 0 && _selected < _classes.Count)
                PlayRequested?.Invoke(_classes[_selected].Type);
        };
        AddChild(_play);

        Resized += Reflow;
        Reflow();
        Select(0);
    }

    private QuestBox _questBox;
    private ExaltationBox _exaltBox;

    private void Reflow()
    {
        _background.Size = Size;

        _close.Position = new Vector2(1596f, 44f);
        _close.Size = new Vector2(180f, 54f);

        _play.Position = new Vector2(816f, 1003f);
        _play.Size = new Vector2(288f, 52f);
    }

    /// <summary>Hands the page the account's record, which is what fills the two right-hand boxes.</summary>
    public void Show(CharacterRoster roster)
    {
        _roster = roster;
        _strip.Roster = roster;
        Select(_selected);
    }

    public void Open()
    {
        Visible = true;
        Select(_selected);
    }

    public void Close()
    {
        if (!Visible)
            return;

        Visible = false;
        Closed?.Invoke();
    }

    public override void _UnhandledKeyInput(InputEvent @event)
    {
        if (Visible && @event is InputEventKey { Pressed: true, Keycode: Key.Escape })
        {
            Close();
            GetViewport().SetInputAsHandled();
        }
    }

    private void Select(int index)
    {
        if (_classes.Count == 0)
            return;

        _selected = Mathf.Clamp(index, 0, _classes.Count - 1);
        var chosen = _classes[_selected];

        _strip.Selected = _selected;
        _overview.Show(chosen, _roster);
        _questBox.Show(chosen, _roster);
        _exaltBox.Show(chosen);
    }

    /// <summary>How many of the class's five quests are done, and what the next one wants.</summary>
    private static (int Stars, int Goal, int Best) Quests(ObjectDesc desc, CharacterRoster roster)
    {
        if (roster == null || desc == null)
            return (0, CharacterRoster.ClassQuestGoals[0], 0);

        return (roster.ClassQuestStars(desc.Type), roster.NextClassQuestGoal(desc.Type),
            roster.HighestAliveFame.GetValueOrDefault(desc.Type));
    }

    // ---------------------------------------------------------------------------------------------
    // Background
    // ---------------------------------------------------------------------------------------------

    /// <summary>
    /// The near-black field with the pattern of diamonds drifting across it.
    /// </summary>
    /// <remarks>
    /// The original's menus all stand on this. It is deliberately almost invisible — a couple of
    /// points of luminance over the ground — so that it gives the page depth without competing with
    /// anything drawn on it.
    /// </remarks>
    private sealed partial class DiamondField : Control
    {
        private const float Pitch = 210f;
        private const float Speed = 5f;

        private float _drift;

        public DiamondField()
        {
            MouseFilter = MouseFilterEnum.Ignore;
            SetAnchorsPreset(LayoutPreset.TopLeft);
        }

        public override void _Process(double delta)
        {
            _drift = Mathf.PosMod(_drift + (float)delta * Speed, Pitch);
            QueueRedraw();
        }

        public override void _Draw()
        {
            DrawRect(new Rect2(Vector2.Zero, Size), new Color("131313"));

            var tint = new Color("1c1c1c");
            for (float y = -Pitch; y < Size.Y + Pitch; y += Pitch)
            {
                for (float x = -Pitch; x < Size.X + Pitch; x += Pitch)
                {
                    var centre = new Vector2(x + _drift, y + _drift * 0.5f);
                    float radius = Pitch * 0.34f;

                    DrawColoredPolygon(new[]
                    {
                        centre + new Vector2(0f, -radius),
                        centre + new Vector2(radius, 0f),
                        centre + new Vector2(0f, radius),
                        centre + new Vector2(-radius, 0f),
                    }, tint);
                }
            }

            // The frame ticks down the outer edges, which is the only ornament the page carries.
            for (int i = 0; i < 6; i++)
            {
                float y = 40f + i * (Size.Y - 120f) / 5f;
                DrawRect(new Rect2(14f, y, 5f, 46f), ChipEdge);
                DrawRect(new Rect2(Size.X - 19f, y, 5f, 46f), ChipEdge);
            }
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Controls
    // ---------------------------------------------------------------------------------------------

    /// <summary>A page tab, the same shape the characters panel uses.</summary>
    private sealed partial class TabHead : Control
    {
        private readonly string _text;
        private readonly bool _selected;

        public TabHead(string text, bool selected)
        {
            _text = text;
            _selected = selected;
            MouseFilter = MouseFilterEnum.Stop;
        }

        public override void _Draw()
        {
            var body = new Rect2(0f, 0f, Size.X, Size.Y - TabFoot);

            DrawRect(body, _selected ? TabActiveEdge : TabIdleEdge);
            DrawRect(body.Grow(-5f), _selected ? Style.TabActive : TabIdleFill);
            DrawRect(new Rect2(0f, Size.Y - TabFoot, Size.X, TabFoot),
                _selected ? TabActiveFoot : TabIdleFoot);

            this.DrawText(
                new Vector2(0f, Style.BaselineIn(Size.Y - TabFoot, 30)), _text, 30,
                _selected ? Style.TabActiveText : Style.StatLabel,
                alignment: HorizontalAlignment.Center, width: Size.X);
        }
    }

    /// <summary>A flat button with a bright top bevel and a dark lip: Close, and Play.</summary>
    private sealed partial class GameChip : Control
    {
        private readonly string _text;
        private readonly Color _face;
        private readonly Color _high;
        private readonly Color _low;
        private readonly int _size;

        public GameChip(string text, Color face, Color high, Color low, int size)
        {
            _text = text;
            _face = face;
            _high = high;
            _low = low;
            _size = size;
            MouseFilter = MouseFilterEnum.Stop;
        }

        public event Action Pressed;

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is InputEventMouseButton { Pressed: true, ButtonIndex: MouseButton.Left })
                Pressed?.Invoke();
        }

        public override void _Draw()
        {
            var full = new Rect2(Vector2.Zero, Size);

            DrawRect(full, _high);
            DrawRect(new Rect2(0f, Size.Y - 5f, Size.X, 5f), _low);
            DrawRect(full.Grow(-5f), _face);

            this.DrawText(
                new Vector2(0f, Style.BaselineIn(Size.Y, _size)), _text, _size, Style.Text,
                alignment: HorizontalAlignment.Center, width: Size.X);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // The overview column
    // ---------------------------------------------------------------------------------------------

    /// <summary>The class itself: its name, its sprite, its switches, and what it is for.</summary>
    private sealed partial class Overview : Control
    {
        private ObjectDesc _class;
        private CharacterRoster _roster;

        private Switch _seasonal;
        private Switch _upgrades;

        public Overview() => MouseFilter = MouseFilterEnum.Stop;

        public override void _Ready()
        {
            // Neither switch reaches the server: it has no seasonal characters and no class
            // upgrades. They are here because the page is the original's page, and they are the
            // only two things on it that do nothing.
            _seasonal = new Switch { Position = new Vector2(246f, 116f), Size = new Vector2(206f, 46f) };
            AddChild(_seasonal);

            _upgrades = new Switch { Position = new Vector2(246f, 231f), Size = new Vector2(206f, 46f) };
            AddChild(_upgrades);
        }

        public void Show(ObjectDesc chosen, CharacterRoster roster)
        {
            _class = chosen;
            _roster = roster;
            QueueRedraw();
        }

        public override void _Draw()
        {
            DrawRect(new Rect2(Vector2.Zero, Size), Field);

            if (_class == null)
                return;

            float centre = Size.X / 2f;
            string name = _class.DisplayId ?? _class.Id ?? "?";

            this.DrawText(new Vector2(0f, 60f), name, 52, Style.Text,
                alignment: HorizontalAlignment.Center, width: Size.X);

            // The sprite on its bracketed square, which is the page's one piece of artwork.
            var plate = new Rect2(457f, 68f, 223f, 223f);
            DrawRect(plate, new Color("434343"));
            Brackets(plate);

            var sprite = CharactersPanel.ClassSprite(_class.Type);
            if (sprite.IsValid)
                this.DrawSprite(sprite, plate.Grow(-46f), outline: 0f);

            this.DrawText(new Vector2(180f, 100f), "Seasonal", 38, Style.Text,
                alignment: HorizontalAlignment.Center, width: 138f);
            this.DrawText(new Vector2(180f, 215f), "Upgrades", 38, Style.Text,
                alignment: HorizontalAlignment.Center, width: 138f);

            if (!string.IsNullOrEmpty(_class.Description))
                this.DrawText(new Vector2(0f, 350f), _class.Description, 30, Dim,
                    alignment: HorizontalAlignment.Center, width: Size.X);

            Divider(centre, 391f);

            var (_, goal, _) = Quests(_class, _roster);

            this.DrawText(new Vector2(0f, 430f), "Next Fame Goal:", 30, QuestOn,
                alignment: HorizontalAlignment.Center, width: Size.X);

            string fameLine = goal > 0
                ? $"Earn {goal:N0} Fame with a {name}"
                : $"Every class quest for the {name} is complete";

            this.DrawText(new Vector2(0f, 462f), fameLine, 30, Dim,
                alignment: HorizontalAlignment.Center, width: Size.X);

            this.DrawText(new Vector2(0f, 509f), "Next Exaltation:", 30, Exalted,
                alignment: HorizontalAlignment.Center, width: Size.X);

            // Said plainly rather than invented: nothing on this server grants exaltations, so there
            // is no dungeon to name and no count to give.
            this.DrawText(new Vector2(0f, 541f), "This realm grants no exaltations", 30, Dim,
                alignment: HorizontalAlignment.Center, width: Size.X);
        }

        /// <summary>The rule under the description, broken by a tick in the middle.</summary>
        private void Divider(float centre, float y)
        {
            DrawRect(new Rect2(centre - 297f, y, 258f, 5f), Rule);
            DrawRect(new Rect2(centre + 39f, y, 258f, 5f), Rule);
            DrawRect(new Rect2(centre - 16f, y - 5f, 5f, 15f), Rule);
            DrawRect(new Rect2(centre + 11f, y - 5f, 5f, 15f), Rule);
        }

        private void Brackets(in Rect2 box)
        {
            const float arm = 26f;
            const float inset = 8f;

            for (int corner = 0; corner < 4; corner++)
            {
                bool right = corner is 1 or 2;
                bool bottom = corner is 2 or 3;

                float x = right ? box.End.X - inset : box.Position.X + inset;
                float y = bottom ? box.End.Y - inset : box.Position.Y + inset;

                DrawLine(new Vector2(x, y), new Vector2(x + (right ? -arm : arm), y), Field, 5f);
                DrawLine(new Vector2(x, y), new Vector2(x, y + (bottom ? -arm : arm)), Field, 5f);
            }
        }
    }

    /// <summary>A two-position switch, stuck on. See <see cref="Overview"/> for why.</summary>
    private sealed partial class Switch : Control
    {
        public Switch() => MouseFilter = MouseFilterEnum.Stop;

        public override void _Draw()
        {
            DrawRect(new Rect2(Vector2.Zero, Size), new Color("313131"));
            DrawRect(new Rect2(5f, 5f, Size.X - 10f, Size.Y - 10f), TabIdleFill);

            var pill = new Rect2(Size.X / 2f, 0f, Size.X / 2f, Size.Y);
            DrawRect(pill, Style.ToggleOnHigh);
            DrawRect(pill.Grow(-5f), Style.ToggleOn);

            this.DrawText(
                new Vector2(pill.Position.X, Style.BaselineIn(Size.Y, 30)), "ON", 30, Style.Text,
                alignment: HorizontalAlignment.Center, width: pill.Size.X);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // The right-hand boxes
    // ---------------------------------------------------------------------------------------------

    /// <summary>The class quest ladder: five rungs, a bar under them, and the fame that fills it.</summary>
    private sealed partial class QuestBox : Control
    {
        private ObjectDesc _class;
        private CharacterRoster _roster;

        /// <summary>The reference's own segment widths, which grow with the fame each rung asks.</summary>
        private static readonly float[] Weights = { 56f, 67f, 78f, 101f, 112f };

        public QuestBox() => MouseFilter = MouseFilterEnum.Stop;

        public void Show(ObjectDesc chosen, CharacterRoster roster)
        {
            _class = chosen;
            _roster = roster;
            QueueRedraw();
        }

        public override void _Draw()
        {
            DrawRect(new Rect2(0f, 0f, Size.X, 43f), BoxHeader);
            DrawRect(new Rect2(0f, 43f, Size.X, Size.Y - 43f), Field);

            this.DrawText(new Vector2(15f, 31f), "Class Quests", 30, Style.Text);

            var (stars, _, alive) = Quests(_class, _roster);
            int best = _roster?.BestFame.GetValueOrDefault(_class?.Type ?? 0) ?? 0;

            // The five rungs, each with the fame it asks for under it.
            float span = Size.X - 58f;
            for (int i = 0; i < CharacterRoster.ClassQuestGoals.Length; i++)
            {
                float x = 29f + span * (i + 0.5f) / 5f;
                bool done = i < stars;

                CharactersPanel.DrawStar(
                    this, new Rect2(x - 14f, 63f, 28f, 28f), done ? QuestOn : QuestOff, true);

                this.DrawText(
                    new Vector2(x - 40f, 112f), $"{CharacterRoster.ClassQuestGoals[i]}", 30,
                    done ? QuestOn : Dim, alignment: HorizontalAlignment.Center, width: 80f);
            }

            Bar(best, stars);

            this.DrawText(new Vector2(0f, 168f), "Highest Alive Fame Achieved", 28, Dim,
                alignment: HorizontalAlignment.Center, width: Size.X);

            this.DrawText(new Vector2(0f, 201f), $"{alive:N0}", 34, Style.Text,
                alignment: HorizontalAlignment.Center, width: Size.X - 44f);

            HudIcons.Fame(this, new Rect2(Size.X / 2f + 28f, 180f, 24f, 24f), Style.IconFame);
        }

        /// <summary>The segmented bar, filled up to the rung the class has actually reached.</summary>
        private void Bar(int best, int stars)
        {
            float total = Weights.Sum();
            float span = Size.X - 58f - 4f * 3f;
            float x = 29f;

            for (int i = 0; i < Weights.Length; i++)
            {
                float width = span * Weights[i] / total;
                var segment = new Rect2(x, 120f, width, 12f);

                DrawRect(segment, QuestOff);

                float fraction = i < stars
                    ? 1f
                    : i == stars ? Progress(best, i) : 0f;

                if (fraction > 0f)
                    DrawRect(new Rect2(segment.Position, new Vector2(width * fraction, 12f)), QuestOn);

                x += width + 3f;
            }
        }

        /// <summary>How far into the given rung the account's best fame has got.</summary>
        private static float Progress(int best, int rung)
        {
            int floor = rung == 0 ? 0 : CharacterRoster.ClassQuestGoals[rung - 1];
            int ceiling = CharacterRoster.ClassQuestGoals[rung];
            return Mathf.Clamp((best - floor) / (float)(ceiling - floor), 0f, 1f);
        }
    }

    /// <summary>
    /// The exaltation board, which this server keeps nothing for.
    /// </summary>
    /// <remarks>
    /// Every number here is zero because zero is the truth: there is no exaltation table in the
    /// database, no dungeon that grants one, and no packet that carries one. Drawing the board with
    /// its counters at nothing is the honest version of a page the original fills in.
    /// </remarks>
    private sealed partial class ExaltationBox : Control
    {
        private static readonly string[] Attributes = { "ATT", "DEF", "SPD", "DEX", "VIT", "WIS", "HP", "MP" };

        private ObjectDesc _class;

        public ExaltationBox() => MouseFilter = MouseFilterEnum.Stop;

        public void Show(ObjectDesc chosen)
        {
            _class = chosen;
            QueueRedraw();
        }

        public override void _Draw()
        {
            DrawRect(new Rect2(0f, 0f, Size.X, 43f), BoxHeader);
            DrawRect(new Rect2(0f, 43f, Size.X, Size.Y - 43f), Field);

            this.DrawText(new Vector2(15f, 31f), "Exaltations", 30, Style.Text);
            this.DrawText(new Vector2(0f, 31f), "0/40", 30, Exalted,
                alignment: HorizontalAlignment.Right, width: Size.X - 40f);

            string name = _class?.DisplayId ?? _class?.Id ?? "Class";

            // The three headings the original puts over the four bonus chips: the class, the armour
            // it wears and the weapon it carries.
            Heading(name, 46f, 130f);
            Heading("Armour", 240f, 110f);
            Heading("Weapon", 360f, 110f);

            string[] chips = { "+0%\nHP", "+0%\nDMG", "0s\nIC", "+0%\nDR" };
            for (int i = 0; i < chips.Length; i++)
            {
                var box = new Rect2(46f + i * 94f, 81f, 78f, 78f);
                DrawRect(box, ChipEdge);
                DrawRect(box.Grow(-4f), ChipFill);

                string[] lines = chips[i].Split('\n');
                this.DrawText(new Vector2(box.Position.X, box.Position.Y + 34f), lines[0], 26, Style.Text,
                    alignment: HorizontalAlignment.Center, width: box.Size.X);
                this.DrawText(new Vector2(box.Position.X, box.Position.Y + 60f), lines[1], 22, Style.StatValue,
                    alignment: HorizontalAlignment.Center, width: box.Size.X);
            }

            // Eight attributes in two columns, each at nothing out of five.
            for (int i = 0; i < Attributes.Length; i++)
            {
                float x = 42f + i % 2 * 208f;
                float y = 187f + i / 2 * 68f;
                var cell = new Rect2(x, y, 196f, 58f);

                DrawRect(cell, CellEdge, filled: false, width: 4f);
                this.DrawText(new Vector2(cell.Position.X, y + 26f), Attributes[i], 24, Dim,
                    alignment: HorizontalAlignment.Center, width: cell.Size.X);
                this.DrawText(new Vector2(cell.Position.X, y + 51f), "0/5", 28, Style.Text,
                    alignment: HorizontalAlignment.Center, width: cell.Size.X);
            }
        }

        private void Heading(string text, float x, float width) =>
            this.DrawText(new Vector2(x, 68f), text, 26, Dim,
                alignment: HorizontalAlignment.Center, width: width);
    }

    // ---------------------------------------------------------------------------------------------
    // The class strip
    // ---------------------------------------------------------------------------------------------

    /// <summary>Every class in a row along the bottom, with the chosen one framed in white.</summary>
    private sealed partial class ClassStrip : Control
    {
        private readonly List<ObjectDesc> _classes;
        private float _offset;
        private int _selected;

        public ClassStrip(List<ObjectDesc> classes)
        {
            _classes = classes;
            MouseFilter = MouseFilterEnum.Stop;
            ClipContents = true;
        }

        public event Action<int> Chose;

        public CharacterRoster Roster { get; set; }

        public int Selected
        {
            get => _selected;
            set
            {
                _selected = value;
                Follow();
                QueueRedraw();
            }
        }

        /// <summary>Keeps the chosen card on screen when the choice moves rather than the strip.</summary>
        private void Follow()
        {
            float left = _selected * (CardWidth + CardGap);
            float right = left + CardWidth;

            _offset = Mathf.Clamp(_offset, right - Size.X, left);
            _offset = Mathf.Clamp(_offset, 0f, Mathf.Max(0f, _classes.Count * (CardWidth + CardGap) - Size.X));
        }

        public override void _GuiInput(InputEvent @event)
        {
            if (@event is not InputEventMouseButton { Pressed: true } click)
                return;

            if (click.ButtonIndex is MouseButton.WheelDown or MouseButton.WheelUp)
            {
                _offset = Mathf.Clamp(
                    _offset + (click.ButtonIndex == MouseButton.WheelDown ? 132f : -132f),
                    0f, Mathf.Max(0f, _classes.Count * (CardWidth + CardGap) - Size.X));

                QueueRedraw();
                return;
            }

            if (click.ButtonIndex != MouseButton.Left)
                return;

            int index = (int)((click.Position.X + _offset) / (CardWidth + CardGap));
            if (index >= 0 && index < _classes.Count)
                Chose?.Invoke(index);
        }

        public override void _Draw()
        {
            // The bar above the strip, which the original always draws.
            float content = _classes.Count * (CardWidth + CardGap);
            float thumb = content <= Size.X ? Size.X : Size.X * Size.X / content;
            float travel = content <= Size.X ? 0f : _offset / (content - Size.X) * (Size.X - thumb);

            DrawRect(new Rect2(travel, -18f, thumb, 9f), new Color("666666"));

            for (int i = 0; i < _classes.Count; i++)
            {
                float x = i * (CardWidth + CardGap) - _offset;
                if (x > Size.X || x + CardWidth < 0f)
                    continue;

                Card(_classes[i], new Rect2(x, 0f, CardWidth, Size.Y), i == _selected);
            }
        }

        private void Card(ObjectDesc desc, in Rect2 box, bool chosen)
        {
            DrawRect(box, chosen ? CardEdgeOn : CardEdge);
            DrawRect(box.Grow(-6f), chosen ? CardFillOn : CardFill);

            var sprite = CharactersPanel.ClassSprite(desc.Type);
            if (sprite.IsValid)
                this.DrawSprite(sprite, new Rect2(box.Position.X + 30f, box.Position.Y + 10f, 66f, 46f), outline: 0f);

            this.DrawText(
                new Vector2(box.Position.X, box.Position.Y + 79f), desc.DisplayId ?? desc.Id ?? "?", 26,
                Style.Text, alignment: HorizontalAlignment.Center, width: box.Size.X);

            int stars = Roster?.ClassQuestStars(desc.Type) ?? 0;
            for (int i = 0; i < 5; i++)
                CharactersPanel.DrawStar(
                    this,
                    new Rect2(box.Position.X + 20f + i * 18f, box.Position.Y + 86f, 16f, 16f),
                    i < stars ? QuestOn : QuestOff, true);
        }
    }
}

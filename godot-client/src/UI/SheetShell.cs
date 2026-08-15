using Godot;

namespace Hendra.UI;

/// <summary>
/// The chrome the character sheet and the account sheet share.
/// </summary>
/// <remarks>
/// <para>
/// The reference's sheets carry no frame in a second colour and no close cross. A sheet is a plate
/// of the interface's own grey behind a thin light edge, with a notch taken out of each corner and
/// a thicker bracket along the inside of the notch. What says "this opened" is the edge and the
/// step, not a border in gold.
/// </para>
/// <para>
/// Drawn here rather than in <see cref="ModalPanel"/> because that shell is shared with the vault
/// and the trade window, which are not being rebuilt against this reference in the same round. It
/// still inherits from it, so Escape, the body rectangle and the open and close plumbing are the
/// ones every other panel uses.
/// </para>
/// </remarks>
public partial class SheetShell : ModalPanel
{
    /// <summary>How much is taken out of each corner.</summary>
    private const float Cut = 8f;

    private const float Edge = 3f;

    /// <summary>The bracket along the inside of a notch, which is thicker than the edge.</summary>
    private const float Tick = 4f;

    /// <summary>The title band, which is deep enough for a display-sized heading.</summary>
    public const float BandHeight = 75f;

    private const float TitleBaseline = 50f;

    /// <summary>The interface's title size is half a display size on a sheet this wide.</summary>
    private const int FontTitle = 40;

    private readonly string _heading;

    /// <param name="title">Drawn rather than set as a label, so it can be set at its own size.</param>
    public SheetShell(string title) : base(string.Empty) => _heading = title;

    protected override float Header => BandHeight;

    protected override bool ShowClose => false;

    public override void _Draw()
    {
        var tall = new Rect2(0f, Cut, Size.X, Size.Y - Cut * 2f);
        var wide = new Rect2(Cut, 0f, Size.X - Cut * 2f, Size.Y);

        DrawRect(tall, Style.ModalFrame);
        DrawRect(wide, Style.ModalFrame);

        var insideTall = new Rect2(
            Edge, Cut + Tick, Size.X - Edge * 2f, Size.Y - (Cut + Tick) * 2f);

        var insideWide = new Rect2(
            Cut + Tick, Edge, Size.X - (Cut + Tick) * 2f, Size.Y - Edge * 2f);

        DrawRect(insideTall, Style.ModalBand);
        DrawRect(insideWide, Style.ModalBand);

        // The title band, which ends on the row the body's first begins under.
        float band = Edge + Header + 1f;

        DrawRect(
            new Rect2(insideTall.Position, new Vector2(insideTall.Size.X, band - insideTall.Position.Y)),
            Style.ModalHeader);

        DrawRect(
            new Rect2(insideWide.Position, new Vector2(insideWide.Size.X, band - insideWide.Position.Y)),
            Style.ModalHeader);

        this.DrawText(
            new Vector2(Size.X / 2f - Style.Measure(_heading, FontTitle) / 2f, TitleBaseline),
            _heading, FontTitle, Style.Text);
    }
}

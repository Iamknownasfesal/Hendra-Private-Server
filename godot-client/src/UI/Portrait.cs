using Godot;

namespace Hendra.UI;

/// <summary>A character's sprite in a two-pixel frame, which is where the state colour goes.</summary>
public sealed partial class Portrait : Control
{
    private Assets.Sprite _sprite;
    private Color _border = Style.StatusOk;

    public Portrait()
    {
        MouseFilter = MouseFilterEnum.Ignore;
    }

    public void Set(Assets.Sprite sprite, Color border)
    {
        if (_border == border && _sprite.Sheet == sprite.Sheet && _sprite.Region == sprite.Region)
            return;

        _sprite = sprite;
        _border = border;
        QueueRedraw();
    }

    public override void _Draw()
    {
        var full = new Rect2(Vector2.Zero, Size);

        DrawRect(full, Style.Slot);

        if (_sprite.IsValid)
            DrawTextureRectRegion(_sprite.Sheet, full.Grow(-2f), _sprite.Region);

        DrawRect(full, _border, filled: false, width: 2f);
    }
}

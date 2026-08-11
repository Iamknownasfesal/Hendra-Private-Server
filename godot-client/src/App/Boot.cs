using System;
using Godot;
using Hendra.Assets;
using Hendra.Core;

namespace Hendra.App;

/// <summary>
/// The first scene. For now it is a smoke screen: it proves the extracted assets are present and
/// parseable before anything tries to use them, and reports what it found.
/// </summary>
/// <remarks>
/// This is where the title screen and the login flow will live. Until then, failing loudly here is
/// worth more than failing later — a missing manifest otherwise surfaces as an object with no
/// sprite, halfway into a dungeon.
/// </remarks>
public partial class Boot : Control
{
    private Label _status;

    public override void _Ready()
    {
        _status = new Label
        {
            AutowrapMode = TextServer.AutowrapMode.WordSmart,
            VerticalAlignment = VerticalAlignment.Center,
            HorizontalAlignment = HorizontalAlignment.Center,
        };
        _status.SetAnchorsPreset(LayoutPreset.FullRect);
        AddChild(_status);

        try
        {
            var manifest = AssetManifest.Load();
            _status.Text = string.Join('\n', new[]
            {
                "Hendra client",
                $"build {Net.ProtocolKeys.BuildVersion}",
                "",
                $"{manifest.Sheets.Count} spritesheets",
                $"{manifest.AnimatedChars.Count} character sheets",
                $"{manifest.Models.Count} models",
                $"{manifest.Xml.Objects.Count} object XML files "
                + $"({manifest.Xml.BaseObjectCount} base, {manifest.Xml.Objects.Count - manifest.Xml.BaseObjectCount} dungeon)",
                $"{manifest.Xml.Ground.Count} ground XML files",
            });
            GD.Print($"[boot] assets ok: {manifest.Sheets.Count} sheets, {manifest.Models.Count} models.");
        }
        catch (Exception ex)
        {
            _status.Text = $"Asset load failed.\n\n{ex.Message}";
            GD.PushError($"[boot] {ex.Message}");
        }
    }
}

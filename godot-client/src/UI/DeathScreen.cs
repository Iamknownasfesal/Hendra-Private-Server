using System;
using System.Collections.Generic;
using Godot;
using Hendra.Account;
using Hendra.Net.Packets;
using System.Globalization;
using Hendra.Resources;

namespace Hendra.UI;

/// <summary>
/// What a character did with its life, shown once it is over.
/// </summary>
/// <remarks>
/// <para>
/// The tally comes from <c>/char/fame</c>, which needs the account and character ids — and those
/// arrive on the Death packet itself. That is the only place the client learns its own numeric
/// account id when it has connected straight to a world rather than through the character list, so
/// this screen is fed the packet rather than a formatted message.
/// </para>
/// <para>
/// The bonuses are worded server-side and shown as sent. The original re-derived their names from a
/// table it kept in step by hand, which meant a server that added a bonus showed a blank line.
/// </para>
/// </remarks>
public partial class DeathScreen : Control
{
    private const int PanelWidth = 520;

    private VBoxContainer _column;
    private Label _heading;
    private Label _subheading;
    private Label _status;
    private VBoxContainer _bonuses;
    private GridContainer _tallies;
    private Label _total;

    private GameData _data;

    /// <summary>The fetched tally, waiting to be applied on the main thread.</summary>
    private FameResult _fame;

    /// <summary>Raised when the player is done reading. The caller returns to the login screen.</summary>
    public event Action Dismissed;

    public override void _Ready()
    {
        this.FillScreen();
        App.ServiceLocator.Audio?.PlayEffect("death_screen");

        // A dim wash over whatever the world last drew, so the panel reads as sitting on top of the
        // game rather than replacing it.
        var backdrop = new ColorRect { Color = new Color(0f, 0f, 0f, 0.72f) };
        backdrop.SetAnchorsPreset(LayoutPreset.FullRect);
        AddChild(backdrop);

        var plate = Style.Plate(Style.Panel, Style.PanelEdge);
        plate.ContentMarginLeft = 18;
        plate.ContentMarginRight = 18;
        plate.ContentMarginTop = 18;
        plate.ContentMarginBottom = 18;

        var panel = new PanelContainer();
        panel.AddThemeStyleboxOverride("panel", plate);
        panel.SetAnchorsPreset(LayoutPreset.Center);
        panel.OffsetLeft = -PanelWidth / 2f;
        panel.OffsetRight = PanelWidth / 2f;
        panel.OffsetTop = -240;
        panel.OffsetBottom = 240;
        AddChild(panel);

        // The plate carries its own padding, so the inner margin box the cut-cornered panel needed
        // is gone with it.
        _column = new VBoxContainer();
        _column.AddThemeConstantOverride("separation", 10);
        panel.AddChild(_column);

        _heading = new Label { Text = "You have died" }
            .Typeset(Style.FontTitle, Style.Text);
        _column.AddChild(_heading);

        _subheading = new Label { AutowrapMode = TextServer.AutowrapMode.WordSmart }
            .Typeset(Style.FontBody, Style.Text);
        _column.AddChild(_subheading);

        _total = new Label().Typeset(Style.FontName, Style.FameFill);
        _column.AddChild(_total);

        _status = new Label { Text = "Tallying…" }.Typeset(Style.FontBody, Style.TextDim);
        _column.AddChild(_status);

        _bonuses = new VBoxContainer();
        _bonuses.AddThemeConstantOverride("separation", 2);
        _column.AddChild(_bonuses);

        _tallies = new GridContainer { Columns = 4 };
        _tallies.AddThemeConstantOverride("h_separation", 12);
        _tallies.AddThemeConstantOverride("v_separation", 2);
        _column.AddChild(_tallies);

        var continueButton = new GameButton("Continue", primary: true);
        continueButton.Pressed += () => Dismissed?.Invoke();
        _column.AddChild(continueButton);

        // Focused so the screen can be dismissed from the keyboard, which is where the player's
        // hands already are.
        continueButton.CallDeferred(Control.MethodName.GrabFocus);
    }

    /// <summary>
    /// Shows what is known immediately, then fills in the tally when the app server answers.
    /// </summary>
    /// <param name="appServerUrl">Base URL of the app server, e.g. <c>http://host:8888</c>.</param>
    public void Show(DeathPacket death, string appServerUrl, GameData data, int level)
    {
        _data = data;

        string killer = string.IsNullOrEmpty(death.KilledBy) ? "something" : death.KilledBy;
        _subheading.Text = $"Killed by {killer} at level {level}.";

        _ = LoadFameAsync(death, appServerUrl);
    }

    private async System.Threading.Tasks.Task LoadFameAsync(DeathPacket death, string appServerUrl)
    {
        try
        {
            using var client = new AppEngineClient(appServerUrl);
            string xml = await client.PostAsync("/char/fame", new Dictionary<string, string>
            {
                ["accountId"] = death.AccountId ?? string.Empty,
                ["charId"] = death.CharId.ToString(),
            });

            // Handed over through a field rather than as an argument: the deferred call marshals
            // through Variant, which has no way to carry a plain C# object.
            _fame = FameResult.Parse(xml);
            CallDeferred(nameof(Apply));
        }
        catch (Exception ex)
        {
            // Worth showing rather than swallowing: the character is still dead either way, but a
            // blank screen looks like the client hung.
            CallDeferred(nameof(ShowError), ex.Message);
        }
    }

    private void ShowError(string message)
    {
        _status.Text = $"The tally could not be fetched — {message}";
        _status.Visible = true;
    }

    private void Apply()
    {
        var fame = _fame;
        if (fame == null)
            return;

        _status.Visible = false;

        string className = _data?.GetObject(fame.ObjectType)?.DisplayId
                           ?? _data?.GetObject(fame.ObjectType)?.Id;

        if (!string.IsNullOrEmpty(fame.CharacterName))
        {
            _heading.Text = string.IsNullOrEmpty(className)
                ? $"{fame.CharacterName} has died"
                : $"{fame.CharacterName} the {className} has died";
        }

        string killer = string.IsNullOrEmpty(fame.KilledBy) ? "something" : fame.KilledBy;
        _subheading.Text = $"Killed by {killer} at level {fame.Level}, with {Number(fame.Experience)} experience.";
        _total.Text = $"{Number(fame.TotalFame)} fame  ({Number(fame.BaseFame)} earned, " +
                      $"{Number(fame.TotalFame - fame.BaseFame)} in bonuses)";

        foreach (var bonus in fame.Bonuses)
        {
            var line = new Label
            {
                Text = $"{bonus.Description}  +{bonus.Fame}",
                AutowrapMode = TextServer.AutowrapMode.WordSmart,
            };
            line.Typeset(Style.FontBody, Style.StatBonus);
            _bonuses.AddChild(line);
        }

        AddTally("Monsters", fame.MonsterKills, "Assists", fame.MonsterAssists);
        AddTally("Gods", fame.GodKills, "God assists", fame.GodAssists);
        AddTally("Oryx", fame.OryxKills, "Cubes", fame.CubeKills);
        AddTally("Quests", fame.QuestsCompleted, "Abilities used", fame.SpecialAbilityUses);
        AddTally("Potions", fame.PotionsDrunk, "Teleports", fame.Teleports);
        AddTally("Tiles seen", fame.TilesUncovered, "Levels helped", fame.LevelUpAssists);
        AddTally("Shots", fame.Shots, "Accuracy", $"{fame.Accuracy * 100f:F0}%");
        AddTally("Minutes played", fame.MinutesActive, string.Empty, string.Empty);
    }

    private void AddTally(string leftName, int leftValue, string rightName, int rightValue) =>
        AddTally(leftName, leftValue, rightName, Number(rightValue));

    private void AddTally(string leftName, int leftValue, string rightName, string rightValue)
    {
        AddCell(leftName, dim: true);
        AddCell(Number(leftValue), dim: false);
        AddCell(rightName, dim: true);
        AddCell(rightValue, dim: false);
    }

    /// <summary>
    /// A grouped number.
    /// </summary>
    /// <remarks>
    /// Invariant rather than the machine's locale, because the interface is English throughout and a
    /// locale that groups with a full stop turns seventy-nine thousand tiles into seventy-nine.
    /// </remarks>
    private static string Number(int value) => value.ToString("N0", CultureInfo.InvariantCulture);

    private void AddCell(string text, bool dim)
    {
        var label = new Label { Text = text }
            .Typeset(Style.FontSmall, dim ? Style.StatLabel : Style.StatValue);
        _tallies.AddChild(label);
    }
}

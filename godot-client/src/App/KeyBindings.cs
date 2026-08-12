using System.Collections.Generic;
using Godot;

namespace Hendra.App;

/// <summary>
/// Every key the game answers to, and the machinery for changing them.
/// </summary>
/// <remarks>
/// <para>
/// The bindings themselves live in the project's input map, which is where the engine already looks
/// and where the defaults are written down. This adds the two things the input map has no opinion
/// about: what to call each action in front of a player, and how to remember that somebody changed
/// one. An override is stored as a keycode against the action name and pushed back into the input
/// map at startup, so the rest of the client goes on asking <c>Input.IsActionPressed</c> and never
/// learns that rebinding exists.
/// </para>
/// <para>
/// The original's options screen was mostly this: a list of actions, a key beside each, and a
/// widget for catching the next press. The grouping and the wording here follow it.
/// </para>
/// </remarks>
public static class KeyBindings
{
    /// <summary>One rebindable action.</summary>
    public readonly struct Binding
    {
        public readonly string Group;
        public readonly string Action;
        public readonly string Label;

        public Binding(string group, string action, string label)
        {
            Group = group;
            Action = action;
            Label = label;
        }
    }

    public const string Movement = "Movement";
    public const string General = "General";
    public const string Menus = "Menus";
    public const string Inventory = "Inventory";
    public const string Social = "Social";

    /// <summary>
    /// The catalogue, in the order and the wording the original uses.
    /// </summary>
    /// <remarks>
    /// A few of these have nothing behind them in this fork -- there are no summons to control, no
    /// emote wheel, no missions panel, no servers or legends screens. They are listed anyway, and
    /// hold their keys, so the layout matches the original and so the day any of them arrives the
    /// key it should be on is already the key players expect.
    /// </remarks>
    public static readonly Binding[] All =
    {
        new(Movement, "move_up", "Move Up"),
        new(Movement, "move_left", "Move Left"),
        new(Movement, "move_down", "Move Down"),
        new(Movement, "move_right", "Move Right"),
        new(Movement, "rotate_left", "Rotate Left"),
        new(Movement, "rotate_right", "Rotate Right"),
        new(Movement, "toggle_centering", "Toggle Centering of Player"),
        new(Movement, "reset_camera", "Reset to Default Camera Angle"),

        new(General, "autofire", "Autofire Toggle"),
        new(General, "shoot", "Attack"),
        new(General, "use_ability", "Use Special Ability"),
        new(General, "control_summons", "Control Summons"),
        new(General, "interact", "Interact/Buy"),
        new(General, "contextual_interaction", "Contextual Interaction"),
        new(General, "nexus", "Escape to Nexus"),
        new(General, "nexus_alt", "Escape to Nexus (second key)"),
        new(General, "switch_tabs", "Switch Tabs"),
        new(General, "toggle_hp_bars", "Toggle HP Bars"),
        new(General, "toggle_log", "Toggle Log Display"),
        new(General, "quick_slot_1", "Use Quick Slot 1"),
        new(General, "quick_slot_2", "Use Quick Slot 2"),
        new(General, "quick_slot_3", "Use Quick Slot 3"),
        new(General, "minimap_zoom_in", "Minimap Zoom In"),
        new(General, "minimap_zoom_out", "Minimap Zoom Out"),
        new(General, "camera_zoom_in", "Camera Zoom In"),
        new(General, "camera_zoom_out", "Camera Zoom Out"),
        new(General, "switch_ability", "Switching Abilities"),
        new(General, "toggle_hud", "Hide the interface"),
        new(General, "toggle_particles", "Particle effects"),
        new(General, "debug_overlay", "Show Debug Readout"),

        new(Menus, "options", "Show Options"),
        new(Menus, "show_servers", "Show Servers"),
        new(Menus, "show_legends", "Show Legends"),
        new(Menus, "character_panel", "Show Character Sheet"),

        new(Inventory, "inv_slot_1", "Use Inventory Slot 1"),
        new(Inventory, "inv_slot_2", "Use Inventory Slot 2"),
        new(Inventory, "inv_slot_3", "Use Inventory Slot 3"),
        new(Inventory, "inv_slot_4", "Use Inventory Slot 4"),
        new(Inventory, "inv_slot_5", "Use Inventory Slot 5"),
        new(Inventory, "inv_slot_6", "Use Inventory Slot 6"),
        new(Inventory, "inv_slot_7", "Use Inventory Slot 7"),
        new(Inventory, "inv_slot_8", "Use Inventory Slot 8"),

        new(Social, "toggle_chat", "Activate Chat"),
        new(Social, "chat_command", "Start Chat Command"),
        new(Social, "tell", "Begin Tell"),
        new(Social, "guild_chat", "Begin Guild Chat"),
        new(Social, "social_panel", "Show Social Panel"),
        new(Social, "scroll_chat_up", "Scroll Chat Up"),
        new(Social, "scroll_chat_down", "Scroll Chat Down"),
        new(Social, "emote_wheel", "Show Emote Wheel"),
        new(Social, "peek_missions", "Peek Current Missions"),
    };

    /// <summary>The keys each action started with, captured before anything overrides them.</summary>
    private static readonly Dictionary<string, Key> Defaults = new();

    /// <summary>
    /// Records the project's own bindings, then applies whatever the player has changed.
    /// </summary>
    /// <remarks>
    /// Must run before anything reads the input map, and exactly once: the defaults are read out of
    /// the map itself, so calling it after an override has been applied would record the override
    /// as the default and lose the way back.
    /// </remarks>
    public static void Apply(Settings settings)
    {
        if (Defaults.Count == 0)
        {
            foreach (var binding in All)
                Defaults[binding.Action] = FirstKey(binding.Action);
        }

        if (settings?.KeyOverrides == null)
            return;

        foreach (var (action, keycode) in settings.KeyOverrides)
            Bind(action, (Key)keycode, settings, save: false);
    }

    /// <summary>The key currently bound to an action, or None if it has none.</summary>
    public static Key KeyFor(string action) => FirstKey(action);

    /// <summary>
    /// What an action is bound to, written the way the options screen shows it.
    /// </summary>
    /// <remarks>
    /// Some of these sit on mouse buttons rather than keys. They are shown, so the screen tells the
    /// whole truth about the controls, but they cannot be rebound from here: catching a click to
    /// rebind inside a panel you click on is how a player loses the ability to shoot.
    /// </remarks>
    public static string BoundTo(string action)
    {
        if (!InputMap.HasAction(action))
            return "None";

        foreach (var stroke in InputMap.ActionGetEvents(action))
        {
            if (stroke is InputEventMouseButton mouse)
            {
                return mouse.ButtonIndex switch
                {
                    MouseButton.Left => "LMB",
                    MouseButton.Right => "RMB",
                    MouseButton.Middle => "MMB",
                    _ => "Mouse",
                };
            }
        }

        var key = FirstKey(action);
        return key == Key.None ? "None" : Name(key);
    }

    /// <summary>Which mouse button an action sits on, or None if it is on a key.</summary>
    public static MouseButton MouseFor(string action)
    {
        if (!InputMap.HasAction(action))
            return MouseButton.None;

        foreach (var stroke in InputMap.ActionGetEvents(action))
        {
            if (stroke is InputEventMouseButton mouse)
                return mouse.ButtonIndex;
        }

        return MouseButton.None;
    }

    /// <summary>Whether this action sits on a mouse button, and so cannot be rebound here.</summary>
    public static bool IsMouse(string action)
    {
        if (!InputMap.HasAction(action))
            return false;

        foreach (var stroke in InputMap.ActionGetEvents(action))
        {
            if (stroke is InputEventMouseButton)
                return true;
        }

        return false;
    }

    /// <summary>Whether an action has been moved off the key the game shipped with.</summary>
    public static bool IsChanged(string action) =>
        Defaults.TryGetValue(action, out var was) && was != FirstKey(action);

    /// <summary>
    /// Points an action at a key, taking it off anything else that was using it.
    /// </summary>
    /// <remarks>
    /// Two actions on one key is not an error the engine will report — it is a key that silently
    /// does two things — so the previous owner is unbound rather than left to collide.
    /// </remarks>
    public static void Bind(string action, Key key, Settings settings, bool save = true)
    {
        if (!InputMap.HasAction(action) || key == Key.None)
            return;

        foreach (var other in All)
        {
            if (other.Action != action && !IsMouse(other.Action) && FirstKey(other.Action) == key)
                Unbind(other.Action, settings);
        }

        InputMap.ActionEraseEvents(action);
        InputMap.ActionAddEvent(action, new InputEventKey { Keycode = key, PhysicalKeycode = 0 });

        if (settings == null)
            return;

        settings.KeyOverrides[action] = (int)key;

        if (save)
            settings.Save();
    }

    private static void Unbind(string action, Settings settings)
    {
        InputMap.ActionEraseEvents(action);

        // Remembered as "deliberately nothing" rather than dropped, or the next startup would put
        // the default back and recreate the clash the player just resolved.
        if (settings != null)
            settings.KeyOverrides[action] = (int)Key.None;
    }

    /// <summary>Puts every binding back to the key the game shipped with.</summary>
    public static void ResetAll(Settings settings)
    {
        foreach (var binding in All)
        {
            if (!Defaults.TryGetValue(binding.Action, out var key) || key == Key.None)
                continue;

            InputMap.ActionEraseEvents(binding.Action);
            InputMap.ActionAddEvent(binding.Action, new InputEventKey { Keycode = key });
        }

        settings?.KeyOverrides.Clear();
        settings?.Save();
    }

    /// <summary>A key's name as a player would write it, rather than as an enum.</summary>
    public static string Name(Key key) => key switch
    {
        Key.None => "—",
        Key.Space => "Space",
        Key.Enter => "Enter",
        Key.Escape => "Esc",
        Key.Tab => "Tab",
        Key.Pageup => "Page Up",
        Key.Pagedown => "Page Down",
        Key.Shift => "Shift",
        Key.Ctrl => "Ctrl",
        Key.Alt => "Alt",
        Key.Minus => "-",
        Key.Equal => "=",
        Key.Slash => "/",
        Key.Comma => ",",
        Key.Period => ".",
        Key.Semicolon => ";",
        Key.Apostrophe => "'",
        Key.Bracketleft => "[",
        Key.Bracketright => "]",
        Key.Backslash => "\\",
        _ => OS.GetKeycodeString(key),
    };

    private static Key FirstKey(string action)
    {
        if (!InputMap.HasAction(action))
            return Key.None;

        foreach (var stroke in InputMap.ActionGetEvents(action))
        {
            if (stroke is InputEventKey key)
                return key.Keycode != Key.None ? key.Keycode : key.PhysicalKeycode;
        }

        return Key.None;
    }
}

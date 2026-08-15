using System;
using System.Collections.Generic;
using System.Text.Json;

namespace Hendra.Text;

/// <summary>
/// Localised strings, keyed by identifier.
/// </summary>
/// <remarks>
/// Populated from the app server's <c>/app/getLanguageStrings</c>, which answers with an array of
/// key/value/family triples. An unknown key resolves to itself, which makes a missing translation
/// visible rather than silently blank.
/// </remarks>
public sealed class StringMap
{
    private readonly Dictionary<string, string> _values = new(StringComparer.Ordinal);

    public int Count => _values.Count;

    public void Set(string key, string value) => _values[key] = value;

    public bool Has(string key) => key != null && _values.ContainsKey(key);

    public string Get(string key)
    {
        if (key == null)
            return string.Empty;
        return _values.TryGetValue(key, out string value) ? value : key;
    }

    /// <summary>
    /// Loads the language table, which arrives as a JSON array of
    /// <c>[key, value, family]</c> triples.
    /// </summary>
    public void LoadFrom(string json)
    {
        // The shipped table begins with a byte order mark, and the app server serves the file
        // verbatim so that a deployment can drop the original's own copy in unchanged. A mark left
        // in front of the opening bracket is not valid JSON, and rejecting the whole table over it
        // leaves every line in the game rendering as the raw key it was looked up by.
        json = json.TrimStart('﻿');

        using var document = JsonDocument.Parse(json);
        if (document.RootElement.ValueKind != JsonValueKind.Array)
            return;

        foreach (var entry in document.RootElement.EnumerateArray())
        {
            if (entry.ValueKind != JsonValueKind.Array || entry.GetArrayLength() < 2)
                continue;

            string key = entry[0].GetString();
            string value = entry[1].GetString();
            if (key != null)
                _values[key] = value ?? string.Empty;
        }
    }
}

/// <summary>
/// Resolves the text the server sends, which is not always literal.
/// </summary>
/// <remarks>
/// <para>
/// Several packets — notifications, trade and guild results, purchase outcomes, failures, chat —
/// carry <em>either</em> a plain string <em>or</em> a JSON object naming a localisation key with
/// substitutions. The two are told apart by whether the string starts with a brace, which is the
/// whole disambiguation rule and the reason a literal message must never begin with one.
/// </para>
/// <para>
/// Substitution has a wrinkle worth knowing: a token's <em>value</em> may itself be a key, marked
/// by wrapping it in braces, in which case it is looked up before being substituted. That is how
/// the server sends "you need {item.health_potion}" without knowing the player's language.
/// </para>
/// </remarks>
public static class LineBuilder
{
    /// <summary>
    /// Resolves a string that may be a literal or a JSON localisation blob.
    /// </summary>
    public static string Resolve(string raw, StringMap strings)
    {
        if (string.IsNullOrEmpty(raw))
            return string.Empty;

        // Anything not starting with a brace is a literal and is passed through untouched.
        if (raw[0] != '{')
            return Unescape(raw);

        try
        {
            using var document = JsonDocument.Parse(raw);
            var root = document.RootElement;

            if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty("key", out var keyElement))
                return Unescape(raw);

            string key = keyElement.GetString();
            string text = strings?.Get(key) ?? key ?? string.Empty;

            if (root.TryGetProperty("tokens", out var tokens) && tokens.ValueKind == JsonValueKind.Object)
            {
                foreach (var token in tokens.EnumerateObject())
                    text = text.Replace("{" + token.Name + "}", ResolveToken(token.Value, strings));
            }

            return Unescape(text);
        }
        catch (JsonException)
        {
            // Malformed JSON is treated as a literal, matching the original's fallback. A chat line
            // that happens to start with a brace should still be readable.
            return Unescape(raw);
        }
    }

    private static string ResolveToken(JsonElement value, StringMap strings)
    {
        string text = value.ValueKind switch
        {
            JsonValueKind.String => value.GetString(),
            JsonValueKind.Number => value.ToString(),
            JsonValueKind.True => "true",
            JsonValueKind.False => "false",
            _ => value.ToString(),
        };

        if (string.IsNullOrEmpty(text))
            return string.Empty;

        // A brace-wrapped value is itself a key to look up.
        if (text.Length > 2 && text[0] == '{' && text[^1] == '}')
            return strings?.Get(text[1..^1]) ?? text;

        return text;
    }

    /// <summary>Turns the two-character escape the data uses into a real newline.</summary>
    private static string Unescape(string text) => text.Replace("\\n", "\n");
}

using Hendra.Text;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// The server-driven text contract.
///
/// Notifications, trade and guild results, purchase outcomes, failures and chat all carry either a
/// plain string or a JSON object naming a localisation key. Getting the disambiguation wrong shows
/// up as raw JSON in the chat log, or as a legitimate message being swallowed.
/// </summary>
public sealed class LineBuilderTests
{
    private static StringMap Strings()
    {
        var map = new StringMap();
        map.Set("server.quest_complete", "Quest complete!");
        map.Set("server.need_item", "You need {item} to continue.");
        map.Set("item.health_potion", "a Health Potion");
        map.Set("server.two", "{first} and {second}");
        return map;
    }

    [Fact]
    public void PlainTextPassesThrough()
    {
        Assert.Equal("hello there", LineBuilder.Resolve("hello there", Strings()));
    }

    [Fact]
    public void AKeyResolvesToItsText()
    {
        Assert.Equal("Quest complete!",
            LineBuilder.Resolve("{\"key\":\"server.quest_complete\"}", Strings()));
    }

    [Fact]
    public void TokensAreSubstituted()
    {
        Assert.Equal("You need 5 gold to continue.",
            LineBuilder.Resolve("{\"key\":\"server.need_item\",\"tokens\":{\"item\":\"5 gold\"}}", Strings()));
    }

    [Fact]
    public void ATokenValueMayItselfBeAKey()
    {
        // This is how the server names an item without knowing the player's language: the token's
        // value is brace-wrapped, marking it as another key to look up.
        Assert.Equal("You need a Health Potion to continue.",
            LineBuilder.Resolve(
                "{\"key\":\"server.need_item\",\"tokens\":{\"item\":\"{item.health_potion}\"}}",
                Strings()));
    }

    [Fact]
    public void SeveralTokensAreAllSubstituted()
    {
        Assert.Equal("swords and shields",
            LineBuilder.Resolve(
                "{\"key\":\"server.two\",\"tokens\":{\"first\":\"swords\",\"second\":\"shields\"}}",
                Strings()));
    }

    [Fact]
    public void AnUnknownKeyResolvesToItself()
    {
        // Deliberate: a missing translation should be visible rather than blank, so it can be
        // noticed and added.
        Assert.Equal("server.unknown",
            LineBuilder.Resolve("{\"key\":\"server.unknown\"}", Strings()));
    }

    [Fact]
    public void MalformedJsonIsTreatedAsLiteral()
    {
        // A chat line that happens to start with a brace must still be readable rather than
        // vanishing into a parse failure.
        const string text = "{not really json";
        Assert.Equal(text, LineBuilder.Resolve(text, Strings()));
    }

    [Fact]
    public void EscapedNewlinesBecomeRealOnes()
    {
        Assert.Equal("first\nsecond", LineBuilder.Resolve("first\\nsecond", Strings()));
    }

    [Fact]
    public void EmptyInputIsSafe()
    {
        Assert.Equal(string.Empty, LineBuilder.Resolve(null, Strings()));
        Assert.Equal(string.Empty, LineBuilder.Resolve(string.Empty, Strings()));
    }

    [Fact]
    public void ResolvingWithoutATableStillProducesSomething()
    {
        // The table arrives over HTTP after the session starts, so text can be resolved before it
        // exists. Keys showing through is the intended degradation.
        Assert.Equal("server.quest_complete",
            LineBuilder.Resolve("{\"key\":\"server.quest_complete\"}", null));
    }

    [Fact]
    public void TheLanguageTableParsesTheServersTripleFormat()
    {
        var map = new StringMap();
        map.LoadFrom("[[\"a.key\",\"A value\",\"en\"],[\"b.key\",\"B value\",\"en\"]]");

        Assert.Equal(2, map.Count);
        Assert.Equal("A value", map.Get("a.key"));
        Assert.Equal("B value", map.Get("b.key"));
    }
}

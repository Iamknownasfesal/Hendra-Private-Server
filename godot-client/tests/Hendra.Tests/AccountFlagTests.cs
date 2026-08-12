using Hendra.Account;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// The boolean flags on an account.
///
/// The server writes a false boolean as an *empty element* rather than leaving it out, so the tag
/// is present either way. Testing for presence reads every one of them as true — which stranded
/// players on the name page, since the only button there then failed: the server charges a thousand
/// credits to *change* a name and a new account has none.
/// </summary>
public sealed class AccountFlagTests
{
    private static string Response(string accountBody) =>
        $"<Chars nextCharId=\"0\" maxNumChars=\"2\"><Account>{accountBody}</Account></Chars>";

    [Fact]
    public void AnEmptyElementIsFalse()
    {
        var account = CharListResult.Parse(Response(
            "<Name>Someone</Name><NameChosen></NameChosen><Admin></Admin>")).Account;

        Assert.False(account.NameChosen);
        Assert.False(account.Admin);
    }

    [Fact]
    public void AMissingElementIsFalse()
    {
        var account = CharListResult.Parse(Response("<Name>Someone</Name>")).Account;

        Assert.False(account.NameChosen);
        Assert.False(account.VerifiedEmail);
    }

    [Theory]
    [InlineData("1")]
    [InlineData("true")]
    [InlineData("True")]
    public void AValueIsTrue(string written)
    {
        var account = CharListResult.Parse(Response(
            $"<Name>Someone</Name><NameChosen>{written}</NameChosen>")).Account;

        Assert.True(account.NameChosen);
    }
}

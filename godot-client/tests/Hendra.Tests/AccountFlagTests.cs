using Hendra.Account;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// The boolean flags on an account, and the guest account that arrives wearing a real one's clothes.
/// </summary>
/// <remarks>
/// The server writes these as <c>NameChosen ? new XElement("NameChosen", "") : null</c> — absent
/// when false, and present but *empty* when true. So the element's presence is the value and its
/// content is always the empty string; reading the content would call every one of them false.
/// </remarks>
public sealed class AccountFlagTests
{
    private static AccountInfo Parse(string accountBody) =>
        CharListResult.Parse(
            $"<Chars nextCharId=\"0\" maxNumChars=\"2\"><Account>{accountBody}</Account></Chars>").Account;

    [Fact]
    public void AnEmptyElementIsTrue()
    {
        var account = Parse("<Name>Someone</Name><NameChosen></NameChosen><Admin></Admin>");

        Assert.True(account.NameChosen);
        Assert.True(account.Admin);
    }

    [Fact]
    public void AMissingElementIsFalse()
    {
        var account = Parse("<Name>Someone</Name>");

        Assert.False(account.NameChosen);
        Assert.False(account.Admin);
        Assert.False(account.VerifiedEmail);
    }

    /// <summary>
    /// A brand-new registered account has a name already — the server assigns one off a fixed list —
    /// so the name cannot stand in for the flag. Only NameChosen says whether it was picked.
    /// </summary>
    [Fact]
    public void ANameDoesNotImplyItWasChosen()
    {
        var account = Parse("<AccountId>10</AccountId><Name>Tal</Name>");

        Assert.Equal("Tal", account.Name);
        Assert.False(account.NameChosen);
    }

    /// <summary>
    /// /char/list answers 200 for credentials it has never seen, building a guest on the spot:
    /// AccountId 0, a name off the same fixed list, no NameChosen. It is never persisted, so
    /// everything that checks credentials properly then refuses it.
    /// </summary>
    [Fact]
    public void TheGuestAccountIsRecognisableByItsId()
    {
        var account = Parse("<AccountId>0</AccountId><Name>Darq</Name>");

        Assert.Equal("0", account.AccountId);
    }
}

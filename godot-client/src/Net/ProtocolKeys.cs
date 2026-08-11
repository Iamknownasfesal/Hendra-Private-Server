using System;
using System.Security.Cryptography;
using System.Text;

namespace Hendra.Net;

/// <summary>
/// The fixed cryptographic material and version string the server expects.
/// </summary>
public static class ProtocolKeys
{
    /// <summary>
    /// Must match wServer.json serverSettings.version exactly. A mismatch is the worst failure mode
    /// in the protocol: HelloHandler returns without sending anything at all, so the client sees a
    /// connection that accepts Hello and then goes permanently silent.
    /// </summary>
    public const string BuildVersion = "Alpha v01";

    /// <summary>Reported to the HTTP app server as gameClientVersion.</summary>
    public const string HttpClientVersion = "Alpha.01";

    /// <summary>
    /// Client-to-server RC4 key. The server hex-decodes wServer.json serverSettings.key
    /// ("B1A5ED") via StringUtils.StringToByteArray and installs it as its ReceiveKey.
    /// </summary>
    public static ReadOnlySpan<byte> ClientToServerKey => new byte[] { 0xB1, 0xA5, 0xED };

    /// <summary>
    /// Server-to-client RC4 key, hardcoded as Client.ServerKey in wServer/networking/Client.cs.
    /// </summary>
    public static ReadOnlySpan<byte> ServerToClientKey => new byte[]
    {
        0x61, 0x2a, 0x80, 0x6c, 0xac, 0x78, 0x11, 0x4b, 0xa5, 0x01, 0x3c, 0xb5, 0x31
    };

    /// <summary>
    /// The public half of the 512-bit key whose private half lives in wServer/networking/RSA.cs.
    /// Only Hello's credential fields use it; the HTTP app server takes credentials in the clear.
    /// </summary>
    private const string PublicKeyPem =
        "-----BEGIN PUBLIC KEY-----\n" +
        "MFswDQYJKoZIhvcNAQEBBQADSgAwRwJAeyjMOLhcK4o2AnFRhn8vPteUy5Fux/cX\n" +
        "N/J+wT/zYIEUINo02frn+Kyxx0RIXJ3CvaHkwmueVL8ytfqo8Ol/OwIDAQAB\n" +
        "-----END PUBLIC KEY-----";

    private static readonly Lazy<RSA> Rsa = new(() =>
    {
        var rsa = RSA.Create();
        rsa.ImportFromPem(PublicKeyPem);
        return rsa;
    });

    /// <summary>
    /// Encrypts a credential the way Hello expects: RSA PKCS#1 v1.5, then base64. What travels on
    /// the wire is the base64 *text*, written with the usual int16-prefixed UTF-8 encoding.
    ///
    /// An empty or null input encodes to an empty string rather than to ciphertext, matching the
    /// short-circuit in the server's RSA.Encrypt/Decrypt. Hello.Secret is always empty.
    /// </summary>
    public static string EncryptCredential(string plaintext)
    {
        if (string.IsNullOrEmpty(plaintext))
            return string.Empty;

        byte[] cipher = Rsa.Value.Encrypt(Encoding.UTF8.GetBytes(plaintext), RSAEncryptionPadding.Pkcs1);
        return Convert.ToBase64String(cipher);
    }
}

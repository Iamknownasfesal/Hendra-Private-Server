using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Compression;
using System.Net;
using System.Net.Http;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace Hendra.Account;

/// <summary>Raised when the app server answers with an error document.</summary>
public sealed class AppEngineException : Exception
{
    /// <summary>
    /// Whether retrying could plausibly help. The server distinguishes <c>&lt;Error&gt;</c>, which
    /// is transient, from <c>&lt;FatalError&gt;</c>, which is not.
    /// </summary>
    public bool IsFatal { get; }

    public AppEngineException(string message, bool isFatal) : base(message)
    {
        IsFatal = isFatal;
    }
}

/// <summary>
/// Talks to the HTTP app server: accounts, character lists, the server list and bulk data.
/// </summary>
/// <remarks>
/// <para>
/// Every endpoint is a POST with a URL-encoded body, even the ones that only read. Responses are
/// raw-deflate compressed — the server writes them with .NET's <c>DeflateStream</c>, which produces
/// RFC 1951 with no zlib wrapper, despite labelling them <c>Content-Encoding: deflate</c>, which by
/// the letter of the HTTP spec means the wrapped form. Automatic decompression is therefore turned
/// off and the body is inflated explicitly, falling back to treating it as plain text.
/// </para>
/// <para>
/// Credentials go over this channel in the clear. Only the game socket's Hello encrypts them, and
/// only there because the server insists. Worth knowing before pointing this at anything across a
/// network you do not control.
/// </para>
/// </remarks>
public sealed class AppEngineClient : IDisposable
{
    private readonly HttpClient _http;

    public AppEngineClient(string baseUrl, TimeSpan? timeout = null)
    {
        BaseUrl = baseUrl.TrimEnd('/');

        _http = new HttpClient(new HttpClientHandler
        {
            // The bodies are raw deflate rather than the zlib form the header implies, so they are
            // inflated by hand below.
            AutomaticDecompression = DecompressionMethods.None,
        })
        {
            Timeout = timeout ?? TimeSpan.FromSeconds(20),
        };
    }

    public string BaseUrl { get; }

    /// <summary>
    /// Posts to <paramref name="path"/> and returns the decoded body.
    /// </summary>
    /// <remarks>
    /// Two parameters are added to every request, matching the original: a client version string,
    /// which the server may check, and a changing value that defeats any cache between here and
    /// there. When a <c>guid</c> is present it is also appended to the query string, which the
    /// original did so that request logs were readable.
    /// </remarks>
    public async Task<string> PostAsync(
        string path,
        IReadOnlyDictionary<string, string> parameters = null,
        CancellationToken cancellationToken = default)
    {
        var form = new Dictionary<string, string>(StringComparer.Ordinal);
        if (parameters != null)
        {
            foreach (var (key, value) in parameters)
                form[key] = value ?? string.Empty;
        }

        form["gameClientVersion"] = Net.ProtocolKeys.HttpClientVersion;
        form["ignore"] = Environment.TickCount.ToString();

        string url = BaseUrl + (path.StartsWith('/') ? path : "/" + path);
        if (form.TryGetValue("guid", out string guid) && !string.IsNullOrEmpty(guid))
            url += "?g=" + Uri.EscapeDataString(guid);

        using var content = new FormUrlEncodedContent(form);
        using var response = await _http.PostAsync(url, content, cancellationToken).ConfigureAwait(false);

        byte[] body = await response.Content.ReadAsByteArrayAsync(cancellationToken).ConfigureAwait(false);
        string text = Decode(body);

        ThrowIfError(text);
        return text;
    }

    /// <summary>
    /// Posts and returns the raw body, for the endpoints that answer with packed binary rather than
    /// XML — bulk textures and the additional XML bundle.
    /// </summary>
    public async Task<byte[]> PostBinaryAsync(
        string path,
        IReadOnlyDictionary<string, string> parameters = null,
        CancellationToken cancellationToken = default)
    {
        var form = new Dictionary<string, string>(StringComparer.Ordinal);
        if (parameters != null)
        {
            foreach (var (key, value) in parameters)
                form[key] = value ?? string.Empty;
        }

        form["gameClientVersion"] = Net.ProtocolKeys.HttpClientVersion;
        form["ignore"] = Environment.TickCount.ToString();

        string url = BaseUrl + (path.StartsWith('/') ? path : "/" + path);

        using var content = new FormUrlEncodedContent(form);
        using var response = await _http.PostAsync(url, content, cancellationToken).ConfigureAwait(false);

        byte[] body = await response.Content.ReadAsByteArrayAsync(cancellationToken).ConfigureAwait(false);
        return Inflate(body) ?? body;
    }

    /// <summary>Inflates the body if it is compressed, otherwise reads it as UTF-8.</summary>
    /// <remarks>
    /// The byte-order mark is stripped. Some of these responses are files served straight off disk
    /// and were saved with one; UTF8.GetString keeps it as a zero-width character, which is
    /// invisible in a log and makes a JSON parser reject the document at position zero.
    /// </remarks>
    private static string Decode(byte[] body)
    {
        byte[] bytes = Inflate(body) ?? body;

        if (bytes.Length >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF)
            return Encoding.UTF8.GetString(bytes, 3, bytes.Length - 3);

        return Encoding.UTF8.GetString(bytes);
    }

    /// <summary>
    /// Raw-deflate inflation, or null if the body is not compressed.
    /// </summary>
    /// <remarks>
    /// A few endpoints answer in plain text, and errors are always plain, so this has to tolerate
    /// being handed uncompressed input rather than assuming.
    /// </remarks>
    private static byte[] Inflate(byte[] body)
    {
        if (body == null || body.Length == 0)
            return null;

        try
        {
            using var source = new MemoryStream(body);
            using var deflate = new DeflateStream(source, CompressionMode.Decompress);
            using var destination = new MemoryStream();
            deflate.CopyTo(destination);
            return destination.ToArray();
        }
        catch (InvalidDataException)
        {
            return null;
        }
    }

    private static void ThrowIfError(string text)
    {
        if (string.IsNullOrEmpty(text))
            return;

        string trimmed = text.TrimStart();
        bool fatal = trimmed.StartsWith("<FatalError>", StringComparison.Ordinal);

        if (!fatal && !trimmed.StartsWith("<Error>", StringComparison.Ordinal))
            return;

        // The message is the element's text; the tags themselves carry nothing useful.
        int open = trimmed.IndexOf('>');
        int close = trimmed.LastIndexOf('<');
        string message = open >= 0 && close > open
            ? trimmed[(open + 1)..close]
            : trimmed;

        throw new AppEngineException(message, fatal);
    }

    public void Dispose() => _http.Dispose();
}

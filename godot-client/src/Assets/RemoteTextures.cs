using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Godot;
using Hendra.Account;
using Hendra.Net;
using Hendra.Resources;

namespace Hendra.Assets;

/// <summary>
/// The per-object artwork that is not shipped with the client, fetched from <c>/app/getTextures</c>.
/// </summary>
/// <remarks>
/// <para>
/// The response is one deflated blob holding every texture the server's game data refers to: a
/// count, then a name, a length and that many PNG bytes, repeated. Names ending <c>_mask</c> are
/// recolour masks for the texture of the same name.
/// </para>
/// <para>
/// The reference client has this path commented out — it sets a placeholder sprite and returns —
/// so every object with a remote texture renders as the same small box there. Fetching them costs
/// one request at startup and is what makes those objects look like themselves.
/// </para>
/// </remarks>
public static class RemoteTextures
{
    private const string MaskSuffix = "_mask";

    /// <summary>
    /// Fetches and registers everything the server has.
    /// </summary>
    /// <remarks>
    /// Deliberately not fatal, and deliberately not awaited by anything that matters. A failure
    /// costs the fifty-odd objects that use these their artwork, which is exactly what the
    /// reference client lives with permanently.
    /// </remarks>
    public static async Task LoadAsync(string baseUrl, AssetLibrary assets, GameData data)
    {
        try
        {
            using var client = new AppEngineClient(baseUrl);
            var payload = await client.PostBinaryAsync("/app/getTextures", null);

            var images = Decode(payload);
            if (images.Count == 0)
                return;

            // Decoding the blob is fine anywhere, but the textures themselves are engine resources
            // and are built on the main thread -- created on a worker they are reported as leaked
            // at exit, because nothing on that thread owns them.
            Callable.From(() => Register(images, assets, data)).CallDeferred();
        }
        catch (Exception ex)
        {
            GD.PushWarning($"[content] could not load remote textures: {ex.Message}");
        }
    }

    /// <summary>Splits the blob into named PNGs.</summary>
    private static Dictionary<string, byte[]> Decode(byte[] payload)
    {
        var images = new Dictionary<string, byte[]>();
        if (payload == null || payload.Length < 4)
            return images;

        var reader = new NetReader(payload);
        int count = reader.ReadInt32();

        // A wrong count means the response was not what we think it is; better to give up than to
        // read a length off the end of a buffer.
        if (count < 0 || count > 100_000)
            return images;

        for (int i = 0; i < count; i++)
        {
            string name = reader.ReadUtf();
            int length = reader.ReadInt32();
            if (length < 0 || length > payload.Length)
                break;

            images[name] = reader.ReadBytes(length);
        }

        return images;
    }

    /// <summary>
    /// Turns the PNGs into animated characters, one per id the game data actually asks for.
    /// </summary>
    /// <remarks>
    /// Driven from the object definitions rather than from the blob, because only the definitions
    /// say which way a given texture faces — and a strip built facing the wrong way mirrors every
    /// pose.
    /// </remarks>
    private static void Register(Dictionary<string, byte[]> images, AssetLibrary assets, GameData data)
    {
        var loaded = new Dictionary<string, Texture2D>();
        int registered = 0;

        foreach (var desc in data.Objects.Values)
        {
            var spec = desc.Texture;
            if (spec is not { Kind: TextureKind.Remote } || string.IsNullOrEmpty(spec.RemoteId))
                continue;

            var texture = Texture(images, loaded, spec.RemoteId);
            if (texture == null)
                continue;

            assets.AddRemoteTexture(
                spec.RemoteId,
                texture,
                Texture(images, loaded, spec.RemoteId + MaskSuffix),
                spec.RemoteFacesRight);

            registered++;
        }

        GD.Print($"[content] {images.Count} remote textures, used by {registered} objects.");
    }

    /// <summary>Decodes one PNG, once, however many objects share it.</summary>
    private static Texture2D Texture(
        Dictionary<string, byte[]> images, Dictionary<string, Texture2D> loaded, string name)
    {
        if (loaded.TryGetValue(name, out var cached))
            return cached;

        if (!images.TryGetValue(name, out var bytes))
            return null;

        var image = new Image();
        var result = image.LoadPngFromBuffer(bytes) == Error.Ok
            ? ImageTexture.CreateFromImage(image)
            : null;

        loaded[name] = result;
        return result;
    }
}

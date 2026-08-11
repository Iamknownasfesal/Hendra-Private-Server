using Hendra.Resources;

namespace Hendra.Assets;

/// <summary>
/// Artwork resolved from a <see cref="TextureSpec"/>: either a still sprite or an animation set.
/// </summary>
public readonly struct ResolvedTexture
{
    public readonly Sprite Still;
    public readonly AnimatedChar Animated;
    public readonly Sprite Mask;

    public ResolvedTexture(Sprite still, AnimatedChar animated, Sprite mask)
    {
        Still = still;
        Animated = animated;
        Mask = mask;
    }

    public bool IsValid => Animated != null || Still.IsValid;
}

/// <summary>
/// Turns the texture declarations parsed out of XML into actual sprites.
/// </summary>
/// <remarks>
/// Kept apart from the XML parsing so the data can be read without the engine, and so remote
/// textures — which arrive over HTTP after the world has already loaded — can be slotted in later
/// without re-parsing anything.
/// </remarks>
public sealed class TextureResolver
{
    private readonly AssetLibrary _assets;

    public TextureResolver(AssetLibrary assets)
    {
        _assets = assets;
    }

    /// <summary>
    /// Resolves a spec for a particular entity.
    /// </summary>
    /// <param name="spec">The declaration to resolve.</param>
    /// <param name="objectId">
    /// Used to pick among random variants. The original indexes by object id rather than drawing a
    /// random number, so a given object always looks the same — "random" here means varied across
    /// the map, not varied over time.
    /// </param>
    /// <param name="altTextureIndex">
    /// The AltTextureIndex stat, which swaps an object to a declared alternate. Zero means the
    /// default.
    /// </param>
    public ResolvedTexture Resolve(TextureSpec spec, int objectId = 0, int altTextureIndex = 0)
    {
        if (spec == null)
            return default;

        if (altTextureIndex != 0 && spec.Alternates != null &&
            spec.Alternates.TryGetValue(altTextureIndex, out var alternate))
        {
            spec = alternate;
        }

        if (spec.Kind == TextureKind.Random && spec.Variants is { Count: > 0 })
        {
            int index = objectId % spec.Variants.Count;
            if (index < 0)
                index += spec.Variants.Count;
            spec = spec.Variants[index];
        }

        var mask = spec.Mask != null ? _assets.GetSprite(spec.Mask.File, spec.Mask.Index) : default;

        return spec.Kind switch
        {
            TextureKind.Sprite => new ResolvedTexture(_assets.GetSprite(spec.File, spec.Index), null, mask),
            TextureKind.AnimatedChar => new ResolvedTexture(
                default, _assets.GetAnimatedChar(spec.File, spec.Index), mask),

            // Fetched from the app server at startup. When that failed, or the server has no
            // artwork under this id, the object falls back to the same placeholder box the
            // reference client shows -- which it shows for every one of them, since it has the
            // fetch commented out. Better a visible marker than an invisible object.
            TextureKind.Remote => ResolveRemote(spec, mask),

            _ => default,
        };
    }

    /// <summary>The sprite the reference client substitutes for a remote texture it cannot load.</summary>
    private const int PlaceholderIndex = 0xFF;

    private ResolvedTexture ResolveRemote(TextureSpec spec, Sprite mask)
    {
        var animated = _assets.GetRemoteTexture(spec.RemoteId);
        if (animated != null)
            return new ResolvedTexture(default, animated, mask);

        return new ResolvedTexture(_assets.GetSprite("lofiObj3", PlaceholderIndex), null, mask);
    }
}

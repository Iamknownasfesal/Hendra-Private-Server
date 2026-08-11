using Godot;

namespace Hendra.Render;

/// <summary>
/// Maps game coordinates onto the 3D scene so that Godot reproduces the original's projection
/// exactly.
/// </summary>
/// <remarks>
/// <para>
/// The AS3 camera looks like a perspective camera and is not one. It builds a
/// <c>PerspectiveProjection</c> and then never composes it into the world-to-screen matrix; what it
/// actually applies is
/// </para>
/// <code>
/// screenX     = 50 * ( (p - c) · r )              r = ( cos a,  sin a, 0)
/// screenYDown = 50 * ( (p - c) · u  -  height )   u = (-sin a,  cos a, 0)
/// </code>
/// <para>
/// That is an oblique (cavalier) projection: the ground plane is not foreshortened at all, and
/// height is added straight into screen Y at a one-to-one ratio. Tilting a camera to get the
/// familiar look would compress the ground along one axis and be visibly wrong, so instead the
/// camera looks straight down and the two unusual parts of the projection are handled explicitly.
/// </para>
/// <para><b>Height.</b> A camera pointing straight down has its "up" vector lying flat in the ground
/// plane. So displacing a sprite along that vector moves it up the screen without moving it in
/// depth — which is precisely the shear the formula above describes. Height therefore becomes a
/// horizontal offset, and needs no shader.</para>
/// <para><b>Depth.</b> With everything flattened onto one plane, the depth buffer has nothing to
/// separate. But the vertical axis is now unused: an orthographic camera looking down it projects
/// it away entirely. So it is repurposed as the sort key, carrying the screen Y of each object's
/// ground point — which is exactly what the original sorted on
/// (<c>sortVal_ = int(posS_[1])</c>, tie-broken by object id). Painter's order then falls out of
/// the depth test for free.</para>
/// </remarks>
public readonly struct WorldProjection
{
    /// <summary>Pixels per world tile. The original's world-to-screen matrix is a uniform scale of 50.</summary>
    public const float PixelsPerTile = 50f;

    /// <summary>
    /// How far apart consecutive sort values are placed along the depth axis. Large enough that
    /// neighbouring objects never z-fight, small enough that a whole map fits inside the camera's
    /// depth range.
    /// </summary>
    private const float DepthScale = 0.001f;

    /// <summary>Keeps every depth key positive so it stays inside the near and far planes.</summary>
    private const float DepthBias = 512f;

    /// <summary>The camera's rotation about the vertical, in radians. The original's cameraAngle.</summary>
    public readonly float Angle;

    private readonly float _sin;
    private readonly float _cos;

    public WorldProjection(float angle)
    {
        Angle = angle;
        _sin = Mathf.Sin(angle);
        _cos = Mathf.Cos(angle);
    }

    /// <summary>
    /// The camera's yaw. Negated because Godot's camera basis turns the opposite way from the
    /// original's screen-space rotation.
    /// </summary>
    public float CameraYaw => -Angle;

    /// <summary>
    /// The direction, in the ground plane, that height displaces a sprite. This is the camera's own
    /// up vector, which lies flat because the camera points straight down.
    /// </summary>
    public Vector3 HeightAxis => new(_sin, 0f, -_cos);

    /// <summary>
    /// Screen Y of a ground point, in tiles and increasing downward. The original's sort key, and
    /// here also the depth key.
    /// </summary>
    /// <remarks>
    /// This is the projection onto the camera's up vector, <c>(-sin a, cos a)</c> — note the
    /// negated sine. Getting that sign wrong mirrors the draw order across the screen's vertical
    /// axis, which looks correct at a camera angle of zero and wrong at every other angle.
    /// </remarks>
    public float ScreenDepth(float tileX, float tileY) => -tileX * _sin + tileY * _cos;

    /// <summary>
    /// Places a point in the 3D scene.
    /// </summary>
    /// <param name="tileX">Game X, in tiles.</param>
    /// <param name="tileY">Game Y, in tiles.</param>
    /// <param name="height">Game Z — height above the ground, in tiles.</param>
    /// <param name="sortBias">
    /// Nudges the depth key without moving the object on screen. Used for the sub-orderings the
    /// original expressed by drawing in separate passes: shadows and flat decals beneath
    /// everything, then objects, then tile tops.
    /// </param>
    public Vector3 ToScene(float tileX, float tileY, float height = 0f, float sortBias = 0f)
    {
        // Height shifts the sprite within the ground plane; the vertical axis carries depth only.
        float x = tileX + height * _sin;
        float z = tileY - height * _cos;
        float depth = (DepthBias + ScreenDepth(tileX, tileY) + sortBias) * DepthScale;
        return new Vector3(x, depth, z);
    }

    /// <summary>
    /// The scene position of the camera itself, given what it is looking at. The height is
    /// arbitrary for an orthographic projection and only has to clear the depth range.
    /// </summary>
    public Vector3 CameraPosition(float tileX, float tileY) => new(tileX, 64f, tileY);

    /// <summary>
    /// Converts a screen offset in pixels into a world offset in tiles, for picking and for
    /// aiming at the cursor.
    /// </summary>
    public Vector2 ScreenToWorldOffset(Vector2 pixels)
    {
        float x = pixels.X / PixelsPerTile;
        float y = pixels.Y / PixelsPerTile;
        // The inverse of the rotation the projection applies.
        return new Vector2(x * _cos - y * _sin, x * _sin + y * _cos);
    }
}

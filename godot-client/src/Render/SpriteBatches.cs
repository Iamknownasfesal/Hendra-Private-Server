using System.Collections.Generic;
using Godot;

namespace Hendra.Render;

/// <summary>
/// The nodes sprites are actually drawn through: one MultiMesh per surface, reused every frame.
/// </summary>
/// <remarks>
/// <para>
/// A MultiMesh needs a node of its own, so this keeps a pool of them and hands one out per surface
/// per frame, hiding whatever is left over. They are never freed: the count settles at however many
/// distinct sheets and tints are on screen and stays there, and creating scene nodes during a frame
/// is precisely the kind of cost this whole exercise is trying to avoid.
/// </para>
/// <para>
/// Two base quads rather than one. Both span nought to one, but the outlined variant reaches an
/// eighth of a sprite beyond that on every side, because the outline is drawn on the empty texels
/// next to solid ones and needs somewhere to land. Putting the padding in the mesh rather than the
/// transform keeps the transform the sprite's own size, so the shader's rectangle arithmetic stays
/// the same for both.
/// </para>
/// </remarks>
public sealed class SpriteBatches
{
    /// <summary>How far an outlined quad reaches past the sprite, as a fraction of its size.</summary>
    private const float OutlinePadding = 0.12f;

    /// <summary>Twelve of transform, four of colour, four naming the sheet rectangle.</summary>
    private const int Stride = 20;

    /// <summary>
    /// Instance counts are rounded up to a multiple of this.
    /// </summary>
    /// <remarks>
    /// Godot will only take a buffer exactly as long as the instance count, so a buffer sized to
    /// the exact number of sprites would have to be reallocated on any frame where that number
    /// changed -- which is every frame. Rounding the count up to a step and parking the spare
    /// instances at zero scale, where they cover no pixels, keeps the buffer the same length for
    /// long stretches and lets it be reused.
    /// </remarks>
    private const int Step = 256;

    private sealed class Batch
    {
        public MultiMeshInstance3D Node;
        public float[] Buffer = System.Array.Empty<float>();
    }

    private readonly Node3D _parent;
    private readonly List<Batch> _pool = new();

    private Mesh _plainQuad;
    private Mesh _paddedQuad;
    private int _used;

    public SpriteBatches(Node3D parent) => _parent = parent;

    public void Begin() => _used = 0;

    /// <summary>Draws a batch of sprites that share a sheet, a tint and an outline setting.</summary>
    /// <param name="instances">
    /// Twenty floats each, and longer than the batch needs -- only the first
    /// <paramref name="count"/> instances are read.
    /// </param>
    public void Submit(bool outlined, Material material, float[] instances, int count)
    {
        var batch = At(_used++);
        var multi = batch.Node.Multimesh;

        var quad = outlined ? Padded() : Plain();
        if (multi.Mesh != quad)
            multi.Mesh = quad;

        int rounded = (count + Step - 1) / Step * Step;
        int floats = rounded * Stride;

        if (batch.Buffer.Length != floats)
            batch.Buffer = new float[floats];

        System.Array.Copy(instances, batch.Buffer, count * Stride);

        // The spare instances are left at every value zero, which is a transform with no scale --
        // a quad of no area, covering nothing.
        System.Array.Clear(batch.Buffer, count * Stride, floats - count * Stride);

        // Count before buffer: the length is checked against whatever the count currently says.
        multi.InstanceCount = rounded;
        multi.Buffer = batch.Buffer;

        batch.Node.MaterialOverride = material;
        batch.Node.Visible = true;
    }

    /// <summary>Hides whatever the pool held over from a busier frame.</summary>
    public void End()
    {
        for (int i = _used; i < _pool.Count; i++)
            _pool[i].Node.Visible = false;
    }

    private Batch At(int index)
    {
        while (_pool.Count <= index)
        {
            var node = new MultiMeshInstance3D
            {
                Multimesh = new MultiMesh
                {
                    TransformFormat = MultiMesh.TransformFormatEnum.Transform3D,
                    UseColors = true,
                    UseCustomData = true,
                },
                CastShadow = GeometryInstance3D.ShadowCastingSetting.Off,
            };

            _parent.AddChild(node);
            _pool.Add(new Batch { Node = node });
        }

        return _pool[index];
    }

    private Mesh Plain() => _plainQuad ??= Quad(0f);

    private Mesh Padded() => _paddedQuad ??= Quad(OutlinePadding);

    /// <summary>
    /// A quad in the plane the sprites lie in, spanning nought to one plus whatever padding.
    /// </summary>
    /// <remarks>
    /// Its UVs match its corners, so a corner at minus an eighth reads an eighth of a sprite's
    /// width outside the rectangle -- which the shader treats as empty, and which is exactly the
    /// room an outline needs.
    /// </remarks>
    private static Mesh Quad(float pad)
    {
        float low = -pad;
        float high = 1f + pad;

        var positions = new[]
        {
            new Vector3(low, low, 0f),
            new Vector3(high, low, 0f),
            new Vector3(high, high, 0f),
            new Vector3(low, high, 0f),
        };

        var uvs = new[]
        {
            new Vector2(low, low),
            new Vector2(high, low),
            new Vector2(high, high),
            new Vector2(low, high),
        };

        var arrays = new Godot.Collections.Array();
        arrays.Resize((int)Mesh.ArrayType.Max);
        arrays[(int)Mesh.ArrayType.Vertex] = positions;
        arrays[(int)Mesh.ArrayType.TexUV] = uvs;
        arrays[(int)Mesh.ArrayType.Index] = new[] { 0, 1, 2, 0, 2, 3 };

        var mesh = new ArrayMesh();
        mesh.AddSurfaceFromArrays(Mesh.PrimitiveType.Triangles, arrays);
        return mesh;
    }
}

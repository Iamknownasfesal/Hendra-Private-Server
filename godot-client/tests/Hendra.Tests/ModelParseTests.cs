using Godot;
using Hendra.Render;
using Xunit;

namespace Hendra.Tests;

/// <summary>
/// Parsing the Wavefront models.
///
/// The subtle part is vertex identity. A vertex is keyed by the whole face token, not by its
/// position index, so <c>6/5</c> and <c>6/9</c> are two vertices at the same point carrying
/// different texture coordinates — which is exactly what a cube corner needs, since the three faces
/// meeting there sample three different parts of the sprite. Keying on the position instead
/// silently smears one face's texture across its neighbours.
/// </summary>
public sealed class ModelParseTests
{
    [Fact]
    public void ReadsPositionsAndTriangles()
    {
        var model = ModelLibrary.Parse("""
            # a single triangle
            v 0 0 0
            v 1 0 0
            v 0 1 0
            f 1 2 3
            """);

        Assert.Single(model.Triangles);
        Assert.Equal(3, model.Vertices.Length);
        Assert.Equal(new Vector3(1f, 0f, 0f), model.Vertices[1]);
    }

    /// <summary>A quad becomes two triangles fanned from its first corner.</summary>
    [Fact]
    public void TriangulatesPolygons()
    {
        var model = ModelLibrary.Parse("""
            v 0 0 0
            v 1 0 0
            v 1 1 0
            v 0 1 0
            f 1 2 3 4
            """);

        Assert.Equal(2, model.Triangles.Length);
        Assert.Equal(0, model.Triangles[0].A);
        Assert.Equal(0, model.Triangles[1].A);
        Assert.Equal(4, model.Vertices.Length);
    }

    /// <summary>
    /// The same position with two different texture coordinates is two vertices; the same token
    /// twice is one.
    /// </summary>
    [Fact]
    public void VertexIdentityIsTheWholeFaceToken()
    {
        var model = ModelLibrary.Parse("""
            v 0 0 0
            v 1 0 0
            v 0 1 0
            vt 0 0
            vt 1 0
            vt 0 1
            f 1/1 2/2 3/3
            f 1/2 2/2 3/3
            """);

        // Six corners across two faces, but 1/1 and 1/2 differ while 2/2 and 3/3 repeat: four.
        Assert.Equal(4, model.Vertices.Length);
        Assert.Equal(model.Vertices[0], model.Vertices[3]);
        Assert.NotEqual(model.Uvs[0], model.Uvs[3]);
    }

    /// <summary>Indices are one-based, and a negative one counts back from the end.</summary>
    [Fact]
    public void HandlesNegativeIndices()
    {
        var model = ModelLibrary.Parse("""
            v 0 0 0
            v 1 0 0
            v 0 1 0
            f -3 -2 -1
            """);

        Assert.Single(model.Triangles);
        Assert.Equal(new Vector3(0f, 0f, 0f), model.Vertices[0]);
        Assert.Equal(new Vector3(0f, 1f, 0f), model.Vertices[2]);
    }

    /// <summary>
    /// A material named Solid marks the faces that take the object's colour instead of its artwork,
    /// and the marking applies from that line until the next material.
    /// </summary>
    [Fact]
    public void SolidMaterialsMarkUntexturedFaces()
    {
        var model = ModelLibrary.Parse("""
            v 0 0 0
            v 1 0 0
            v 0 1 0
            v 0 0 1
            usemtl lambert3SG
            f 1 2 3
            usemtl Solid1
            f 1 2 4
            """);

        Assert.Equal(2, model.Triangles.Length);
        Assert.True(model.Triangles[0].Textured);
        Assert.False(model.Triangles[1].Textured);
    }

    /// <summary>
    /// A face with no texture coordinate gets the origin, which samples one texel — six of the
    /// shipped models have no coordinates at all.
    /// </summary>
    [Fact]
    public void FacesWithoutTextureCoordinatesGetTheOrigin()
    {
        var model = ModelLibrary.Parse("""
            v 0 0 0
            v 1 0 0
            v 0 1 0
            f 1//2 2//2 3//2
            """);

        Assert.Single(model.Triangles);
        Assert.All(model.Uvs, uv => Assert.Equal(Vector2.Zero, uv));
    }

    /// <summary>The normal comes from the winding, and is what the lighting is computed against.</summary>
    [Fact]
    public void ComputesFaceNormals()
    {
        var model = ModelLibrary.Parse("""
            v 0 0 0
            v 1 0 0
            v 0 1 0
            f 1 2 3
            """);

        // Counter-clockwise in the ground plane, so the normal points straight up.
        Assert.Equal(new Vector3(0f, 0f, 1f), model.Triangles[0].Normal);
    }

    /// <summary>Nonsense in, nothing out — never an exception during a frame.</summary>
    [Theory]
    [InlineData("")]
    [InlineData("# just a comment\n")]
    [InlineData("f 1 2 3\n")]
    [InlineData("v 0 0 0\nf 1 9 42\n")]
    [InlineData("v nope nope nope\nf 1 1 1\n")]
    public void ToleratesMalformedInput(string text)
    {
        var model = ModelLibrary.Parse(text);
        Assert.NotNull(model);
    }
}

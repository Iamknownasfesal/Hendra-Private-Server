using System;
using System.Collections.Generic;
using System.Globalization;
using Godot;

namespace Hendra.Render;

/// <summary>One triangle of a model, already flattened out of whatever polygon it came from.</summary>
public readonly struct ModelTriangle
{
    /// <summary>Indices into the model's vertex list.</summary>
    public readonly int A;

    public readonly int B;
    public readonly int C;

    /// <summary>The face normal in model space, for lighting.</summary>
    public readonly Vector3 Normal;

    /// <summary>
    /// Whether this face takes the object's artwork or a flat colour.
    /// </summary>
    /// <remarks>
    /// Decided by the material name: anything beginning <c>Solid</c> is painted in the object's
    /// declared colour instead of sampling its sprite. It is how the models express the parts that
    /// are not meant to look like the artwork — the mortar between bricks, mostly.
    /// </remarks>
    public readonly bool Textured;

    public ModelTriangle(int a, int b, int c, Vector3 normal, bool textured)
    {
        A = a;
        B = b;
        C = c;
        Normal = normal;
        Textured = textured;
    }
}

/// <summary>
/// A parsed model: unique vertices with their texture coordinates, and the triangles over them.
/// </summary>
/// <remarks>
/// Model units are tiles, and the axes are the game's: X and Y along the ground, Z up. A cube is
/// therefore one unit wide and one unit tall, sitting on the ground plane rather than centred on it.
/// </remarks>
public sealed class Model3D
{
    public Vector3[] Vertices = Array.Empty<Vector3>();
    public Vector2[] Uvs = Array.Empty<Vector2>();
    public ModelTriangle[] Triangles = Array.Empty<ModelTriangle>();

    public bool IsValid => Triangles.Length > 0;
}

/// <summary>
/// The models named by <c>&lt;Model&gt;</c> in the game data, parsed from the Wavefront OBJ files
/// the original embedded.
/// </summary>
/// <remarks>
/// <para>
/// Parsed here rather than through Godot's own OBJ importer, which produces a mesh and throws away
/// the two things this needs: which material each face used — the <c>Solid</c> distinction above —
/// and the exact vertex identity the original relies on. The original's parser keys a vertex on the
/// whole face token, so <c>6/5</c> and <c>6/9</c> are different vertices sharing a position, which
/// is what lets one corner of a cube carry three different texture coordinates.
/// </para>
/// <para>
/// The files themselves come out of the asset extractor: the embedded <c>.dat</c> resources are
/// Wavefront OBJ text with the extension changed.
/// </para>
/// </remarks>
public sealed class ModelLibrary
{
    private readonly Dictionary<string, Model3D> _models = new();
    private readonly Assets.AssetManifest _manifest;

    public ModelLibrary(Assets.AssetManifest manifest)
    {
        _manifest = manifest;
    }

    public int Count => _models.Count;

    /// <summary>
    /// The model of that name, parsed on first use. Null when the game data names one that is not
    /// in the manifest, which happens and is not worth failing over.
    /// </summary>
    public Model3D Get(string name)
    {
        if (string.IsNullOrEmpty(name))
            return null;

        if (_models.TryGetValue(name, out var cached))
            return cached;

        var model = Load(name);
        _models[name] = model;
        return model;
    }

    private Model3D Load(string name)
    {
        if (_manifest?.Models == null || !_manifest.Models.TryGetValue(name, out string file))
            return null;

        string path = Assets.AssetManifest.ModelDirectory + file;
        using var handle = FileAccess.Open(path, FileAccess.ModeFlags.Read);
        if (handle == null)
        {
            GD.PushWarning($"[content] missing model {path}; run tools/extract_assets.py.");
            return null;
        }

        try
        {
            return Parse(handle.GetAsText());
        }
        catch (Exception ex)
        {
            GD.PushWarning($"[content] could not parse model {file}: {ex.Message}");
            return null;
        }
    }

    /// <summary>
    /// Parses Wavefront OBJ text. Only v, vt, f and usemtl carry meaning here.
    /// </summary>
    /// <remarks>Public so it can be tested without a filesystem or an engine behind it.</remarks>
    public static Model3D Parse(string text)
    {
        var positions = new List<Vector3>();
        var texCoords = new List<Vector2>();

        // Vertices are keyed by the face token, so the same position with a different texture
        // coordinate becomes a separate vertex -- which is what a cube corner needs.
        var byToken = new Dictionary<string, int>();
        var vertices = new List<Vector3>();
        var uvs = new List<Vector2>();
        var triangles = new List<ModelTriangle>();

        bool textured = true;
        var separators = new[] { ' ', '\t' };

        foreach (string rawLine in text.Split('\n'))
        {
            string line = rawLine.Trim();
            if (line.Length == 0 || line[0] == '#')
                continue;

            var parts = line.Split(separators, StringSplitOptions.RemoveEmptyEntries);
            if (parts.Length < 2)
                continue;

            switch (parts[0])
            {
                case "v" when parts.Length >= 4:
                    positions.Add(new Vector3(Number(parts[1]), Number(parts[2]), Number(parts[3])));
                    break;

                case "vt" when parts.Length >= 3:
                    texCoords.Add(new Vector2(Number(parts[1]), Number(parts[2])));
                    break;

                case "usemtl":
                    textured = !parts[1].StartsWith("Solid", StringComparison.Ordinal);
                    break;

                case "f" when parts.Length >= 4:
                    AddFace(parts, positions, texCoords, byToken, vertices, uvs, triangles, textured);
                    break;
            }
        }

        return new Model3D
        {
            Vertices = vertices.ToArray(),
            Uvs = uvs.ToArray(),
            Triangles = triangles.ToArray(),
        };
    }

    private static void AddFace(
        string[] parts,
        List<Vector3> positions,
        List<Vector2> texCoords,
        Dictionary<string, int> byToken,
        List<Vector3> vertices,
        List<Vector2> uvs,
        List<ModelTriangle> triangles,
        bool textured)
    {
        Span<int> corners = stackalloc int[parts.Length - 1];

        for (int i = 1; i < parts.Length; i++)
        {
            string token = parts[i];
            if (byToken.TryGetValue(token, out int existing))
            {
                corners[i - 1] = existing;
                continue;
            }

            var fields = token.Split('/');
            int positionIndex = Index(fields[0], positions.Count);
            if (positionIndex < 0)
                return;

            // A face may name no texture coordinate at all, or name one on a model that ships none.
            // Both mean this face samples a single texel, which is what the original does with it.
            int uvIndex = fields.Length > 1 ? Index(fields[1], texCoords.Count) : -1;

            byToken[token] = vertices.Count;
            corners[i - 1] = vertices.Count;
            vertices.Add(positions[positionIndex]);
            uvs.Add(uvIndex >= 0 ? texCoords[uvIndex] : Vector2.Zero);
        }

        // The original takes the normal from the first, second and last corners, so a fan uses one
        // normal for the whole polygon -- these faces are flat.
        var normal = Normal(vertices[corners[0]], vertices[corners[1]], vertices[corners[^1]]);

        for (int i = 1; i + 1 < corners.Length; i++)
            triangles.Add(new ModelTriangle(corners[0], corners[i], corners[i + 1], normal, textured));
    }

    /// <summary>An OBJ index: one-based, and negative counts back from the end.</summary>
    private static int Index(string field, int count)
    {
        if (field.Length == 0 || !int.TryParse(field, NumberStyles.Integer, CultureInfo.InvariantCulture, out int value))
            return -1;

        int index = value > 0 ? value - 1 : count + value;
        return index >= 0 && index < count ? index : -1;
    }

    private static Vector3 Normal(Vector3 a, Vector3 b, Vector3 c)
    {
        var normal = (b - a).Cross(c - a);
        return normal.LengthSquared() > 0f ? normal.Normalized() : Vector3.Up;
    }

    private static float Number(string text) =>
        float.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out float value) ? value : 0f;
}

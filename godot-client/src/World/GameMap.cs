using System;
using System.Collections.Generic;
using Hendra.Resources;

namespace Hendra.World;

/// <summary>
/// The world: a grid of tiles and the entities standing on it.
/// </summary>
/// <remarks>
/// Tiles are stored as a flat array covering the whole map, allocated once when MapInfo arrives.
/// Terrain streams in through Update packets and is never removed, so an allocated-on-demand
/// structure would only ever grow anyway.
///
/// Entities are kept in insertion order as well as by id. The original iterated a Flash
/// <c>Dictionary</c>, whose order is unspecified, which meant update and draw order varied between
/// runs — a source of irreproducible behaviour that is not worth carrying over.
/// </remarks>
public sealed class GameMap : ITileQuery
{
    private readonly GameData _data;
    private Square[] _squares = Array.Empty<Square>();

    private readonly Dictionary<int, Entity> _entitiesById = new();
    private readonly List<Entity> _entities = new();

    // Mutating the entity table while iterating it is normal — an entity's update can spawn a
    // projectile or kill something — so changes made during a tick are deferred.
    private readonly List<Entity> _pendingAdds = new();
    private readonly List<int> _pendingRemovals = new();
    private bool _updating;

    public GameMap(GameData data)
    {
        _data = data;
    }

    public int Width { get; private set; }
    public int Height { get; private set; }
    public string Name { get; private set; } = string.Empty;

    /// <summary>Our own player, once CreateSuccess and the first Update have arrived.</summary>
    public LocalPlayer Player { get; internal set; }

    public IReadOnlyList<Entity> Entities => _entities;

    /// <summary>Allocates the grid for a newly entered world, discarding whatever was there.</summary>
    public void Reset(int width, int height, string name)
    {
        Width = width;
        Height = height;
        Name = name ?? string.Empty;

        _squares = new Square[width * height];
        _entitiesById.Clear();
        _entities.Clear();
        _pendingAdds.Clear();
        _pendingRemovals.Clear();
        Player = null;
    }

    private bool InBounds(int x, int y) => x >= 0 && y >= 0 && x < Width && y < Height;

    /// <summary>The tile at these coordinates, or null if they fall outside the map.</summary>
    public Square GetSquare(int x, int y)
    {
        if (!InBounds(x, y))
            return null;

        int index = x + y * Width;
        return _squares[index] ??= new Square();
    }

    public Square GetSquare(float x, float y) => GetSquare((int)x, (int)y);

    /// <summary>Applies one tile from an Update packet.</summary>
    public void SetTile(int x, int y, ushort type)
    {
        var square = GetSquare(x, y);
        if (square == null)
            return;

        square.TileType = type;
        square.Desc = _data.GetGround(type);

        // Sink depth is a property of the terrain, resolved once here rather than per frame.
        square.Sink = square.Desc is { Sink: true } ? 12 : 0;
    }

    // ------------------------------------------------------------------------------------------
    // Entities
    // ------------------------------------------------------------------------------------------

    public Entity GetEntity(int objectId) =>
        _entitiesById.TryGetValue(objectId, out var entity) ? entity : null;

    public void Add(Entity entity, float x, float y)
    {
        entity.Place(x, y);

        if (_updating)
        {
            _pendingAdds.Add(entity);
            return;
        }

        Insert(entity);
    }

    private void Insert(Entity entity)
    {
        // An id can be reused once the server has dropped the previous holder.
        if (_entitiesById.TryGetValue(entity.ObjectId, out var existing))
            Detach(existing);

        _entitiesById[entity.ObjectId] = entity;
        _entities.Add(entity);
        Attach(entity);
    }

    public void Remove(int objectId)
    {
        if (_updating)
        {
            _pendingRemovals.Add(objectId);
            return;
        }

        if (_entitiesById.TryGetValue(objectId, out var entity))
        {
            Detach(entity);
            _entitiesById.Remove(objectId);
            _entities.Remove(entity);
        }
    }

    /// <summary>Binds a static object to its tile, so collision can find it without a search.</summary>
    private void Attach(Entity entity)
    {
        var square = GetSquare(entity.X, entity.Y);
        entity.Square = square;

        if (square != null && entity.Desc is { Static: true })
            square.StaticObject = entity;
    }

    private void Detach(Entity entity)
    {
        if (entity.Square?.StaticObject == entity)
            entity.Square.StaticObject = null;
        entity.Square = null;
    }

    /// <summary>Moves an entity, keeping its tile binding current.</summary>
    public void MoveEntity(Entity entity, float x, float y)
    {
        var square = GetSquare(x, y);
        if (square == null)
            return;

        entity.X = x;
        entity.Y = y;

        if (!ReferenceEquals(square, entity.Square))
        {
            if (entity.Desc is { Static: true })
            {
                if (entity.Square?.StaticObject == entity)
                    entity.Square.StaticObject = null;
                square.StaticObject = entity;
            }

            entity.Square = square;
        }
    }

    /// <summary>Advances every entity by one frame.</summary>
    public void Update(int nowMs, int deltaMs)
    {
        _updating = true;
        foreach (var entity in _entities)
            entity.Update(nowMs, deltaMs);
        _updating = false;

        if (_pendingRemovals.Count > 0)
        {
            foreach (int objectId in _pendingRemovals)
                Remove(objectId);
            _pendingRemovals.Clear();
        }

        if (_pendingAdds.Count > 0)
        {
            foreach (var entity in _pendingAdds)
                Insert(entity);
            _pendingAdds.Clear();
        }
    }

    // ------------------------------------------------------------------------------------------
    // ITileQuery
    // ------------------------------------------------------------------------------------------

    public bool IsWalkable(int tileX, int tileY) => GetSquare(tileX, tileY)?.IsWalkable ?? false;

    /// <summary>
    /// Out-of-bounds and not-yet-streamed tiles both report as blocking, which is what confines the
    /// player to the region the server has actually described.
    /// </summary>
    public bool IsFullOccupy(int tileX, int tileY) => GetSquare(tileX, tileY)?.IsFullOccupy ?? true;
}

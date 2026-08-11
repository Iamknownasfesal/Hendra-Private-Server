using System;
using Godot;
using Hendra.Assets;
using Hendra.Core;
using Hendra.Data;
using Hendra.Net;
using Hendra.Net.Packets;
using Hendra.Render;
using Hendra.Resources;

namespace Hendra.World;

/// <summary>
/// Runs a session: turns packets into world state, input into movement, and world state into
/// geometry.
/// </summary>
/// <remarks>
/// <para>
/// The order of operations inside a frame is load bearing rather than incidental. Input and
/// movement run first, the resulting position is published to the session, and only then is the
/// socket drained — because draining it may deliver a NewTick, and answering a NewTick means
/// sending a Move built from that published position. Polling first would report last frame's
/// position every tick, which reads as rubber-banding.
/// </para>
/// <para>
/// This replaces roughly thirty Robotlegs config classes, ninety signal types and sixty mediators
/// in the original, which between them did the same wiring indirectly.
/// </para>
/// </remarks>
public partial class WorldController : Node
{
    private GameSession _session;
    private GameMap _map;
    private GameData _data;
    private AssetLibrary _assets;
    private TextureResolver _textures;
    private WorldRoot _world;
    private GameClock _clock;

    private float _cameraAngle = 7f * Mathf.Pi / 4f;
    private Entity _focus;

    /// <summary>Whether the camera keeps the player centred or offset towards the top.</summary>
    public bool CenterOnPlayer { get; set; } = true;

    public GameMap Map => _map;

    public void Begin(
        GameSession session,
        GameData data,
        AssetLibrary assets,
        WorldRoot world,
        GameClock clock)
    {
        _session = session;
        _data = data;
        _assets = assets;
        _textures = new TextureResolver(assets);
        _world = world;
        _clock = clock;
        _map = new GameMap(data);

        _session.MapLoaded += OnMapLoaded;
        _session.WorldUpdated += OnWorldUpdated;
        _session.Ticked += OnTicked;
        _session.Repositioned += OnRepositioned;
        _session.Entered += OnEntered;
    }

    public override void _ExitTree()
    {
        if (_session == null)
            return;

        _session.MapLoaded -= OnMapLoaded;
        _session.WorldUpdated -= OnWorldUpdated;
        _session.Ticked -= OnTicked;
        _session.Repositioned -= OnRepositioned;
        _session.Entered -= OnEntered;
    }

    // ------------------------------------------------------------------------------------------
    // Packet handling
    // ------------------------------------------------------------------------------------------

    private void OnMapLoaded(MapInfoPacket mapInfo)
    {
        _map.Reset(mapInfo.Width, mapInfo.Height, mapInfo.Name);

        // Per-map XML overlays replace base definitions for the types they mention.
        foreach (string xml in mapInfo.ClientXml)
            SafeMerge(xml);
        foreach (string xml in mapInfo.ExtraXml)
            SafeMerge(xml);
    }

    private void SafeMerge(string xml)
    {
        if (string.IsNullOrWhiteSpace(xml))
            return;

        try
        {
            _data.AddObjects(xml);
        }
        catch (Exception ex)
        {
            // A malformed overlay costs us one dungeon's custom objects; it should not end the
            // session.
            GD.PushWarning($"[world] could not merge map XML: {ex.Message}");
        }
    }

    private void OnEntered(CreateSuccessPacket packet)
    {
        // The player entity itself arrives in the first Update, not here.
    }

    private void OnWorldUpdated(UpdatePacket update)
    {
        foreach (var tile in update.Tiles)
            _map.SetTile(tile.X, tile.Y, tile.Type);

        foreach (int objectId in update.Drops)
            _map.Remove(objectId);

        foreach (var definition in update.NewObjects)
            AddObject(definition);
    }

    private void AddObject(in ObjectDef definition)
    {
        var desc = _data.GetObject(definition.ObjectType);
        bool isSelf = definition.Stats.ObjectId == _session.PlayerObjectId;

        Entity entity = isSelf ? new LocalPlayer() : new Entity();
        entity.ObjectId = definition.Stats.ObjectId;
        entity.ObjectType = definition.ObjectType;
        entity.Desc = desc;
        entity.Z = desc?.Z ?? 0f;
        entity.Equipment = StatApplier.NewEmptyInventory();

        if (desc != null)
        {
            entity.MaxHp = desc.MaxHitPoints;
            entity.Hp = desc.MaxHitPoints;
            entity.Defense = desc.Defense;
            entity.Texture1 = desc.Tex1;
            entity.Texture2 = desc.Tex2;
        }

        StatApplier.Apply(entity, definition.Stats, isSelf);
        _map.Add(entity, definition.Stats.Position.X, definition.Stats.Position.Y);

        if (isSelf && entity is LocalPlayer player)
        {
            _map.Player = player;
            _focus = player;
            player.OnMoved();

            // Publish immediately rather than waiting for the next frame. This Update and the first
            // NewTick can be drained in the same poll, and the Move answering that tick reads these
            // fields -- so deferring it reports a position we never had.
            _session.PlayerX = player.X;
            _session.PlayerY = player.Y;
            _session.HasPlayerPosition = true;
        }
    }

    private void OnTicked(NewTickPacket tick)
    {
        foreach (var status in tick.Statuses)
        {
            var entity = _map.GetEntity(status.ObjectId);
            if (entity == null)
                continue;

            bool isSelf = status.ObjectId == _session.PlayerObjectId;

            // Our own position is never taken from a tick; the client owns it between Moves.
            if (!isSelf)
                entity.OnTickPosition(status.Position.X, status.Position.Y, tick.TickTime);

            StatApplier.Apply(entity, status, isSelf);
        }
    }

    private void OnRepositioned(GotoPacket packet)
    {
        var entity = _map.GetEntity(packet.ObjectId);
        entity?.OnGoto(packet.Position.X, packet.Position.Y);

        if (entity is LocalPlayer player)
            player.OnMoved();
    }

    // ------------------------------------------------------------------------------------------
    // Frame
    // ------------------------------------------------------------------------------------------

    public override void _Process(double delta)
    {
        if (_session == null || _map == null)
            return;

        int now = _clock.FrameMs;
        int deltaMs = _clock.DeltaMs;

        ApplyInput(deltaMs);

        var player = _map.Player;
        if (player != null)
            player.UpdateMovement(_map, _cameraAngle, deltaMs);

        _map.Update(now, deltaMs);

        // Publish before polling: a NewTick delivered by Poll answers with a Move built from this.
        if (player != null)
        {
            _session.PlayerX = player.X;
            _session.PlayerY = player.Y;
            _session.PlayerPaused = player.IsPaused;
            _session.HasPlayerPosition = true;
        }

        _session.RecordPosition();
        _session.Poll();

        Draw(now);
    }

    private void ApplyInput(int deltaMs)
    {
        var player = _map.Player;
        if (player == null)
            return;

        float x = Input.GetActionStrength("move_right") - Input.GetActionStrength("move_left");
        float y = Input.GetActionStrength("move_down") - Input.GetActionStrength("move_up");
        float rotate = Input.GetActionStrength("rotate_right") - Input.GetActionStrength("rotate_left");

        player.SetInput(x, y, rotate);

        if (player.InputRotate != 0f)
            _cameraAngle += deltaMs * LocalPlayer.RotateSpeed * player.InputRotate;

        if (Input.IsActionJustPressed("reset_camera"))
            _cameraAngle = 7f * Mathf.Pi / 4f;
    }

    // ------------------------------------------------------------------------------------------
    // Drawing
    // ------------------------------------------------------------------------------------------

    private void Draw(int now)
    {
        var focus = _focus ?? _map.Player;
        if (focus == null)
            return;

        _world.Configure(focus.X, focus.Y, _cameraAngle);

        float radius = _world.VisibleRadius();
        DrawGround(focus, radius, now);
        DrawEntities(now, _cameraAngle);

        _world.Render();
    }

    private void DrawGround(Entity focus, float radius, int now)
    {
        int minX = Mathf.Max(0, (int)(focus.X - radius));
        int maxX = Mathf.Min(_map.Width - 1, (int)(focus.X + radius));
        int minY = Mathf.Max(0, (int)(focus.Y - radius));
        int maxY = Mathf.Min(_map.Height - 1, (int)(focus.Y + radius));
        float radiusSquared = radius * radius;

        for (int y = minY; y <= maxY; y++)
        {
            for (int x = minX; x <= maxX; x++)
            {
                // The visible area is a circle, not the bounding square, so corners are skipped.
                float dx = x + 0.5f - focus.X;
                float dy = y + 0.5f - focus.Y;
                if (dx * dx + dy * dy > radiusSquared)
                    continue;

                var square = _map.GetSquare(x, y);
                if (square is not { IsKnown: true } || square.Desc == null)
                    continue;

                var texture = _textures.Resolve(square.Desc.Texture, x * 31 + y * 17);
                if (!texture.Still.IsValid)
                    continue;

                _world.Ground.Add(new GroundDraw
                {
                    TileX = x,
                    TileY = y,
                    Sprite = texture.Still,
                    UvOffset = AnimationOffset(square.Desc, now),
                    Modulate = Colors.White,
                });
            }
        }
    }

    /// <summary>Scroll or ripple offset for an animated tile, in fractions of a tile.</summary>
    private static Vector2 AnimationOffset(GroundDesc desc, int now)
    {
        float seconds = now / 1000f;
        return desc.Animation switch
        {
            GroundAnimation.Wave => new Vector2(
                Mathf.Sin(desc.AnimationDx * seconds),
                Mathf.Sin(desc.AnimationDy * seconds)),

            GroundAnimation.Flow => new Vector2(
                desc.AnimationDx * seconds,
                desc.AnimationDy * seconds),

            _ => Vector2.Zero,
        };
    }

    private void DrawEntities(int now, float cameraAngle)
    {
        foreach (var entity in _map.Entities)
        {
            if (entity.Desc == null || entity.Square is not { IsKnown: true })
                continue;

            var resolved = _textures.Resolve(entity.Desc.Texture, entity.ObjectId, entity.AltTextureIndex);
            if (!resolved.IsValid)
                continue;

            var draw = new SpriteDraw
            {
                TileX = entity.X,
                TileY = entity.Y,
                Height = entity.Z,
                AnchorX = 0.5f,
                Modulate = Modulate(entity),
                // Ties between entities on the same tile resolve by object id, matching the
                // original's secondary sort.
                SortBias = (entity.ObjectId & 0xFF) * 0.001f,
            };

            if (resolved.Animated != null)
            {
                var frame = SelectFrame(entity, resolved.Animated, now, cameraAngle);
                if (!frame.IsValid)
                    continue;

                draw.Sprite = frame.Sprite;
                draw.Mirrored = frame.Mirrored;
                SizeQuad(ref draw, entity, frame.Sprite, frame.CellsWide, frame.RegionCells);
                draw.AnchorX = AnchorFor(frame);
            }
            else
            {
                draw.Sprite = resolved.Still;
                SizeQuad(ref draw, entity, resolved.Still, 1, 1);
            }

            // Objects painted flat on the ground sort beneath anything standing on the tile.
            if (entity.Desc.DrawUnder)
                draw.SortBias -= 1f;

            _world.Sprites.Add(draw);
        }
    }

    /// <summary>
    /// Sizes the quad from the sprite's pixel dimensions and the entity's size stat.
    /// </summary>
    /// <remarks>
    /// Size is a percentage, and the sprites are authored at 8 pixels to the tile, so a
    /// hundred-percent 8x8 sprite is exactly one tile across.
    /// </remarks>
    private static void SizeQuad(ref SpriteDraw draw, Entity entity, Sprite sprite, int cellsWide, int regionCells)
    {
        const float PixelsPerTile = 8f;

        float scale = entity.Size / 100f;
        float cellWidth = sprite.Region.Size.X / (float)Mathf.Max(regionCells, 1);

        draw.WidthTiles = cellWidth * cellsWide / PixelsPerTile * scale;
        draw.HeightTiles = sprite.Region.Size.Y / PixelsPerTile * scale;
    }

    /// <summary>
    /// Where the sprite's anchor sits horizontally.
    /// </summary>
    /// <remarks>
    /// Normally the middle. The extended attack frame is wider than the character and has an empty
    /// cell on one side, so the anchor shifts to keep the character over its tile while the weapon
    /// overhangs — and the empty cell swaps sides when the frame is mirrored.
    /// </remarks>
    private static float AnchorFor(in CharFrame frame)
    {
        if (frame.CellsWide <= 1)
            return 0.5f;

        // Centre of the cell the character itself occupies, as a fraction of the whole quad.
        float characterCell = frame.EffectiveContentCell + (frame.Mirrored ? frame.RegionCells - 1f : 0f);
        return (characterCell + 0.5f) / frame.CellsWide;
    }

    private static CharFrame SelectFrame(Entity entity, AnimatedChar animated, int now, float cameraAngle)
    {
        // Attacking wins over walking, and holds the pose for a fixed window after the shot.
        if (entity.IsAttacking(now))
        {
            float phase = (now - entity.AttackStartMs) % Entity.AttackPeriodMs / (float)Entity.AttackPeriodMs;
            float facing = entity.Desc is { DontFaceAttacks: true } ? entity.Facing : entity.AttackAngle;
            return animated.Frame(facing, cameraAngle, CharAction.Attack, phase);
        }

        if (entity.IsMoving)
        {
            // A fixed cadence rather than one tied to speed; the original quantised it to a
            // multiple of 400ms, which amounts to the same thing for every practical speed.
            const float WalkPeriodMs = 400f;
            float phase = now % WalkPeriodMs / WalkPeriodMs;
            return animated.Frame(entity.Facing, cameraAngle, CharAction.Walk, phase);
        }

        return animated.Frame(entity.Facing, cameraAngle, CharAction.Stand, 0f);
    }

    private static Color Modulate(Entity entity)
    {
        // Invisible players are drawn faintly rather than hidden, so allies can still be followed.
        if (entity.IsInvisible)
            return new Color(1f, 1f, 1f, 0.7f);

        return Colors.White;
    }
}

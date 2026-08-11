using System;
using Godot;
using Hendra.Assets;
using Hendra.Core;
using Hendra.Data;
using Hendra.Net;
using Hendra.Net.Packets;
using Hendra.Render;
using Hendra.Resources;
using Hendra.Text;
using Hendra.UI;

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
    private Combat _combat;
    private Interaction _interaction;
    private Inventory _inventory;
    private Trading _trading;
    private MinimapView _minimap;
    private TileColors _tileColors;
    private TileAtlas _tileAtlas;
    private TileBlender _tileBlender;
    private WorldOverlay _overlay;
    private HudView _hud;
    private ChatView _chat;

    /// <summary>Localised strings, shared with the rest of the client.</summary>
    private StringMap _strings = new();

    private float _cameraAngle = 7f * Mathf.Pi / 4f;
    private Entity _focus;
    private bool _autofire;

    /// <summary>Starts autofire on, for unattended runs. See LaunchOptions.</summary>
    public bool AutofireOnStart { set => _autofire = value; }

    /// <summary>
    /// Raised when the player asks to return to the Nexus.
    /// </summary>
    /// <remarks>
    /// Deliberately not the Escape packet. This fork's server disconnects anyone who sends Escape
    /// while already in the Nexus, and the original client stopped sending it entirely -- it
    /// performs a local reconnect to game id -2 instead.
    /// </remarks>
    public event System.Action NexusRequested;

    /// <summary>Raised when our character dies, with something to show the player.</summary>
    public event System.Action<string> Died;

    /// <summary>Whether the camera keeps the player centred or offset towards the top.</summary>
    public bool CenterOnPlayer { get; set; } = true;

    public GameMap Map => _map;

    /// <summary>The trade in progress, if any. Never null once Begin has run.</summary>
    public Trading Trading => _trading;

    /// <summary>The chat log, so the scene can report things that happen outside the world.</summary>
    public UI.ChatView Chat => _chat;

    public void Begin(
        GameSession session,
        GameData data,
        AssetLibrary assets,
        WorldRoot world,
        GameClock clock,
        WorldOverlay overlay,
        HudView hud = null,
        ChatView chat = null,
        MinimapView minimap = null)
    {
        _minimap = minimap;
        _strings = App.ServiceLocator.Strings ?? _strings;
        _overlay = overlay;
        _hud = hud;
        _chat = chat;

        _session = session;
        _data = data;
        _assets = assets;
        _textures = new TextureResolver(assets);
        _tileColors = new TileColors(assets);
        _tileAtlas = new TileAtlas();
        _tileBlender = new TileBlender(data, assets, _tileAtlas);
        _world = world;
        _clock = clock;
        _map = new GameMap(data);
        _minimap?.Configure(_map, _tileColors);
        _combat = new Combat(_map, data, session, clock);
        _interaction = new Interaction(_map);
        _inventory = new Inventory(_map, data, session, clock);
        _trading = new Trading(session);

        // Subscribed after construction, not alongside the field assignments above: these forward
        // to objects that do not exist until the map does.
        if (_chat != null)
            _chat.Submitted += OnChatSubmitted;

        if (_hud != null)
        {
            _hud.SlotActivated += OnSlotActivated;
            _hud.ContainerSlotActivated += OnContainerSlotActivated;
        }

        _session.MapLoaded += OnMapLoaded;
        _session.WorldUpdated += OnWorldUpdated;
        _session.Ticked += OnTicked;
        _session.Repositioned += OnRepositioned;
        _session.Entered += OnEntered;
        _session.PacketReceived += OnPacket;
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
        _session.PacketReceived -= OnPacket;
    }

    // ------------------------------------------------------------------------------------------
    // Packet handling
    // ------------------------------------------------------------------------------------------

    private void OnMapLoaded(MapInfoPacket mapInfo)
    {
        _map.Reset(mapInfo.Width, mapInfo.Height, mapInfo.Name);
        _combat.Clear();
        _minimap?.Configure(_map, _tileColors);

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
        {
            _map.SetTile(tile.X, tile.Y, tile.Type);
            _minimap?.SetTile(tile.X, tile.Y, _map.GetSquare(tile.X, tile.Y));
        }

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

    /// <summary>Packets the session does not answer itself.</summary>
    private void OnPacket(ServerPacket packet)
    {
        int now = _clock.FrameMs;

        switch (packet)
        {
            case EnemyShootPacket shot:
                OnEnemyShoot(shot, now);
                break;

            case ServerPlayerShootPacket shot:
                OnServerPlayerShoot(shot, now);
                break;

            case AllyShootPacket shot:
                // Cosmetic only: no damage, no acknowledgement, and it spawns from the shooter's
                // current position rather than one carried on the wire.
                _map.GetEntity(shot.OwnerId)?.SetAttack(shot.Angle, now);
                break;

            case DamagePacket damage:
                OnDamage(damage);
                break;

            case TradeRequestedPacket:
            case TradeStartPacket:
            case TradeChangedPacket:
            case TradeAcceptedPacket:
            case TradeDonePacket:
                _trading.Handle(packet);
                break;

            case GuildResultPacket guild:
                _chat?.AddSystem(LineBuilder.Resolve(guild.LineBuilderJson, _strings));
                break;

            case InvitedToGuildPacket invite:
                _chat?.AddSystem(
                    $"{invite.Name} invited you to {invite.GuildName}. Type /join {invite.GuildName} to accept.");
                break;

            case NameResultPacket name when !name.Success:
                _chat?.AddSystem(name.ErrorText);
                break;

            case BuyResultPacket buy:
                _chat?.AddSystem(buy.Result == BuyResultCode.Unknown
                    ? buy.ResultString
                    : LineBuilder.Resolve(buy.ResultString, _strings));
                break;

            case DeathPacket death:
                OnDeath(death);
                break;

            case TextPacket text:
                _chat?.Add(text, LineBuilder.Resolve(text.Text, _strings));
                break;

            case NotificationPacket notification:
                _chat?.AddSystem(LineBuilder.Resolve(notification.Message, _strings));
                break;

            case GlobalNotificationPacket announcement:
                _chat?.AddSystem(LineBuilder.Resolve(announcement.Text, _strings));
                break;
        }
    }

    /// <summary>
    /// An enemy fired. One packet can describe a whole volley.
    /// </summary>
    /// <remarks>
    /// A missing or dead shooter is answered with ShootAck(-1), the "could not spawn this" signal,
    /// and nothing is created.
    /// </remarks>
    private void OnEnemyShoot(EnemyShootPacket shot, int now)
    {
        var owner = _map.GetEntity(shot.OwnerId);
        if (owner?.Desc?.Projectiles == null || owner.Dead ||
            !owner.Desc.Projectiles.TryGetValue(shot.BulletType, out var desc))
        {
            _session.Send(new ShootAckPacket { Time = -1 });
            return;
        }

        for (int i = 0; i < Math.Max(1, (int)shot.NumShots); i++)
        {
            _combat.SpawnRemote(
                desc,
                owner.ObjectType,
                shot.OwnerId,
                (byte)((shot.BulletId + i) % 256),
                shot.Angle + shot.AngleInc * i,
                shot.StartingPos.X,
                shot.StartingPos.Y,
                now,
                shot.Damage,
                damagesPlayers: true);
        }

        _session.Send(new ShootAckPacket { Time = now });
        owner.SetAttack(shot.Angle + shot.AngleInc * (shot.NumShots - 1) / 2f, now);
    }

    /// <summary>A projectile the server authored, such as an ability shot or a nova.</summary>
    private void OnServerPlayerShoot(ServerPlayerShootPacket shot, int now)
    {
        var container = _data.GetObject((ushort)shot.ContainerType);
        if (container?.Projectiles == null || !container.Projectiles.TryGetValue(0, out var desc))
        {
            _session.Send(new ShootAckPacket { Time = -1 });
            return;
        }

        bool mine = shot.OwnerId == _session.PlayerObjectId;
        _combat.SpawnRemote(
            desc,
            (ushort)shot.ContainerType,
            shot.OwnerId,
            shot.BulletId,
            shot.Angle,
            shot.StartingPos.X,
            shot.StartingPos.Y,
            now,
            shot.Damage,
            damagesPlayers: !mine);

        // Only shots we own are acknowledged.
        if (mine)
            _session.Send(new ShootAckPacket { Time = now });
    }

    /// <summary>
    /// Our character died.
    /// </summary>
    /// <remarks>
    /// Permanent: the character is gone, and the session ends. A zombie death is different -- the
    /// server hands back a new object to keep playing as -- but that path is not implemented, so it
    /// is reported the same way rather than silently doing nothing.
    /// </remarks>
    private void OnDeath(DeathPacket death)
    {
        string killer = string.IsNullOrEmpty(death.KilledBy) ? "something" : death.KilledBy;
        Died?.Invoke($"Killed by {killer} at level {_map.Player?.Level ?? 0}.");
    }

    /// <summary>The server's authoritative word on damage, overriding any local prediction.</summary>
    private void OnDamage(DamagePacket damage)
    {
        var target = _map.GetEntity(damage.TargetId);
        if (target == null)
            return;

        target.Hp -= damage.DamageAmount;

        foreach (var effect in damage.Effects)
            target.Conditions |= effect.ToFlag();

        if (damage.Kill)
            target.Dead = true;
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
        ApplyAttackInput(now);
        _combat.Update(now);
        _interaction.Update(now);
        _hud?.ShowPrompt(_interaction.Current.Exists ? _interaction.Current.Label : null);
        _hud?.ShowContainer(OpenContainer);

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

        _hud?.Refresh(_map.Player);
        _minimap?.Refresh(_cameraAngle);

        Draw(now);
    }

    /// <summary>The container in reach, if any. Its panel appears and disappears with proximity.</summary>
    private Entity OpenContainer =>
        _interaction.Current is { Kind: InteractionKind.Container, Entity: { } entity } ? entity : null;

    /// <summary>
    /// A click on one of our own slots: use it, or move it into an open container.
    /// </summary>
    private void OnSlotActivated(int slotIndex)
    {
        var container = OpenContainer;
        if (container != null)
        {
            _inventory.PutInto(container, slotIndex);
            return;
        }

        _inventory.Activate(slotIndex);
    }

    private void OnContainerSlotActivated(int slotIndex)
    {
        var container = OpenContainer;
        if (container != null)
            _inventory.TakeFrom(container, slotIndex);
    }

    /// <summary>Sends a chat line, or a slash command, exactly as typed.</summary>
    private void OnChatSubmitted(string line) => _session.Send(new PlayerTextPacket { Text = line });

    private void ApplyInput(int deltaMs)
    {
        var player = _map.Player;
        if (player == null)
            return;

        // While the chat box has the keyboard, the movement keys belong to it -- they are letters.
        if (_chat is { IsTyping: true })
        {
            player.SetInput(0f, 0f, 0f);

            if (Input.IsKeyPressed(Key.Escape))
                _chat.EndTyping();
            return;
        }

        if (Input.IsActionJustPressed("toggle_chat"))
        {
            _chat?.BeginTyping();
            return;
        }

        float x = Input.GetActionStrength("move_right") - Input.GetActionStrength("move_left");
        float y = Input.GetActionStrength("move_down") - Input.GetActionStrength("move_up");
        float rotate = Input.GetActionStrength("rotate_right") - Input.GetActionStrength("rotate_left");

        player.SetInput(x, y, rotate);

        if (player.InputRotate != 0f)
            _cameraAngle += deltaMs * LocalPlayer.RotateSpeed * player.InputRotate;

        if (Input.IsActionJustPressed("reset_camera"))
            _cameraAngle = 7f * Mathf.Pi / 4f;

        if (Input.IsActionJustPressed("autofire"))
            _autofire = !_autofire;

        if (Input.IsActionJustPressed("interact"))
            Interact();

        if (Input.IsActionJustPressed("nexus"))
            NexusRequested?.Invoke();

        if (Input.IsActionJustPressed("health_potion"))
            _inventory.UsePotion(health: true);

        if (Input.IsActionJustPressed("magic_potion"))
            _inventory.UsePotion(health: false);
    }

    /// <summary>Acts on whatever the player is standing next to.</summary>
    private void Interact()
    {
        var target = _interaction.Current;
        if (!target.Exists)
            return;

        switch (target.Kind)
        {
            case InteractionKind.Portal:
                // The answer arrives as a Reconnect, which tears this session down and stands a new
                // one up against the destination world.
                _session.Send(new UsePortalPacket { ObjectId = target.Entity.ObjectId });
                break;

            case InteractionKind.Container:
                // The panel is already showing whenever one is in reach, so the key has nothing
                // left to do.
                break;

            case InteractionKind.Merchant:
                _chat?.AddSystem($"{target.Label} is not implemented yet.");
                break;
        }
    }

    /// <summary>
    /// Fires at the cursor while the button is held, or continuously when autofire is on.
    /// </summary>
    /// <remarks>
    /// The aim angle is the cursor's offset from the screen centre, turned back into world space
    /// through the camera's rotation -- so aiming stays where the cursor points as the world turns.
    /// </remarks>
    private void ApplyAttackInput(int now)
    {
        if (_map.Player == null)
            return;

        if (!_autofire && !Input.IsActionPressed("shoot"))
            return;

        var viewport = GetViewport();
        var centre = viewport.GetVisibleRect().Size / 2f;
        var offset = viewport.GetMousePosition() - centre;

        var world = _world.Projection.ScreenToWorldOffset(offset);

        // A cursor sitting exactly on the player gives no direction; fire along the camera's
        // rightward axis rather than doing nothing.
        float angle = world.LengthSquared() < 0.0001f
            ? _cameraAngle
            : Mathf.Atan2(world.Y, world.X);

        _combat.TryShoot(now, angle);
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
        DrawProjectiles(now);
        DrawOverlay();

        // One upload for however many tiles were baked while sweeping the visible area.
        _tileAtlas.Flush();

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

                // Blended artwork where the tile borders something that outranks it, and the plain
                // sheet sprite otherwise.
                var sprite = _tileBlender.Blend(_map, x, y, square);
                if (!sprite.IsValid)
                {
                    // Random-variant terrain picks by position, so a field of grass is varied but
                    // each tile is stable.
                    sprite = _textures.Resolve(square.Desc.Texture, x * 31 + y * 17).Still;
                }

                if (!sprite.IsValid)
                    continue;

                _world.Ground.Add(new GroundDraw
                {
                    TileX = x,
                    TileY = y,
                    Sprite = sprite,
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
                Tint = TintFor(entity),
                Outlined = true,
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

            AddShadow(entity, draw.SortBias);
            _world.Sprites.Add(draw);
        }
    }

    /// <summary>
    /// Puts a soft ellipse under an entity so it reads as standing on the ground rather than
    /// floating above it.
    /// </summary>
    /// <remarks>
    /// Screen-aligned rather than laid flat, which sounds wrong and is not: under this projection
    /// the ground plane is unforeshortened, so an ellipse on it and an ellipse on the screen are
    /// the same shape. The original drew exactly that, a 30-by-15 pixel radial gradient.
    ///
    /// Anything flying gets one too, and it stays on the ground while the sprite rises, which is
    /// what conveys the height at all.
    /// </remarks>
    private void AddShadow(Entity entity, float sortBias)
    {
        if (entity.Desc.ShadowSize <= 0 || entity.Desc.DrawUnder)
            return;

        var shadow = _shadowTexture ??= BuildShadowTexture();

        // The original scales by both the entity's size and its declared shadow size, so a giant
        // and its shadow grow together.
        float scale = entity.Size / 100f * (entity.Desc.ShadowSize / 100f);

        _world.Sprites.Add(new SpriteDraw
        {
            TileX = entity.X,
            TileY = entity.Y,
            Height = 0f,
            Sprite = new Sprite(shadow, new Rect2I(0, 0, ShadowTextureSize, ShadowTextureSize)),
            WidthTiles = 60f / WorldProjection.PixelsPerTile * scale,
            HeightTiles = 30f / WorldProjection.PixelsPerTile * scale,
            AnchorX = 0.5f,
            AnchorY = 0.5f,
            Modulate = new Color(0f, 0f, 0f, 0.5f),
            Outlined = false,
            // Beneath the entity it belongs to, and beneath anything else on the same tile.
            SortBias = sortBias - 2f,
        });
    }

    private const int ShadowTextureSize = 32;
    private Texture2D _shadowTexture;

    /// <summary>A soft radial blob, generated once rather than shipped as an asset.</summary>
    private static Texture2D BuildShadowTexture()
    {
        var image = Image.CreateEmpty(ShadowTextureSize, ShadowTextureSize, false, Image.Format.Rgba8);
        float centre = (ShadowTextureSize - 1) / 2f;

        for (int y = 0; y < ShadowTextureSize; y++)
        {
            for (int x = 0; x < ShadowTextureSize; x++)
            {
                float dx = (x - centre) / centre;
                float dy = (y - centre) / centre;
                float distance = Mathf.Sqrt(dx * dx + dy * dy);

                // Squared falloff, which reads as a soft edge rather than a disc with a fringe.
                float alpha = Mathf.Clamp(1f - distance, 0f, 1f);
                image.SetPixel(x, y, new Color(1f, 1f, 1f, alpha * alpha));
            }
        }

        return ImageTexture.CreateFromImage(image);
    }

    /// <summary>
    /// Positions the health bars and name plates over whichever entities warrant them.
    /// </summary>
    /// <remarks>
    /// The original decided this per frame by reading three pixels out of the finished sprite and
    /// testing their alpha -- a texture read-back per visible object, every frame, to answer a
    /// question the object's own definition already knew.
    /// </remarks>
    private void DrawOverlay()
    {
        if (_overlay == null)
            return;

        _overlay.Clear();

        foreach (var entity in _map.Entities)
        {
            var desc = entity.Desc;
            if (desc == null || entity.Square is not { IsKnown: true } || entity.Dead)
                continue;

            bool combatant = desc.IsEnemy || desc.IsPlayer;
            bool named = desc.ShowName && !string.IsNullOrEmpty(entity.Name);

            // Nothing to say about a plain decoration.
            if (!combatant && !named)
                continue;

            // An enemy that cannot be hurt should not advertise a health bar, and an invisible one
            // should not advertise anything at all.
            bool showBar = combatant
                           && !entity.IsInvisible
                           && !entity.IsInvulnerable
                           && !desc.NoMiniMap
                           && entity.MaxHp > 0;

            if (!showBar && !named)
                continue;

            var scene = _world.Projection.ToScene(entity.X, entity.Y, entity.Z);
            _overlay.Add(new OverlayItem
            {
                Anchor = _world.Unproject(scene),
                Name = named ? entity.Name : null,
                NameColor = desc.IsPlayer ? new Color(0.99f, 0.87f, 0f) : Colors.White,
                Hp = entity.Hp,
                MaxHp = entity.MaxHp,
                ShowHealthBar = showBar,
            });
        }

        _overlay.Commit();
    }

    /// <summary>
    /// Draws projectiles in flight.
    /// </summary>
    /// <remarks>
    /// Projectiles carry their own rotation: most sprites are drawn pointing along their direction
    /// of travel, offset by whatever correction the artwork needs. That rotation is baked into the
    /// quad rather than the texture, so no sprite has to be redrawn per angle -- which is what the
    /// original did, caching a bitmap per rotation step.
    /// </remarks>
    private void DrawProjectiles(int now)
    {
        foreach (var projectile in _combat.Projectiles)
        {
            if (projectile.Desc == null)
                continue;

            var resolved = _textures.Resolve(projectile.Desc.Texture, projectile.ObjectId);
            if (!resolved.Still.IsValid)
                continue;

            var draw = new SpriteDraw
            {
                TileX = projectile.X,
                TileY = projectile.Y,
                Height = projectile.Z,
                Sprite = resolved.Still,
                AnchorX = 0.5f,
                Modulate = Colors.White,
                Outlined = true,
                // Slightly above whatever they are flying over, so a bullet is never swallowed by
                // the sprite it is about to hit.
                SortBias = 0.5f,
            };

            SizeQuad(ref draw, projectile, resolved.Still, 1, 1);
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

    /// <summary>
    /// The whole-sprite colour treatment a status effect calls for.
    /// </summary>
    /// <remarks>
    /// Greyscale reads as "this thing is not currently participating", which covers paused, stasis
    /// and petrified alike. Curse gets its own red wash so it is distinguishable at a glance.
    /// </remarks>
    private static SpriteTint TintFor(Entity entity)
    {
        if (entity.Has(ConditionEffects.Curse))
            return SpriteTint.Red;

        if (entity.IsPaused || entity.IsStasis || entity.IsPetrified)
            return SpriteTint.Greyscale;

        return SpriteTint.None;
    }

    private static Color Modulate(Entity entity)
    {
        // Invisible players are drawn faintly rather than hidden, so allies can still be followed.
        if (entity.IsInvisible)
            return new Color(1f, 1f, 1f, 0.7f);

        return Colors.White;
    }
}

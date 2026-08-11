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
    private Party _party;
    private ParticleSystem _particles;
    private Audio.AudioLibrary _audio;
    private SpritePalette _palette;
    private MinimapView _minimap;
    private TileColors _tileColors;
    private TileAtlas _tileAtlas;
    private TileBlender _tileBlender;
    private ModelLibrary _models;
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

    /// <summary>Uses the ability on a loop, for unattended runs. See LaunchOptions.</summary>
    public bool AutoAbility { get; set; }

    /// <summary>Walks in a circle, for unattended runs. See LaunchOptions.</summary>
    public bool AutoWalk { get; set; }

    /// <summary>Overrides the starting camera heading, in radians. See LaunchOptions.</summary>
    public float? StartingCameraAngle { set { if (value.HasValue) _cameraAngle = value.Value; } }

    private int _nextAutoAbilityMs;

    /// <summary>
    /// Lines to send once the world is up, one at a time. For unattended runs; see LaunchOptions.
    /// </summary>
    /// <remarks>
    /// The queue is owned by the scene and shared across worlds, so a line is sent once for the
    /// whole session rather than once per world -- and a script that has to change world partway
    /// through carries on where it left off after the reconnect.
    ///
    /// Delayed rather than sent the instant a player entity appears: the server drops chat from a
    /// client it does not yet consider fully in the world, and does so silently.
    /// </remarks>
    public System.Collections.Generic.Queue<string> ScriptedLines { private get; set; }

    /// <summary>When the next scripted line is due, or zero before the clock has been read.</summary>
    private int _sayAtMs;

    /// <summary>
    /// Raised when the player asks to return to the Nexus.
    /// </summary>
    /// <remarks>
    /// Deliberately not the Escape packet. This fork's server disconnects anyone who sends Escape
    /// while already in the Nexus, and the original client stopped sending it entirely -- it
    /// performs a local reconnect to game id -2 instead.
    /// </remarks>
    public event System.Action NexusRequested;

    /// <summary>Raised when the player asks for the options panel.</summary>
    public event System.Action OptionsToggled;

    /// <summary>Raised when the player asks for the guild panel.</summary>
    public event System.Action GuildToggled;

    /// <summary>Whether the options panel is up, so input can be held back while it is.</summary>
    public System.Func<bool> OptionsAreOpen { private get; set; }

    /// <summary>
    /// Raised when our character dies, carrying the packet itself.
    /// </summary>
    /// <remarks>
    /// The packet rather than a formatted message, because the account and character ids on it are
    /// what the fame tally is fetched with — and in a session that connected straight to a world
    /// they are the only place the client learns its own numeric account id.
    /// </remarks>
    public event System.Action<DeathPacket> Died;

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
        _models = new ModelLibrary(assets.Manifest);
        _world = world;
        _clock = clock;
        _map = new GameMap(data);
        _minimap?.Configure(_map, _tileColors);
        _combat = new Combat(_map, data, session, clock);
        _interaction = new Interaction(_map);
        _inventory = new Inventory(_map, data, session, clock);
        _inventory.UseCombat(_combat);
        _trading = new Trading(session);
        _party = new Party(_map);
        _particles = new ParticleSystem(_map);
        _audio = App.ServiceLocator.Audio;
        _palette = new SpritePalette();

        // Subscribed after construction, not alongside the field assignments above: these forward
        // to objects that do not exist until the map does.
        if (_chat != null)
            _chat.Submitted += OnChatSubmitted;

        if (_hud != null)
        {
            _hud.SlotActivated += OnSlotActivated;
            _hud.ContainerSlotActivated += OnContainerSlotActivated;
            _hud.BuyPressed += OnBuyPressed;
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
        _audio?.PlayMusic(mapInfo.Music);
        _map.Reset(mapInfo.Width, mapInfo.Height, mapInfo.Name);
        _combat.Clear();
        _particles.Clear();
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

    /// <summary>The level we last saw, so a level-up can be noticed rather than announced.</summary>
    private int _lastLevel = -1;

    /// <summary>Plays the level-up chime when the server raises our level.</summary>
    private void NoticeLevelUp()
    {
        int level = _map.Player?.Level ?? -1;
        if (level > _lastLevel && _lastLevel > 0)
            _audio?.PlayEffect("level_up");

        _lastLevel = level;
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

        // The chime that says something dropped. Only for bags that appear near enough to be worth
        // hearing about; the server streams every container in the world as it comes into view.
        if (desc?.Class == "Container" && _map.Player != null && entity.DistanceTo(_map.Player.X, _map.Player.Y) < 12f)
            _audio?.PlayEffect("loot_appears");

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
                OnAllyShoot(shot, now);
                break;

            case DamagePacket damage:
                OnDamage(damage);
                break;

            case ShowEffectPacket effect:
                _particles.Show(effect, now);
                break;

            case QuestObjIdPacket quest:
                _questObjectId = quest.ObjectId;
                break;

            case PlaySoundPacket sound:
                PlayObjectSound(sound);
                break;

            case SwitchMusicPacket music:
            _audio?.PlayMusic(music.Music);
                break;

            case TradeRequestedPacket:
            case TradeStartPacket:
            case TradeChangedPacket:
            case TradeAcceptedPacket:
            case TradeDonePacket:
                _trading.Handle(packet);
                break;

            case AccountListPacket list when list.ListId == AccountListId.Starred:
                _party.SetStarred(list.AccountIds);
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

    /// <summary>
    /// Another player fired.
    /// </summary>
    /// <remarks>
    /// Drawn but inert: the projectile exists on the server, where its owner's client is the one
    /// that reports what it hits, so joining in would double the damage reported for it. There is
    /// no acknowledgement either. It spawns from the shooter's current position rather than one
    /// carried on the wire, which the packet does not have room for.
    /// </remarks>
    private void OnAllyShoot(AllyShootPacket shot, int now)
    {
        var owner = _map.GetEntity(shot.OwnerId);
        if (owner == null || owner.Dead)
            return;

        owner.SetAttack(shot.Angle, now);

        var container = _data.GetObject((ushort)shot.ContainerType);
        if (container?.Projectiles == null || !container.Projectiles.TryGetValue(0, out var desc))
            return;

        _combat.SpawnCosmetic(desc, (ushort)shot.ContainerType, shot.OwnerId, shot.BulletId,
            shot.Angle, owner.X, owner.Y, now);
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
    private void OnDeath(DeathPacket death) => Died?.Invoke(death);

    /// <summary>The level our own character reached, for the death screen.</summary>
    public int PlayerLevel => _map?.Player?.Level ?? 0;

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

        // Only things that fight make a noise when they are hit. A struck decoration is silent, and
        // so is a scratch that did nothing.
        if (target.Desc is { IsEnemy: true } or { IsPlayer: true })
        {
            if (damage.Kill)
                _audio?.PlayEffect(target.Desc.DeathSound);
            else if (damage.DamageAmount > 0)
                _audio?.PlayEffect(target.Desc.HitSound);
        }

        ThrowDebris(target, damage);
    }

    /// <summary>
    /// The spray of a struck object's own colours.
    /// </summary>
    /// <remarks>
    /// Thrown away from the shot when the projectile responsible is still in flight, and in every
    /// direction otherwise — which covers deaths, ground damage and anything that connected locally
    /// before the server's word arrived. Both are the original's behaviour, which chose between them
    /// on whether it had a projectile to hand.
    /// </remarks>
    private void ThrowDebris(Entity target, DamagePacket damage)
    {
        if (target.Desc == null)
            return;

        var resolved = _textures.Resolve(target.Desc.Texture, target.ObjectId, target.AltTextureIndex);

        // Any one frame will do: a character's palette is the same whichever way it is facing.
        var sprite = resolved.Animated != null
            ? resolved.Animated.Frame(0f, 0f, CharAction.Stand, 0f).Sprite
            : resolved.Still;

        if (!sprite.IsValid)
            return;

        var palette = _palette.For(target.ObjectType, sprite, target.Desc.BloodProb, target.Desc.BloodColor);

        if (damage.Kill)
        {
            _particles.Explode(target.X, target.Y, palette, target.Size, count: 30);
            return;
        }

        var projectile = _combat.Find(damage.ObjectId, damage.BulletId);
        if (projectile?.ProjectileDesc != null)
        {
            _particles.Hit(target.X, target.Y, palette, target.Size, count: 10,
                projectile.Angle, projectile.ProjectileDesc.Speed);
            return;
        }

        _particles.Explode(target.X, target.Y, palette, target.Size, count: 10);
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
        _particles.Update(now, deltaMs);
        _interaction.Update(now);
        _party.Update(now);
        _hud?.ShowPrompt(_interaction.Current.Exists ? _interaction.Current.Label : null);
        _hud?.ShowContainer(OpenContainer);
        _hud?.ShowMerchant(NearbyMerchant, _map.Player);
        _hud?.ShowParty(_party.Members);

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
        SayOnEntryIfDue(player, now);

        NoticeLevelUp();
        _hud?.Refresh(_map.Player);
        _minimap?.Refresh(_cameraAngle);

        Draw(now);
    }

    /// <summary>The container in reach, if any. Its panel appears and disappears with proximity.</summary>
    private Entity OpenContainer =>
        _interaction.Current is { Kind: InteractionKind.Container, Entity: { } entity } ? entity : null;

    /// <summary>The vendor in reach, if any.</summary>
    private Entity NearbyMerchant =>
        _interaction.Current is { Kind: InteractionKind.Merchant, Entity: { } entity } ? entity : null;

    /// <summary>
    /// Buys whatever the nearby vendor is selling.
    /// </summary>
    /// <remarks>
    /// The quantity field is parsed and then ignored by the server, so one press buys one item
    /// whatever is put there. The outcome arrives as a BuyResult and is reported in chat.
    /// </remarks>
    private void OnBuyPressed()
    {
        var merchant = NearbyMerchant;
        if (merchant != null)
            _session.Send(new BuyPacket { ObjectId = merchant.ObjectId, Quantity = 1 });
    }

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

    /// <summary>Sends the next scripted line once the world has settled. See <see cref="ScriptedLines"/>.</summary>
    private void SayOnEntryIfDue(LocalPlayer player, int now)
    {
        const int SettleMs = 2000;
        const int GapMs = 2000;

        if (ScriptedLines == null || ScriptedLines.Count == 0 || player == null)
            return;

        if (_sayAtMs == 0)
        {
            _sayAtMs = now + SettleMs;
            return;
        }

        if (now < _sayAtMs)
            return;

        OnChatSubmitted(ScriptedLines.Dequeue());
        _sayAtMs = now + GapMs;
    }

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

        if (Input.IsActionJustPressed("options"))
        {
            OptionsToggled?.Invoke();
            return;
        }

        if (Input.IsActionJustPressed("guild"))
        {
            GuildToggled?.Invoke();
            return;
        }

        // With the panel up the keyboard belongs to it, but movement should stop rather than
        // continue in whatever direction was last held.
        if (OptionsAreOpen != null && OptionsAreOpen())
        {
            player.SetInput(0f, 0f, 0f);
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

        if (AutoWalk)
        {
            // A slow circle, so the walker stays near where it started and keeps changing heading.
            float phase = _clock.FrameMs / 3000f * Mathf.Tau;
            x = Mathf.Cos(phase);
            y = Mathf.Sin(phase);
        }

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
                // The panel is already showing whenever a vendor is in reach.
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
        var player = _map.Player;
        if (player == null)
            return;

        // Chat has the keyboard, so the ability key is a letter being typed. The mouse still works.
        bool typing = _chat is { IsTyping: true };

        if (AutoAbility && now >= _nextAutoAbilityMs)
        {
            _nextAutoAbilityMs = now + 1500;

            // A fixed point a few tiles away rather than the cursor: an unattended run has the
            // pointer wherever the window opened, which is usually a corner and often outside the
            // ability's reach.
            var auto = new Vector2(player.X + 3f, player.Y);
            _inventory.BeginAbility(now, auto.X, auto.Y);
            _inventory.EndAbility(now, auto.X, auto.Y);
        }

        if (!typing && Input.IsActionJustPressed("use_ability"))
        {
            var target = AimPoint();
            var ability = _inventory.EquippedAbility;

            if (_inventory.BeginAbility(now, target.X, target.Y))
                PlayItemSound(ability);
            else if (ability != null && player.Mp < ability.MpCost)
                _audio?.PlayEffect("no_mana");
        }

        // Released even while typing: a key-up that arrives after the chat box took focus would
        // otherwise leave a multi-phase ability charging forever.
        if (Input.IsActionJustReleased("use_ability"))
        {
            var target = AimPoint();
            _inventory.EndAbility(now, target.X, target.Y);
        }

        if (typing || (OptionsAreOpen != null && OptionsAreOpen()))
            return;

        if (!_autofire && !Input.IsActionPressed("shoot"))
            return;

        if (_combat.TryShoot(now, AimAngle()))
            PlayItemSound(_combat.EquippedWeapon);
    }

    /// <summary>
    /// Plays the noise an item makes when it is fired.
    /// </summary>
    /// <remarks>
    /// Quieter than everything else, at three quarters, which is the original's figure — a weapon
    /// fires several times a second and at full volume it drowns out the things it is shooting at.
    /// </remarks>
    private void PlayItemSound(ObjectDesc item)
    {
        if (item?.Sounds != null && item.Sounds.TryGetValue(0, out string sound))
            _audio?.PlayEffect(sound, 0.75f);
    }

    /// <summary>Plays one of an entity's declared sounds, named by index on the wire.</summary>
    private void PlayObjectSound(PlaySoundPacket packet)
    {
        var owner = _map.GetEntity(packet.OwnerId);
        if (owner?.Desc?.Sounds != null && owner.Desc.Sounds.TryGetValue(packet.SoundId, out string sound))
            _audio?.PlayEffect(sound);
    }

    /// <summary>The direction from the player to the cursor, in world radians.</summary>
    private float AimAngle()
    {
        var world = CursorOffset();

        // A cursor sitting exactly on the player gives no direction; fire along the camera's
        // rightward axis rather than doing nothing.
        return world.LengthSquared() < 0.0001f
            ? _cameraAngle
            : Mathf.Atan2(world.Y, world.X);
    }

    /// <summary>
    /// Where the cursor is pointing, in tiles.
    /// </summary>
    /// <remarks>
    /// Abilities are aimed at a point rather than along a direction: several of them land where they
    /// are pointed, and the server refuses any aimed further than its own limit.
    /// </remarks>
    private Vector2 AimPoint()
    {
        var player = _map.Player;
        var offset = CursorOffset();
        return new Vector2(player.X + offset.X, player.Y + offset.Y);
    }

    /// <summary>The cursor's offset from the player, in tiles.</summary>
    private Vector2 CursorOffset()
    {
        var viewport = GetViewport();
        var centre = viewport.GetVisibleRect().Size / 2f;
        return _world.Projection.ScreenToWorldOffset(viewport.GetMousePosition() - centre);
    }

    // ------------------------------------------------------------------------------------------
    // Drawing
    // ------------------------------------------------------------------------------------------

    private void Draw(int now)
    {
        var focus = _focus ?? _map.Player;
        if (focus == null)
            return;

        // An earthquake displaces the camera rather than the world, so everything in it stays
        // consistent with everything else -- including the health bars and name plates, which are
        // positioned by unprojecting through this same camera.
        float shakeX = 0f;
        float shakeY = 0f;
        if (_particles.Jitter > 0f)
        {
            shakeX = (float)GD.RandRange(-_particles.Jitter, _particles.Jitter);
            shakeY = (float)GD.RandRange(-_particles.Jitter, _particles.Jitter);
        }

        _world.Configure(focus.X + shakeX, focus.Y + shakeY, _cameraAngle);

        float radius = _world.VisibleRadius();
        DrawGround(focus, radius, now);
        DrawEntities(now, _cameraAngle);
        DrawProjectiles(now);
        DrawParticles();
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
                {
                    ReportBlankTerrain(square);
                    continue;
                }

                if (square.Desc.HasEdge && _blankTerrainReported.Add(square.TileType))
                    GD.Print($"[diag] edge terrain {square.TileType} {square.Desc.Id} sameType={square.Desc.SameTypeEdgeMode} corner={square.Desc.CornerTexture != null} inner={square.Desc.InnerCornerTexture != null}");

                _world.Ground.Add(new GroundDraw
                {
                    TileX = x,
                    TileY = y,
                    Sprite = sprite,
                    UvOffset = TileOffset(square.Desc, x, y, now),
                    Modulate = Colors.White,
                });
            }
        }
    }

    /// <summary>Terrain types already complained about, so the warning is one line and not a flood.</summary>
    private readonly System.Collections.Generic.HashSet<ushort> _blankTerrainReported = new();

    /// <summary>
    /// Says so, once, when a terrain type has no artwork to draw.
    /// </summary>
    /// <remarks>
    /// Terrain that resolves to nothing leaves a hole the background shows through, which on a dark
    /// background looks like a deliberate black tile rather than a fault. Worth a line in the log:
    /// it means the type is missing from the sheets, or its texture entry names something that is
    /// not there.
    /// </remarks>
    private void ReportBlankTerrain(Square square)
    {
        if (!_blankTerrainReported.Add(square.TileType))
            return;

        GD.PushWarning(
            $"[world] terrain type {square.TileType} ({square.Desc?.Id ?? "unnamed"}) has no artwork; " +
            "those tiles will be blank.");
    }

    /// <summary>
    /// How far a tile's artwork is slid within its own rectangle, in fractions of a tile.
    /// </summary>
    /// <remarks>
    /// Three things can move it, and only one of them moves over time. A tile can declare a fixed
    /// offset; a tile marked RandomOffset takes a whole-texel one that has to be the same every
    /// frame or the ground crawls; and an animated tile ripples or scrolls on top of that.
    ///
    /// The wrapping this depends on lives in the ground shader. Sliding past the edge of the tile's
    /// rectangle would otherwise sample whatever sprite sits next to it on the sheet.
    /// </remarks>
    private static Vector2 TileOffset(GroundDesc desc, int tileX, int tileY, int now)
    {
        var offset = desc.RandomOffset ? RandomOffset(tileX, tileY) : new Vector2(desc.XOffset, desc.YOffset);

        float seconds = now / 1000f;
        return desc.Animation switch
        {
            GroundAnimation.Wave => offset + new Vector2(
                Mathf.Sin(desc.AnimationDx * seconds),
                Mathf.Sin(desc.AnimationDy * seconds)),

            GroundAnimation.Flow => offset + new Vector2(
                desc.AnimationDx * seconds,
                desc.AnimationDy * seconds),

            _ => offset,
        };
    }

    /// <summary>
    /// A whole-texel offset for a tile that wants its artwork jumbled.
    /// </summary>
    /// <remarks>
    /// Derived from the tile's position rather than drawn at random, so it is the same on every
    /// frame and after every reconnect. The original rolled it once when the square was built and
    /// kept it on the square; deriving it needs no storage and survives the square being rebuilt.
    /// </remarks>
    private static Vector2 RandomOffset(int tileX, int tileY)
    {
        const int Texels = 8;

        int hash = tileX * 73856093 ^ tileY * 19349663;
        return new Vector2(
            (hash >>> 3 & (Texels - 1)) / (float)Texels,
            (hash >>> 11 & (Texels - 1)) / (float)Texels);
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

            // A few objects are real geometry rather than a picture of it. Those are drawn as
            // geometry and their sprite becomes the texture on it, so the flat quad is skipped.
            if (AddModel(entity, draw))
                continue;

            AddShadow(entity, draw.SortBias);
            _world.Sprites.Add(draw);
            AddFlash(entity, draw, now);
        }
    }

    /// <summary>
    /// Lays the flash colour over an entity that is pulsing.
    /// </summary>
    /// <remarks>
    /// The same quad again, in the flash colour, at the strength the pulse currently calls for.
    /// Ordinary alpha blending then gives <c>sprite·(1-s) + colour·s</c>, which is exactly the
    /// colour transform the original computed — except that it did so by cloning the finished
    /// bitmap and transforming the clone, once per frame for as long as the flash lasted.
    /// </remarks>
    private void AddFlash(Entity entity, SpriteDraw draw, int now)
    {
        float strength = entity.FlashStrength(now, out int color);
        if (strength <= 0f)
            return;

        draw.Modulate = new Color(
            (color >> 16 & 0xFF) / 255f,
            (color >> 8 & 0xFF) / 255f,
            (color & 0xFF) / 255f,
            strength);

        // No tint under the flash: the flash colour is the whole point of it.
        draw.Tint = SpriteTint.None;
        draw.SortBias += 0.5f;
        _world.Sprites.Add(draw);
    }

    /// <summary>
    /// Draws an entity as real geometry, if its definition names a model.
    /// </summary>
    /// <remarks>
    /// The object's own sprite becomes the texture on it, which is how a brick pillar is a pillar
    /// made of that brick. Its declared rotation is in eighths of a turn like every other angle in
    /// the game data.
    /// </remarks>
    /// <returns>Whether the entity was drawn as a model, in which case it needs no quad.</returns>
    private bool AddModel(Entity entity, in SpriteDraw draw)
    {
        if (string.IsNullOrEmpty(entity.Desc.Model))
            return false;

        var model = _models.Get(entity.Desc.Model);
        if (model == null)
            return false;

        var colour = entity.Desc.Color;

        _world.Models.Add(new ModelDraw
        {
            Model = model,
            TileX = entity.X,
            TileY = entity.Y,
            Rotation = entity.Desc.Rotation,
            Sprite = draw.Sprite,
            SolidColor = new Color(
                (colour >> 16 & 0xFF) / 255f,
                (colour >> 8 & 0xFF) / 255f,
                (colour & 0xFF) / 255f),
            SortBias = draw.SortBias,
        });

        return true;
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
    /// <summary>The entity the server has named as the current objective, or zero for none.</summary>
    private int _questObjectId;

    private void DrawOverlay()
    {
        if (_overlay == null)
            return;

        _overlay.Clear();
        _overlay.SetQuestMarker(QuestMarkerPosition());

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
    /// Where the quest objective is on screen, or null when there is none to point at.
    /// </summary>
    /// <remarks>
    /// The server names the objective once, by object id, and never withdraws it -- so a target
    /// that dies or is left behind in another world simply stops being in the map, which is what
    /// clears the marker here.
    /// </remarks>
    private Vector2? QuestMarkerPosition()
    {
        if (_questObjectId == 0)
            return null;

        var target = _map.GetEntity(_questObjectId);
        if (target == null || target.Dead)
            return null;

        return _world.Unproject(_world.Projection.ToScene(target.X, target.Y, target.Z));
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
    /// Draws every live particle.
    /// </summary>
    /// <remarks>
    /// All of them share one texture and therefore one surface, however many effects are running —
    /// the colour rides on the vertex tint. The original appended three display-list entries per
    /// particle per frame and baked a bitmap for every colour and size combination it saw.
    /// </remarks>
    private void DrawParticles()
    {
        // The original's size unit: a hundred is five screen pixels.
        const float PixelsPerSizeUnit = 5f / 100f;

        var sprite = ParticleTexture.Sprite;

        foreach (var particle in _particles.Particles)
        {
            if (particle.Size <= 0f)
                continue;

            float tiles = particle.Size * PixelsPerSizeUnit / WorldProjection.PixelsPerTile
                          * ParticleTexture.QuadToCore;

            _world.Sprites.Add(new SpriteDraw
            {
                TileX = particle.X,
                TileY = particle.Y,
                Height = particle.Z,
                Sprite = sprite,
                WidthTiles = tiles,
                HeightTiles = tiles,

                // Centred on the point rather than standing on it: a particle is a mote in the air,
                // not something with its feet on a tile.
                AnchorX = 0.5f,
                AnchorY = 0.5f,
                Modulate = new Color(
                    (particle.Color >> 16 & 0xFF) / 255f,
                    (particle.Color >> 8 & 0xFF) / 255f,
                    (particle.Color & 0xFF) / 255f),
                Outlined = false,

                // Above whatever they are playing over, so an effect on a monster is not swallowed
                // by the monster.
                SortBias = 0.25f,
            });
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

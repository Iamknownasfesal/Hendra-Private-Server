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
    private VaultStore _vault;
    private Inventory _inventory;
    private Trading _trading;
    private Party _party;
    private ParticleSystem _particles;
    private Audio.AudioLibrary _audio;
    private SpritePalette _palette;
    private MinimapView _minimap;
    private TileAtlas _tileAtlas;
    private TileBlender _tileBlender;
    private ModelLibrary _models;
    private WorldOverlay _overlay;
    private TileColors _tileColors;
    private HudView _hud;
    private ChatView _chat;

    /// <summary>Localised strings, shared with the rest of the client.</summary>
    private StringMap _strings = new();

    /// <summary>The eight carried slots that answer to the number keys.</summary>
    private const int InventoryHotkeys = 8;

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

    /// <summary>Raised by the debug key, which shows what the machine and the line are doing.</summary>
    public event System.Action DebugToggled;

    /// <summary>Raised when the player asks for the guild panel.</summary>
    public event System.Action GuildToggled;

    /// <summary>Whether the interface is hidden. Bound to I, as the original bound its inventory.</summary>
    public bool HudHidden { get; private set; }

    /// <summary>Raised when the interface is hidden or shown.</summary>
    public event System.Action<bool> HudVisibilityChanged;

    /// <summary>Raised when the character sheet should open or close.</summary>
    public event System.Action CharacterToggled;

    /// <summary>Raised when the account panel should open or close.</summary>
    public event System.Action AccountToggled;

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
        _overlay?.Configure(new SheetConditionIcons(assets));

        // The overlay draws in screen space but the numbers belong to places in the world, so it is
        // given the projection rather than a snapshot of where things were when they were made.
        if (_overlay != null)
            _overlay.Project = (x, y, z) => _world.Unproject(_world.Projection.ToScene(x, y, z));
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
        _combat.Struck += OnProjectileStruck;
        _combat.Damaged += OnDamageDealt;
        _combat.Fired += (shot, x, y, angle) => Muzzle(shot, x, y, angle);
        _interaction = new Interaction(_map);
        _vault = new VaultStore(_session, _data);
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
            _hud.VaultSlotActivated += OnVaultSlotActivated;
            _hud.VaultPurchaseRequested += () => _vault.Buy();
            _hud.SlotDropped += OnSlotDropped;
            _hud.SlotDroppedOutside += OnSlotDroppedOutside;
            _hud.PotionRequested += health => _inventory.UsePotion(health);
            _hud.NexusPressed += () => NexusRequested?.Invoke();
            _hud.OptionsPressed += () => OptionsToggled?.Invoke();
            _hud.BuyPressed += OnBuyPressed;

            // The card's own buttons open the panels they name. Routed through the controller
            // rather than wired straight to a panel inside the view, so what a button does is
            // decided in one place and can be changed without opening the interface.
            _hud.AccountPressed += () => AccountToggled?.Invoke();
            _hud.StatsPressed += () => CharacterToggled?.Invoke();
            _hud.PartyMemberActivated += who => _chat?.BeginTyping($"/tell {who} ");

            // Four buttons the reference has and this server does not answer. Saying so is better
            // than a button that swallows a click and does nothing, which reads as a broken client.
            _hud.ShopPressed += () => Unavailable("The shop");
            _hud.NewsPressed += () => Unavailable("News");
            _hud.SwapPressed += () => Unavailable("Swapping loadouts");
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

    /// <summary>Raised when a world is named, so the loading screen can say where you are going.</summary>
    public event System.Action<string, int> WorldEntering;

    /// <summary>Raised once the player is standing in it.</summary>
    public event System.Action WorldEntered;

    private bool _announcedArrival;

    private void OnMapLoaded(MapInfoPacket mapInfo)
    {
        _announcedArrival = false;
        _worldName = WorldName(mapInfo);
        _worldId = mapInfo.Name ?? string.Empty;
        WorldEntering?.Invoke(_worldName, mapInfo.Difficulty);

        _audio?.PlayMusic(mapInfo.Music);
        _map.Reset(mapInfo.Width, mapInfo.Height, mapInfo.Name);
        _combat.Clear();
        _particles.Clear();
        _departing.Clear();
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

    /// <summary>
    /// The number thrown off something that was hit.
    /// </summary>
    /// <remarks>
    /// Red over your own character and pale gold over anything else, because damage you took and
    /// damage you dealt are the two things you must never confuse mid-fight.
    /// </remarks>
    private void ShowDamage(Entity target, int amount, bool self)
    {
        if (amount <= 0 || _overlay == null || !WantsDamageText(self))
            return;

        // The client predicts its own hits and the server confirms some of them; showing both would
        // double every number. Whichever arrives first wins for a moment.
        int now = _clock.FrameMs;
        if (_shownDamageAt.TryGetValue(target.ObjectId, out int last) && now - last < DamageTextGapMs)
            return;

        _shownDamageAt[target.ObjectId] = now;

        _overlay.AddFloatingText(target.X, target.Y, target.Z,
            self ? $"-{amount}" : amount.ToString(System.Globalization.CultureInfo.InvariantCulture),
            self ? new Color("ff4040") : new Color("ffe9a8"));
    }

    /// <summary>Whether the player has asked to see this kind of thing's health.</summary>
    private bool WantsHealthBar(Resources.ObjectDesc desc)
    {
        var mode = (App.HealthBarMode)(Options?.HealthBars ?? (int)App.HealthBarMode.All);

        return mode switch
        {
            App.HealthBarMode.Off => false,
            App.HealthBarMode.Enemies => desc.IsEnemy,
            App.HealthBarMode.Allies => desc.IsPlayer,
            _ => true,
        };
    }

    /// <summary>Damage numbers, gated separately for the things dishing it out and taking it.</summary>
    private bool WantsDamageText(bool self) =>
        self ? Options is not { AllyDamageText: false } : Options is not { EnemyDamageText: false };

    /// <summary>When each entity last had a number thrown off it, so the two sources cannot double up.</summary>
    private readonly System.Collections.Generic.Dictionary<int, int> _shownDamageAt = new();

    private const int DamageTextGapMs = 120;

    /// <summary>
    /// A shot of ours landed, and the client worked out what it was worth.
    /// </summary>
    /// <remarks>
    /// This is the only account of a kill we made ourselves. The server sends a Damage packet to
    /// everyone who should see the hit except the person who reported it, so an enemy killed by our
    /// own bullet used to simply stop being drawn: no spray, no flash, nothing to say it died. Both
    /// of those now come off this path, which is where the hit is actually known.
    /// </remarks>
    private void OnDamageDealt(DamageDealt hit)
    {
        ShowDamage(hit.Target, hit.Amount, hit.Self);
        Struck(hit.Target, hit.Killed, hit.Projectile);
    }

    /// <summary>Plays the level-up chime when the server raises our level.</summary>
    private void NoticeLevelUp()
    {
        var player = _map.Player;
        int level = player?.Level ?? -1;

        if (level > _lastLevel && _lastLevel > 0)
            _audio?.PlayEffect("level_up");

        _lastLevel = level;
        NoticeExperience(player);
    }

    /// <summary>The experience we last saw, so a gain can be noticed rather than announced.</summary>
    private int _lastExperience = -1;

    /// <summary>
    /// Throws the experience gained off the character.
    /// </summary>
    /// <remarks>
    /// Derived from the stat rather than from a packet, because nothing on the wire says "you
    /// gained experience" -- the server simply sends a new total, several times a second while
    /// anything nearby is dying. Levelling resets the total, so a drop is a level rather than a
    /// loss and says nothing.
    /// </remarks>
    private void NoticeExperience(LocalPlayer player)
    {
        if (player == null)
            return;

        int gained = player.Experience - _lastExperience;
        _lastExperience = player.Experience;

        if (gained > 0 && gained < 1_000_000 && _lastExperience >= 0)
            _overlay?.AddFloatingText(player.X, player.Y, player.Z, $"+{gained} XP", new Color("7fe07f"));
    }

    private void OnEntered(CreateSuccessPacket packet)
    {
        // The player entity itself arrives in the first Update, not here.
    }

    /// <summary>
    /// Says the world is standing up, once the player is actually in it.
    /// </summary>
    /// <remarks>
    /// Driven from the tick rather than from an Update. Updates arrive when there is something to
    /// send, so in a quiet world the one that follows the player's own arrival may be a long time
    /// coming -- which left the loading screen up over a world that was already standing. Ticks
    /// come six times a second regardless, and a tick with a player on the map means the world is
    /// there to be looked at.
    /// </remarks>
    private void AnnounceArrival()
    {
        if (_announcedArrival || _map?.Player == null)
            return;

        _announcedArrival = true;
        WorldEntered?.Invoke();
    }

    private void OnWorldUpdated(UpdatePacket update)
    {

        foreach (var tile in update.Tiles)
        {
            _map.SetTile(tile.X, tile.Y, tile.Type);
            _minimap?.SetTile(tile.X, tile.Y, _map.GetSquare(tile.X, tile.Y));
        }

        foreach (int objectId in update.Drops)
        {
            var leaving = _map.GetEntity(objectId);
            var stoodOn = leaving?.Square;

            _map.Remove(objectId);
            NoteDeparture(leaving, stoodOn);
        }

        foreach (var definition in update.NewObjects)
            AddObject(definition);
    }

    // ------------------------------------------------------------------------------------------
    // Coming and going
    // ------------------------------------------------------------------------------------------

    /// <summary>How long a character takes to arrive or to leave, in milliseconds.</summary>
    private const int TransitionMs = 550;

    /// <summary>How far a departing character rises, and an arriving one falls from, in tiles.</summary>
    private const float TransitionRiseTiles = 1.6f;

    /// <summary>
    /// How near a character must be for its coming or going to be worth animating, in tiles.
    /// </summary>
    /// <remarks>
    /// The server drops an object from the update both when it stops existing and when it merely
    /// walks out of the twenty-tile sight radius, and nothing on the wire tells the two apart. A
    /// death or a portal happens where the player can see it; the radius edge is where things
    /// scroll out of view all the time, and animating that would have half the screen ascending
    /// whenever the player walked east.
    /// </remarks>
    private const float TransitionRadius = 14f;

    /// <summary>A character that has left the world but is still finishing its exit.</summary>
    private readonly System.Collections.Generic.List<(Entity Entity, int StartMs)> _departing = new();

    /// <summary>
    /// Whether a thing's coming or going is worth showing.
    /// </summary>
    /// <remarks>
    /// Characters only. Walls, floors and scenery arrive in their hundreds whenever the map scrolls
    /// or a world is entered, and a world that assembled itself out of falling masonry every time
    /// would be unreadable.
    /// </remarks>
    private bool WorthAnimating(Entity entity)
    {
        if (entity?.Desc == null || entity.Desc.Static)
            return false;

        if (!entity.Desc.IsPlayer && !entity.Desc.IsEnemy)
            return false;

        var player = _map.Player;
        if (player == null || ReferenceEquals(entity, player))
            return false;

        float dx = entity.X - player.X;
        float dy = entity.Y - player.Y;
        return dx * dx + dy * dy <= TransitionRadius * TransitionRadius;
    }

    private void NoteDeparture(Entity entity, Square stoodOn)
    {
        if (!WorthAnimating(entity))
            return;

        // Leaving the map unbinds the entity from its tile, and the draw path will not touch
        // anything without one. The binding is handed back here: the map has let go of both ends of
        // it already, so what is left is a back-pointer on a picture, which is all it needs to be.
        entity.Square = stoodOn;

        // Kept drawing after the world has forgotten it: it is a picture now, not a thing, and
        // nothing will move it or shoot it while it goes.
        _departing.Add((entity, _clock.FrameMs));
    }

    private void DropFinishedDepartures(int now)
    {
        for (int i = _departing.Count - 1; i >= 0; i--)
        {
            if (now - _departing[i].StartMs >= TransitionMs)
                _departing.RemoveAt(i);
        }
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

        // Stamped after the position is known, because whether an arrival is worth showing depends
        // on where it happened.
        if (WorthAnimating(entity))
            entity.ArrivedAtMs = _clock.FrameMs;

        // Walls and scenery are baked into the minimap where they stand, on the original's exact
        // condition: static, occupying its square, and not marked no-minimap. It is what turns a
        // dungeon's map from a wash of floor colour into its floor plan.
        if (desc is { Static: true, OccupySquare: true, NoMiniMap: false })
            _minimap?.SetObject((int)entity.X, (int)entity.Y, desc);

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
        AnnounceArrival();

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

            case VaultUpdatePacket vault:
                _vault.Apply(vault);
                break;

            case ShowEffectPacket effect:
                _particles.Show(effect, now);
                break;

            case QuestObjIdPacket quest:
                SetQuest(quest.ObjectId);
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
                {
                    string said = LineBuilder.Resolve(text.Text, _strings);
                    _chat?.Add(text, said);
                    Speak(text, said);
                }
                break;

            case AoePacket blast:
                OnAoe(blast);
                break;

            case NotificationPacket notification:
                OnNotification(notification);
                break;

            case GlobalNotificationPacket announcement:
                HandleGlobalNotification(announcement.Text);
                break;
        }
    }

    /// <summary>
    /// Acts on a GlobalNotification.
    /// </summary>
    /// <remarks>
    /// Despite the name and the string field, this packet carries commands rather than anything to
    /// read: a key colour to show, the key interface to toggle, whether the gift chest has anything
    /// in it. Printing the string is why "giftChestEmpty" was floating in the Nexus -- it is the
    /// server saying the chest is empty, not the name of a thing.
    /// </remarks>
    private void HandleGlobalNotification(string text)
    {
        switch (text)
        {
            case "giftChestOccupied":
                HasGift = true;
                break;

            case "giftChestEmpty":
                HasGift = false;
                break;

            // Key colours and the key interface, neither of which this port shows yet. Swallowed
            // rather than printed, because none of them is a sentence.
            case "yellow" or "red" or "green" or "purple" or "showKeyUI":
                break;

            default:
                // Anything the original does not recognise either, which it treats as a message.
                _chat?.AddSystem(LineBuilder.Resolve(text, _strings));
                break;
        }
    }

    /// <summary>Whether the account has an unclaimed gift waiting in the Nexus chest.</summary>
    public bool HasGift { get; private set; }

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

        float centre = shot.Angle + shot.AngleInc * (shot.NumShots - 1) / 2f;
        owner.SetAttack(centre, now);
        Muzzle(desc, shot.StartingPos.X, shot.StartingPos.Y, centre);
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

        Muzzle(desc, owner.X, owner.Y, shot.Angle);

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
    private void OnDeath(DeathPacket death)
    {
        // The character is gone. What is left on screen is a corpse the camera happens to be
        // pointed at, and it should not answer the keyboard: the death screen goes up over a world
        // that carries on without us, and walking the body around underneath it is nonsense.
        _dead = true;

        _map.Player?.SetInput(0f, 0f, 0f);

        Died?.Invoke(death);
    }

    /// <summary>Whether our character has died, after which nothing we press reaches the world.</summary>
    private bool _dead;

    /// <summary>
    /// The player's preferences, read at the points they govern rather than copied out.
    /// </summary>
    /// <remarks>
    /// Read live so a setting changed with the panel open takes effect on the next frame, which is
    /// the only way to judge a shadow or a particle setting: by looking at it while you change it.
    /// </remarks>
    private App.Settings Options => App.ServiceLocator.Settings;

    /// <summary>Where the frame goes. Only measured while the debug readout is up.</summary>
    public readonly FramePhases Phases = new();

    /// <summary>The level our own character reached, for the death screen.</summary>
    public int PlayerLevel => _map?.Player?.Level ?? 0;

    /// <summary>The server's authoritative word on damage, overriding any local prediction.</summary>
    private void OnDamage(DamagePacket damage)
    {
        var target = _map.GetEntity(damage.TargetId);
        if (target == null)
            return;

        target.Hp -= damage.DamageAmount;

        // The number, for damage that did not come from one of our own shots -- another player's
        // work, a wall of poison, anything the client did not predict for itself.
        if (damage.DamageAmount > 0 && target.Desc is { IsEnemy: true } or { IsPlayer: true })
            ShowDamage(target, damage.DamageAmount, ReferenceEquals(target, _map.Player));

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
    private void ThrowDebris(Entity target, DamagePacket damage) =>
        Struck(target, damage.Kill, _combat.Find(damage.ObjectId, damage.BulletId));

    /// <summary>
    /// Everything that says a thing was hit: the flash on the sprite and the spray off it.
    /// </summary>
    /// <remarks>
    /// Reached from both accounts of a hit -- the server's Damage packet for everyone else's, and
    /// the client's own prediction for ours -- so the two look the same and neither is silent.
    /// <see cref="ShowDamage"/> guards against a hit arriving twice; this does not need to, because
    /// a second flash over the first is invisible and a second handful of debris reads as a bigger
    /// handful rather than as a mistake.
    /// </remarks>
    private void Struck(Entity target, bool killed, Projectile projectile)
    {
        if (target?.Desc == null)
            return;

        target.StartFlash(_clock.FrameMs, 0xFFFFFF, killed ? DeathFlashMs : HitFlashMs, repeats: 1);

        // The flash stays whatever the particle settings say: it is how a hit registers at all, and
        // turning particles off to keep a slow machine playable should not also make combat silent
        // to look at.
        bool wanted = target.Desc.IsPlayer
            ? Options is not { PlayerHitParticles: false }
            : Options is not { EnemyParticles: false };

        if (Options is { Particles: false } || !wanted)
            return;

        var resolved = _textures.Resolve(target.Desc.Texture, target.ObjectId, target.AltTextureIndex);

        // Any one frame will do: a character's palette is the same whichever way it is facing.
        var sprite = resolved.Animated != null
            ? resolved.Animated.Frame(0f, 0f, CharAction.Stand, 0f).Sprite
            : resolved.Still;

        if (!sprite.IsValid)
            return;

        var palette = _palette.For(target.ObjectType, sprite, target.Desc.BloodProb, target.Desc.BloodColor);

        if (killed)
        {
            _particles.Explode(target.X, target.Y, palette, target.Size, count: 30);
            return;
        }

        if (projectile?.ProjectileDesc != null)
        {
            _particles.Hit(target.X, target.Y, palette, target.Size, count: 10,
                projectile.Angle, projectile.ProjectileDesc.Speed);
            return;
        }

        _particles.Explode(target.X, target.Y, palette, target.Size, count: 10);
    }

    /// <summary>
    /// How far past the visible circle an entity is still drawn, in tiles.
    /// </summary>
    /// <remarks>
    /// The radius is measured to the corner of the screen from the camera's focus, but a sprite is
    /// anchored at its feet and can be several tiles tall, and a big one is wider than its tile.
    /// This is the slack that keeps something enormous from popping in only once its feet cross the
    /// edge -- cheap insurance against a visible mistake, where the cost of erring wide is one
    /// entity's worth of work.
    /// </remarks>
    private const float OffscreenMargin = 6f;

    /// <summary>How long a struck sprite burns white for.</summary>
    /// <remarks>
    /// Short. Long enough to register on a hit that lands among many, brief enough that a stream of
    /// them reads as a rhythm rather than leaving the target permanently pale. A kill holds it
    /// longer, because it is the last thing the sprite does before it stops being drawn.
    /// </remarks>
    private const int HitFlashMs = 80;

    private const int DeathFlashMs = 200;

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

        using (Phases.Measure("entities"))
            _map.Update(now, deltaMs);

        ApplyGroundDamage(now);
        ApplyAttackInput(now);

        using (Phases.Measure("projectiles"))
            _combat.Update(now);

        _particles.Quality = Options?.ParticleDetail ?? 2;
        _world.Zoom = Mathf.Clamp(Options?.CameraZoom ?? 1f, FurthestZoom, NearestZoom);

        using (Phases.Measure("particles"))
            _particles.Update(now, deltaMs);

        _interaction.Update(now);
        _party.Update(now);
        _hud?.ShowPrompt(_interaction.Current.Exists ? _interaction.Current.Label : null);
        _hud?.ShowContainer(OpenContainer);
        UpdateVault();
        _hud?.ShowMerchant(NearbyMerchant, _map.Player);
        _hud?.ShowParty(_party.Members);
        _hud?.ShowWorld(_worldName, PlayersHere(), 0);
        RefreshQuest();

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

        // Pushed in rather than pulled: the view is handed the numbers it draws and never reaches
        // back into the inventory to ask.
        var cooldown = _inventory.AbilityCooldown(now);
        _hud?.SetAbilityCooldown(cooldown.Remaining, cooldown.Total);

        using (Phases.Measure("minimap"))
            _minimap?.Refresh();

        using (Phases.Measure("draw"))
            Draw(now);
    }

    /// <summary>Who is saying what, and until when. Keyed by the speaker's object id.</summary>
    private readonly System.Collections.Generic.Dictionary<int, (string Text, int UntilMs)> _bubbles = new();

    /// <summary>
    /// Puts a line of chat over the head of whoever said it.
    /// </summary>
    /// <remarks>
    /// Only when the server asked for one: the Text packet carries a bubble duration, and it is
    /// zero for the lines that are not speech -- guild chat, whispers, and everything the server
    /// says in its own voice.
    /// </remarks>
    private void Speak(TextPacket text, string said)
    {
        if (text.BubbleTime == 0 || text.ObjectId <= 0 || string.IsNullOrEmpty(said))
            return;

        // Long enough to read is the server's call, but a paragraph over someone's head is not.
        string shown = said.Length <= 64 ? said : said[..63] + "…";

        _bubbles[text.ObjectId] = (shown, _clock.FrameMs + text.BubbleTime * 1000);
    }

    /// <summary>What this entity is saying right now, or null.</summary>
    private string BubbleFor(Entity entity)
    {
        if (!_bubbles.TryGetValue(entity.ObjectId, out var bubble))
            return null;

        if (_clock.FrameMs < bubble.UntilMs)
            return bubble.Text;

        _bubbles.Remove(entity.ObjectId);
        return null;
    }

    /// <summary>
    /// A blast: something exploded, and everything inside the radius is caught in it.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The client decides whether it was caught. The server broadcasts the blast and waits for an
    /// acknowledgement carrying where we were when it landed -- it does not work out the hit for
    /// us -- so a client that ignores these takes no damage from a grenade and answers nothing.
    /// </para>
    /// <para>
    /// The ack goes out on every path, including having no player at all, because it is an answer
    /// to a question rather than a report of damage. This server's handler happens to be an empty
    /// TODO, but a client that only answered sometimes would be a client that desynchronises
    /// against any server that counts them.
    /// </para>
    /// </remarks>
    private void OnAoe(AoePacket blast)
    {
        var player = _map.Player;

        if (player == null)
        {
            _session.Send(new AoeAckPacket { Time = _clock.FrameMs, Position = new WorldPos(0f, 0f) });
            return;
        }

        // The blast itself, whether or not it reached us: something that goes off across the room
        // is worth seeing.
        if (Options is not { Particles: false } and not { AoeParticles: false })
            _particles.Blast(blast.Position.X, blast.Position.Y, blast.Radius, blast.OrigType);

        float dx = player.X - blast.Position.X;
        float dy = player.Y - blast.Position.Y;
        bool caught = dx * dx + dy * dy < blast.Radius * blast.Radius;

        if (caught && !player.IsInvincible && !player.IsPaused)
        {
            int damage = Entity.ApplyDefense(blast.Damage, player.Defense, false, player.Conditions);

            player.Hp -= damage;
            ShowDamage(player, damage, self: true);

            if (blast.Effect != 0)
                player.Conditions |= blast.Effect.ToFlag();

            if (damage > 0)
                _audio?.PlayEffect(player.Hp <= 0 ? player.Desc?.DeathSound : player.Desc?.HitSound);
        }

        _session.Send(new AoeAckPacket
        {
            Time = _clock.FrameMs,
            Position = new WorldPos(player.X, player.Y),
        });
    }

    /// <summary>
    /// Something the server wants said about a particular thing in the world.
    /// </summary>
    /// <remarks>
    /// Over the thing it is about, not in the chat log. These are "Quest Complete!", the effect of
    /// a potion, an enemy shrugging off a hit -- all of them about a position on screen, and all of
    /// them useless three lines up in a log by the time they are read.
    /// </remarks>
    private void OnNotification(NotificationPacket notification)
    {
        string message = LineBuilder.Resolve(notification.Message, _strings);
        var entity = _map.GetEntity(notification.ObjectId);

        // The server says a quest is done in the same breath as it says anything else about the
        // player, so the key is the only thing that distinguishes it.
        if (notification.Message != null && notification.Message.Contains("quest_complete"))
            QuestCompleted();

        if (entity == null)
        {
            _chat?.AddSystem(message);
            return;
        }

        var colour = new Color(
            notification.Color.R / 255f, notification.Color.G / 255f, notification.Color.B / 255f);

        // A server that sends no colour at all should not produce invisible text.
        if (colour.R + colour.G + colour.B < 0.05f)
            colour = Colors.White;

        _overlay?.AddFloatingText(entity.X, entity.Y, entity.Z, message, colour);
    }

    /// <summary>How often a damaging tile can hurt the same square's occupant.</summary>
    private const int GroundDamageIntervalMs = 500;

    /// <summary>
    /// Lava, and everything else that hurts to stand on.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The client is the one that decides this happened: it rolls the damage, applies it, and tells
    /// the server, which re-rolls the same number from its own copy of the shared stream and agrees.
    /// The server does not send a Damage packet back for it, so nothing else would ever apply it.
    /// </para>
    /// <para>
    /// Which makes the roll itself load-bearing beyond this feature. <c>Player.ForceGroundHit</c>
    /// draws from the same synchronised generator every time a ground hit lands; a client that never
    /// drew would fall one step behind for the rest of the session and mispredict every shot after
    /// it. Standing in lava was quietly corrupting the damage stream.
    /// </para>
    /// </remarks>
    private void ApplyGroundDamage(int now)
    {
        var player = _map.Player;
        var square = player?.Square;

        if (square?.Desc == null || square.Desc.MaxDamage <= 0)
            return;

        if (now < square.LastDamageMs + GroundDamageIntervalMs)
            return;

        if (player.IsInvincible || player.IsPaused)
            return;

        // Something built over the tile can shelter whoever stands on it -- a bridge over lava.
        if (square.StaticObject?.Desc is { ProtectFromGroundDamage: true })
            return;

        square.LastDamageMs = now;

        int damage = _session.Random == null
            ? square.Desc.MinDamage
            : (int)_session.Random.NextIntRange((uint)square.Desc.MinDamage, (uint)square.Desc.MaxDamage);

        // Armour does not help against the floor, which is why the original passes the rolled
        // number straight through rather than through its defence formula.
        player.Hp -= damage;
        ShowDamage(player, damage, self: true);

        if (damage > 0)
            _audio?.PlayEffect(player.Hp <= 0 ? player.Desc?.DeathSound : player.Desc?.HitSound);

        _session.Send(new GroundDamagePacket
        {
            Time = now,
            Position = new WorldPos(player.X, player.Y),
        });
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
    /// <summary>
    /// How many players this client can see, itself included.
    /// </summary>
    /// <remarks>
    /// Not the world's population, which nothing on the wire carries -- the server list's usage is
    /// a fraction per server, not a count per world. In a Nexus it is very nearly everyone; in a
    /// realm it is the ones near you.
    /// </remarks>
    private int PlayersHere()
    {
        int count = _map.Player == null ? 0 : 1;

        foreach (var entity in _map.Entities)
        {
            if (entity.Desc is { IsPlayer: true } && !entity.Dead && !ReferenceEquals(entity, _map.Player))
                count++;
        }

        return count;
    }

    /// <summary>The world's own name, for the line over the party list.</summary>
    private string _worldName = string.Empty;

    /// <summary>What the debug readout asks for, so it can be shown without it reaching in here.</summary>
    public string CurrentWorldName => _worldName;

    public int EntityCount => _map?.Entities.Count ?? 0;

    public int ProjectileCount => _combat?.Projectiles.Count ?? 0;

    /// <summary>How many surfaces the sprites became: the part of the draw calls we control.</summary>
    public int SpriteSurfaces => _world?.Sprites?.SurfaceCount ?? 0;

    public (float X, float Y) PlayerAt =>
        _map?.Player is { } player ? (player.X, player.Y) : (0f, 0f);

    /// <summary>The world's internal name, which is what the server's own rules are written against.</summary>
    private string _worldId = string.Empty;

    /// <summary>
    /// What to call the world we have just entered.
    /// </summary>
    /// <remarks>
    /// The server sends some of these as localisation keys in braces -- the vault arrives as
    /// "{server.vault}" -- so a name that looks like one is looked up before it is shown. Anything
    /// the table does not know keeps its own text rather than showing the braces.
    /// </remarks>
    private string WorldName(MapInfoPacket mapInfo)
    {
        string name = mapInfo.DisplayName ?? mapInfo.Name ?? string.Empty;

        if (name.Length < 3 || name[0] != '{' || name[^1] != '}')
            return name;

        string key = name[1..^1];
        return _strings != null && _strings.Has(key) ? _strings.Get(key) : key;
    }

    /// <summary>Says that a button the reference carries is not answered by this server.</summary>
    private void Unavailable(string what) =>
        _chat?.AddSystem($"{what} is not available on this server.");

    /// <summary>
    /// An item was dragged out of a slot and let go over the world.
    /// </summary>
    /// <remarks>
    /// Only the player's own slots can drop: dragging out of a chest and letting go over the ground
    /// would be asking the server to move an item between two things it does not own, and it has no
    /// packet for that.
    /// </remarks>
    private void OnSlotDroppedOutside(SlotAddress from)
    {
        if (from.Owner != SlotOwner.Player)
            return;

        // The server refuses outright in the Nexus and answers nothing at all -- a bare return, no
        // InvResult -- so a client that just sent it would show the item leaving the slot and then
        // reappearing a tick later with no explanation.
        if (string.Equals(_worldId, "Nexus", System.StringComparison.OrdinalIgnoreCase))
        {
            _chat?.AddSystem("Items cannot be dropped in the Nexus.");
            return;
        }

        _inventory.Drop(from.Index);
    }

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

    /// <summary>
    /// An item was dragged from one slot onto another.
    /// </summary>
    /// <remarks>
    /// The container has to be handed over each time rather than held, because which one is open
    /// changes as the player walks around, and a drag that started over one chest should not land
    /// in another.
    /// </remarks>
    private void OnSlotDropped(SlotAddress from, SlotAddress to)
    {
        // Anything touching the vault goes on the vault's own wire: its slots belong to no entity,
        // so InvSwap has no way to name them.
        if (from.Owner == SlotOwner.Vault || to.Owner == SlotOwner.Vault)
        {
            _vault.Move(from, to);
            return;
        }

        _inventory.OpenContainer = OpenContainer;
        _inventory.Move(from, to);
    }

    /// <summary>
    /// A vault slot was clicked, which moves the item to the first free carried slot.
    /// </summary>
    /// <remarks>
    /// The same gesture a chest has always answered to, and the reason it stays enabled under every
    /// sort and filter: taking an item out names no destination in the vault, so nothing about the
    /// view can make it ambiguous.
    /// </remarks>
    private void OnVaultSlotActivated(SlotAddress from)
    {
        var player = _map.Player;
        if (player?.Equipment == null)
            return;

        bool gift = from.Owner == SlotOwner.VaultGift;
        int held = gift ? _vault.GiftAt(from.Index) : _vault.ItemAt(from.Index);

        if (held == VaultStore.NoItem)
            return;

        int free = -1;
        for (int i = Inventory.CarriedFirstSlot; i < player.Equipment.Length; i++)
            if (player.Equipment[i] == Inventory.NoItem)
            {
                free = i;
                break;
            }

        if (free < 0)
        {
            _chat?.AddSystem("No room to take that.");
            return;
        }

        if (gift)
            _vault.ClaimGift(from.Index, free);
        else
            _vault.Move(from, new SlotAddress(SlotOwner.Player, free));
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

        if (_dead)
        {
            player.SetInput(0f, 0f, 0f);
            return;
        }

        // The vault's search field takes the keyboard on the same terms the chat box does: while it
        // has focus the letters are text, not movement.
        if (_hud is { VaultTyping: true })
        {
            player.SetInput(0f, 0f, 0f);
            return;
        }

        // While the chat box has the keyboard, the movement keys belong to it -- they are letters.
        if (_chat is { IsTyping: true })
        {
            player.SetInput(0f, 0f, 0f);

            if (Input.IsKeyPressed(Key.Escape))
                _chat.EndTyping();
            return;
        }

        if (Input.IsActionJustPressed("debug_overlay"))
            DebugToggled?.Invoke();

        if (Input.IsActionJustPressed("options"))
        {
            OptionsToggled?.Invoke();
            return;
        }

        if (Input.IsActionJustPressed("social_panel"))
        {
            GuildToggled?.Invoke();
            return;
        }

        // The character sheet is the one panel that does not hold the keyboard: it opens beside the
        // world and everything below carries on working while it is up.
        if (Input.IsActionJustPressed("character_panel"))
            CharacterToggled?.Invoke();

        // With the panel up the keyboard belongs to it, but movement should stop rather than
        // continue in whatever direction was last held.
        if (OptionsAreOpen != null && OptionsAreOpen())
        {
            player.SetInput(0f, 0f, 0f);
            return;
        }

        // Three ways into the chat box, and two of them arrive with something already typed. The
        // original had the same: a slash opens it ready for a command, tab ready for a whisper, and
        // G ready for guild chat.
        if (Input.IsActionJustPressed("toggle_chat"))
        {
            _chat?.BeginTyping();
            return;
        }

        if (Input.IsActionJustPressed("chat_command"))
        {
            _chat?.BeginTyping("/");
            return;
        }

        if (Input.IsActionJustPressed("tell"))
        {
            _chat?.BeginTyping("/tell ");
            return;
        }

        if (Input.IsActionJustPressed("guild_chat"))
        {
            _chat?.BeginTyping("/g ");
            return;
        }

        if (Input.IsActionJustPressed("toggle_centering"))
            CenterOnPlayer = !CenterOnPlayer;

        for (int slot = 0; slot < InventoryHotkeys; slot++)
        {
            if (!Input.IsActionJustPressed($"inv_slot_{slot + 1}"))
                continue;

            // The border flashes whether or not the item did anything, so a key that fired
            // something invisible -- a potion at full health, an ability on cooldown -- is still
            // distinguishable from a key that did not register.
            // Whichever page the hotbar is showing: 3 always means the third square you can see.
            int index = (_hud?.HotbarFirstSlot ?? Inventory.CarriedFirstSlot) + slot;

            _hud?.FlashSlot(index);
            OnSlotActivated(index);
        }

        if (Input.IsActionJustPressed("scroll_chat_up"))
            _chat?.Scroll(-3);

        if (Input.IsActionJustPressed("scroll_chat_down"))
            _chat?.Scroll(3);

        float x = Input.GetActionStrength("move_right") - Input.GetActionStrength("move_left");
        float y = Input.GetActionStrength("move_down") - Input.GetActionStrength("move_up");
        float rotate = Options is { AllowCameraRotation: false }
            ? 0f
            : Input.GetActionStrength("rotate_right") - Input.GetActionStrength("rotate_left");

        if (AutoWalk)
        {
            // A slow circle, so the walker stays near where it started and keeps changing heading.
            float phase = _clock.FrameMs / 3000f * Mathf.Tau;
            x = Mathf.Cos(phase);
            y = Mathf.Sin(phase);
        }

        player.SetInput(x, y, rotate);

        if (player.InputRotate != 0f)
            _cameraAngle += deltaMs * LocalPlayer.RotateSpeed * RotationSpeed * player.InputRotate;

        if (Input.IsActionJustPressed("reset_camera"))
            _cameraAngle = DefaultCameraAngle;

        if (Input.IsActionJustPressed("camera_zoom_in"))
            StepCameraZoom(1);

        if (Input.IsActionJustPressed("camera_zoom_out"))
            StepCameraZoom(-1);

        if (Input.IsActionJustPressed("autofire"))
            _autofire = !_autofire;

        if (Input.IsActionJustPressed("interact"))
            Interact();

        if (Input.IsActionJustPressed("switch_tabs"))
            _hud?.SwitchTab();

        if (Input.IsActionJustPressed("toggle_hud"))
        {
            HudHidden = !HudHidden;
            HudVisibilityChanged?.Invoke(HudHidden);
        }

        if (Input.IsActionJustPressed("toggle_hp_bars"))
            _overlay?.ToggleHealthBars();

        if (Input.IsActionJustPressed("minimap_zoom_in"))
            _minimap?.Zoom(1);

        if (Input.IsActionJustPressed("minimap_zoom_out"))
            _minimap?.Zoom(-1);

        // Two keys for the Nexus, as in the original -- R for the hand on the keyboard and F5 for
        // the one that has just been surprised.
        if (Input.IsActionJustPressed("nexus") || Input.IsActionJustPressed("nexus_alt"))
            NexusRequested?.Invoke();

        if (Input.IsActionJustPressed("quick_slot_1"))
            _inventory.UsePotion(health: true);

        if (Input.IsActionJustPressed("quick_slot_2"))
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

            case InteractionKind.Vault:
                // Already showing whenever the player is standing on the chest, like the other two.
                break;
        }
    }

    /// <summary>Keeps the vault panel open wherever the player is standing. See GameScene.OpenVault.</summary>
    public bool HoldVaultOpen { get; set; }

    /// <summary>The vault access object the player is standing on, if any.</summary>
    private Entity NearbyVault =>
        _interaction.Current is { Kind: InteractionKind.Vault, Entity: { } entity } ? entity : null;

    /// <summary>
    /// Closes the vault when the player walks away from the thing that opened it.
    /// </summary>
    /// <remarks>
    /// Opened by standing on the chest and closed by stepping off it, which is how the loot bag
    /// and the vendor already work -- the vault was the one that also wanted a key press, and a key
    /// that opens what you are already standing on is a key that does nothing you did not ask for.
    ///
    /// Losing the panel by leaving is intended: storage stays somewhere you go, rather than
    /// something you carry. A drag in flight is cancelled rather than landed, since the slot it
    /// started from is about to stop being addressable.
    /// </remarks>
    private void UpdateVault()
    {
        if (_hud == null)
            return;

        if (NearbyVault != null || HoldVaultOpen)
        {
            _hud.UseVault(_vault);

            if (!_hud.VaultOpen)
                _hud.ShowVault(true);

            return;
        }

        if (!_hud.VaultOpen)
            return;

        if (GetViewport().GuiIsDragging())
            Input.ParseInputEvent(new InputEventMouseButton
            {
                ButtonIndex = MouseButton.Left,
                Pressed = false,
            });

        _hud.ShowVault(false);
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
        if (player == null || _dead)
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
        if (Options is { WeaponSounds: false })
            return;

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

        using (Phases.Measure("draw.ground"))
            DrawGround(focus, radius, now);

        using (Phases.Measure("draw.sprites"))
            DrawEntities(now, _cameraAngle, radius);

        using (Phases.Measure("draw.shots"))
            DrawProjectiles(now, radius);

        using (Phases.Measure("draw.particles"))
            DrawParticles();

        using (Phases.Measure("draw.overlay"))
            DrawOverlay(now);

        // One upload for however many tiles were baked while sweeping the visible area.
        using (Phases.Measure("draw.atlas"))
            _tileAtlas.Flush();

        using (Phases.Measure("draw.submit"))
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

    private void DrawEntities(int now, float cameraAngle, float radius)
    {
        DropFinishedDepartures(now);

        // The server streams a twenty-tile circle, which is a good deal more ground than the screen
        // shows, so a large share of what the client knows about is off the edge of it. Resolving a
        // texture, picking an animation frame and writing an instance for each of those is work
        // whose only outcome is a quad the GPU clips away.
        var focus = _focus ?? _map.Player;
        float reach = radius + OffscreenMargin;
        float reachSquared = reach * reach;

        foreach (var entity in _map.Entities)
        {
            if (focus != null && !ReferenceEquals(entity, focus))
            {
                float dx = entity.X - focus.X;
                float dy = entity.Y - focus.Y;
                if (dx * dx + dy * dy > reachSquared)
                    continue;
            }

            DrawEntity(entity, now, cameraAngle, Arriving(entity, now));
        }

        // Things that have left. Drawn from a list of their own because the world no longer holds
        // them: they are pictures finishing a movement, and nothing can touch them while they do.
        foreach (var (entity, startMs) in _departing)
            DrawEntity(entity, now, cameraAngle, Leaving(now - startMs));
    }

    /// <summary>Where a coming-or-going sprite is on its way, as a rise in tiles and an opacity.</summary>
    private readonly record struct Transition(float Rise, float Alpha)
    {
        public static readonly Transition Settled = new(0f, 1f);
    }

    /// <summary>
    /// A thing that has just appeared, falling the last of the way in and fading up.
    /// </summary>
    private static Transition Arriving(Entity entity, int nowMs)
    {
        if (entity.ArrivedAtMs == 0)
            return Transition.Settled;

        int since = nowMs - entity.ArrivedAtMs;
        if (since < 0 || since >= TransitionMs)
        {
            entity.ArrivedAtMs = 0;
            return Transition.Settled;
        }

        // Eased so it lands softly rather than arriving at full speed and stopping dead.
        float t = since / (float)TransitionMs;
        float remaining = (1f - t) * (1f - t);
        return new Transition(TransitionRiseTiles * remaining, t);
    }

    /// <summary>
    /// A thing on its way out, rising and fading.
    /// </summary>
    /// <remarks>
    /// It goes pale as it climbs as well as transparent, which is what makes it read as leaving
    /// rather than as a rendering fault: colour drains out of it before it stops being there.
    /// </remarks>
    private static Transition Leaving(int since)
    {
        float t = Mathf.Clamp(since / (float)TransitionMs, 0f, 1f);
        return new Transition(TransitionRiseTiles * t * t, 1f - t);
    }

    private void DrawEntity(Entity entity, int now, float cameraAngle, Transition transition)
    {
        if (entity.Desc == null || entity.Square is not { IsKnown: true })
            return;

        // A merchant is drawn as the thing it is selling, which is how the original does it:
        // its own Merchant.getTexture returns the merchandise texture rather than any sprite of
        // its own. Without this every vendor in the Nexus is the same grey placeholder and the
        // only way to see what is for sale is to walk into it.
        var texture = entity.Desc.Texture;
        int variant = entity.ObjectId;

        if (entity.MerchandiseType >= 0 && _data?.GetObject((ushort)entity.MerchandiseType) is { } sold)
        {
            texture = sold.Texture;
            variant = entity.MerchandiseType;
        }

        var resolved = _textures.Resolve(texture, variant, entity.AltTextureIndex);
        if (!resolved.IsValid)
            return;

        var modulate = Modulate(entity);
        if (transition.Alpha < 1f)
        {
            // Pale as well as faint, so a departure reads as ascending rather than as a sprite
            // failing to draw.
            modulate = modulate.Lightened(1f - transition.Alpha);
            modulate.A *= transition.Alpha;
        }

        var draw = new SpriteDraw
        {
            TileX = entity.X,
            TileY = entity.Y,
            Height = entity.Z + transition.Rise,
            AnchorX = 0.5f,
            Modulate = modulate,
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
                return;

            draw.Sprite = frame.Sprite;
            draw.Mirrored = frame.Mirrored;
            SizeQuad(ref draw, entity, frame.Sprite, frame.RegionCells, frame.RegionCells);
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
            return;

        // Nothing casts a shadow while it is off the ground on its way in or out.
        if (transition.Rise <= 0f && Options is not { Shadows: 0 })
            AddShadow(entity, draw.SortBias);

        _world.Sprites.Add(draw);
        AddFlash(entity, draw, now);
    }

    /// <summary>
    /// The status icons showing on an entity, or null if it has none.
    /// </summary>
    /// <remarks>
    /// One list per entity, kept and refilled rather than allocated: this runs for every visible
    /// entity every frame, and most of them have nothing on them at all.
    /// </remarks>
    private System.Collections.Generic.List<int> ConditionsFor(Entity entity, int now)
    {
        if (entity.Conditions == ConditionEffects.None)
            return null;

        if (!_conditionIcons.TryGetValue(entity.ObjectId, out var icons))
        {
            icons = new System.Collections.Generic.List<int>(4);
            _conditionIcons[entity.ObjectId] = icons;
        }

        Render.ConditionIcons.Collect(entity.Conditions, now, icons);
        return icons.Count == 0 ? null : icons;
    }

    private readonly System.Collections.Generic.Dictionary<int, System.Collections.Generic.List<int>>
        _conditionIcons = new();

    /// <summary>Cuts status icons out of the sheet the original takes them from.</summary>
    private sealed class SheetConditionIcons : Render.IConditionSheet
    {
        private readonly Assets.AssetLibrary _assets;

        public SheetConditionIcons(Assets.AssetLibrary assets) => _assets = assets;

        public Godot.Texture2D Texture =>
            _assets?.GetSprite(Render.ConditionIcons.Sheet, 0).Sheet;

        public Godot.Rect2? Region(int index)
        {
            var sprite = _assets?.GetSprite(Render.ConditionIcons.Sheet, index) ?? default;
            return sprite.IsValid
                ? new Godot.Rect2(sprite.Region.Position, sprite.Region.Size)
                : null;
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
            // Degrees in the XML for a model object; the draw list wants radians.
            Rotation = entity.Desc.Rotation * (Mathf.Pi / 180f),
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

    private void DrawOverlay(int now)
    {
        if (_overlay == null)
            return;

        _overlay.Clear();
        _overlay.SetQuestMarker(QuestMarkerPosition());

        if (_minimap != null)
            _minimap.QuestTargetId = _questObjectId;

        foreach (var entity in _map.Entities)
        {
            var desc = entity.Desc;
            if (desc == null || entity.Square is not { IsKnown: true } || entity.Dead)
                continue;

            bool combatant = desc.IsEnemy || desc.IsPlayer;
            bool named = desc.ShowName && !string.IsNullOrEmpty(entity.Name);
            string bubble = Options is { TextBubbles: false } ? null : BubbleFor(entity);

            // Nothing to say about a plain decoration.
            if (!combatant && !named)
                continue;

            // An enemy that cannot be hurt should not advertise a health bar, and an invisible one
            // should not advertise anything at all.
            bool showBar = combatant
                           && !entity.IsInvisible
                           && !entity.IsInvulnerable
                           && !desc.NoMiniMap
                           && entity.MaxHp > 0
                           && WantsHealthBar(desc);

            if (!showBar && !named && bubble == null)
                continue;

            var scene = _world.Projection.ToScene(entity.X, entity.Y, entity.Z);
            var anchor = _world.Unproject(scene);

            // How tall this thing draws, in screen pixels: the difference between its feet and the
            // top of its artwork, once its size stat has been applied.
            float top = _world.Unproject(
                _world.Projection.ToScene(entity.X, entity.Y, entity.Z + SpriteHeightTiles(entity))).Y;

            _overlay.Add(new OverlayItem
            {
                Anchor = anchor,
                SpriteHeight = Mathf.Abs(anchor.Y - top),
                Bubble = bubble,
                Name = named ? entity.Name : null,
                NameColor = desc.IsPlayer ? new Color(0.99f, 0.87f, 0f) : Colors.White,
                Hp = entity.Hp,
                MaxHp = entity.MaxHp,
                ShowHealthBar = showBar,
                Conditions = ConditionsFor(entity, now),
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
        var target = QuestTarget();

        return target == null
            ? null
            : _world.Unproject(_world.Projection.ToScene(target.X, target.Y, target.Z));
    }

    /// <summary>
    /// The thing the realm has asked for, once it is worth pointing at.
    /// </summary>
    /// <remarks>
    /// Held back for four seconds after the server names it, and for fifteen or so after one is
    /// finished, which is the original's timing. A quest arrow that snapped to the next monster the
    /// instant the last one died would be pointing somewhere new before the player had noticed the
    /// first was gone.
    /// </remarks>
    private Entity QuestTarget()
    {
        if (_questObjectId == 0 || _clock.FrameMs < _questAvailableAtMs)
            return null;

        var target = _map.GetEntity(_questObjectId);
        return target is { Dead: false } ? target : null;
    }

    /// <summary>When the current quest becomes worth showing, and how long it counts as new.</summary>
    private int _questAvailableAtMs;

    private int _questNewUntilMs;

    private void SetQuest(int objectId)
    {
        // Only a quest arriving where there was none waits: one replacing another is the realm
        // moving the player on, and holding that back would leave the arrow pointing at a corpse.
        if (_questObjectId == 0 && objectId != 0)
        {
            _questAvailableAtMs = _clock.FrameMs + 4000;
            _questNewUntilMs = _questAvailableAtMs + 2000;
        }

        _questObjectId = objectId;
    }

    /// <summary>
    /// A quest was finished, so the next one waits a while.
    /// </summary>
    /// <remarks>
    /// Fifteen seconds less a random few, exactly as the original does it -- the stagger is there so
    /// that a group who all finished the same quest do not all turn to the same next target at the
    /// same instant.
    /// </remarks>
    private void QuestCompleted()
    {
        _questAvailableAtMs = _clock.FrameMs + 15000 - (int)(GD.Randf() * 10000f);
        _questNewUntilMs = _questAvailableAtMs + 2000;
    }

    /// <summary>Pushes the quest to the interface: who it is, and whether it is still new.</summary>
    private void RefreshQuest()
    {
        var target = QuestTarget();

        if (target?.Desc == null)
        {
            _hud?.ShowQuest(null, 0, false);
            return;
        }

        string name = target.Desc.DisplayId ?? target.Desc.Id;
        _hud?.ShowQuest(name, target.ObjectType, _clock.FrameMs < _questNewUntilMs);
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
    private void DrawProjectiles(int now, float radius)
    {
        // Same reasoning as the entities: a bullet fired across the room is simulated whether or
        // not it is on screen, but only the ones on screen are worth building a quad for.
        var focus = _focus ?? _map.Player;
        float reach = radius + OffscreenMargin;
        float reachSquared = reach * reach;

        foreach (var projectile in _combat.Projectiles)
        {
            if (projectile.Desc == null)
                continue;

            if (focus != null)
            {
                float dx = projectile.X - focus.X;
                float dy = projectile.Y - focus.Y;
                if (dx * dx + dy * dy > reachSquared)
                    continue;
            }

            var resolved = _textures.Resolve(projectile.Desc.Texture, projectile.ObjectId);
            if (!resolved.Still.IsValid)
                continue;

            // The original's angle: where it is flying, less where the camera is standing, plus
            // the correction that squares the artwork with it, plus any spin. Bullet art is drawn
            // pointing up and to the right rather than along the x axis, which is what
            // AngleCorrection is for -- 219 of the objects in this data set carry one.
            float spin = projectile.Desc.Rotation == 0f ? 0f : now / projectile.Desc.Rotation;
            float angle = projectile.Angle - _world.Projection.Angle +
                          projectile.Desc.AngleCorrection + spin;

            var draw = new SpriteDraw
            {
                TileX = projectile.X,
                TileY = projectile.Y,
                Height = projectile.Z,
                Sprite = resolved.Still,
                AnchorX = 0.5f,
                AnchorY = 0.5f,
                Rotation = angle,
                Modulate = Colors.White,
                Outlined = true,
                // Slightly above whatever they are flying over, so a bullet is never swallowed by
                // the sprite it is about to hit.
                SortBias = 0.5f,
            };

            SizeQuad(ref draw, projectile, resolved.Still, 1, 1);
            _world.Sprites.Add(draw);

            LeaveTrail(projectile, now);
        }
    }

    /// <summary>
    /// Spawns the sparks a trailing projectile leaves behind it.
    /// </summary>
    /// <remarks>
    /// On a clock rather than per frame, which is what the original does. Three sparks every frame
    /// ties the density of a trail to the frame rate, so the same shot leaves a thicker trail on a
    /// faster machine; spawning on an interval makes a trail look the same everywhere.
    /// </remarks>
    /// <summary>
    /// The burst where a shot stopped.
    /// </summary>
    /// <remarks>
    /// Coloured by the shot's own trail colour where it has one, so a fire bolt splashes orange and
    /// an arrow splashes white. A hit on terrain throws fewer sparks than a hit on something alive,
    /// which is the difference between striking a wall and striking a target.
    /// </remarks>
    /// <summary>
    /// How tall an entity's artwork is, in tiles.
    /// </summary>
    /// <remarks>
    /// The same arithmetic the sprite list does -- eight pixels to the tile, scaled by the size
    /// stat -- so the furniture over an entity lines up with the artwork rather than with a guess.
    /// Animated characters are measured from their standing frame, which is representative enough.
    /// </remarks>
    private float SpriteHeightTiles(Entity entity)
    {
        const float PixelsPerTile = 8f;

        var resolved = _textures.Resolve(entity.Desc.Texture, entity.ObjectId, entity.AltTextureIndex);

        var sprite = resolved.Animated != null
            ? resolved.Animated.Frame(0f, 0f, Assets.CharAction.Stand, 0f).Sprite
            : resolved.Still;

        if (!sprite.IsValid)
            return 1f;

        return sprite.Region.Size.Y / PixelsPerTile * (entity.Size / 100f);
    }

    private void OnProjectileStruck(Projectile projectile, ProjectileEnding ending)
    {
        var shot = projectile.ProjectileDesc;
        int colour = shot is { ParticleTrail: true } ? shot.ParticleTrailColor : 0xFFFFFF;
        int count = ending == ProjectileEnding.HitTerrain ? 3 : 6;

        _particles.Impact(projectile.X, projectile.Y, projectile.Z, projectile.Angle, colour, count);
    }

    /// <summary>
    /// The flash where a shot leaves.
    /// </summary>
    /// <remarks>
    /// New rather than ported: the original gives firing no visual at all beyond the projectile
    /// appearing, and on a slow-moving shot there is nothing to say the trigger was pulled until it
    /// has crossed a tile. Coloured from the shot itself so it belongs to the weapon that fired it.
    /// </remarks>
    /// <summary>The range the view distance may be set to, as camera multipliers.</summary>
    private const float NearestZoom = 2f;

    private const float FurthestZoom = 0.5f;

    /// <summary>Moves the view distance a step, and remembers it.</summary>
    private void StepCameraZoom(int steps)
    {
        var options = Options;
        if (options == null)
            return;

        options.CameraZoom = Mathf.Clamp(
            Mathf.Round((options.CameraZoom + steps * 0.1f) * 100f) / 100f, FurthestZoom, NearestZoom);

        options.Save();
    }

    /// <summary>How fast a held turn key sweeps the camera, as a multiple of the original's rate.</summary>
    private float RotationSpeed => (Options?.CameraRotationSpeed ?? 1) switch
    {
        0 => 0.5f,
        2 => 2f,
        _ => 1f,
    };

    /// <summary>
    /// The angle the camera resets to, in radians.
    /// </summary>
    /// <remarks>
    /// The original's two choices are the isometric default and a further eighth of a turn, which
    /// squares the world up against the screen. Seven quarters of pi is the default it ships with.
    /// </remarks>
    private float DefaultCameraAngle =>
        Options is { DefaultCameraAngle: 45 }
            ? 7f * Mathf.Pi / 4f + Mathf.Pi / 4f
            : 7f * Mathf.Pi / 4f;

    private void Muzzle(ProjectileDesc shot, float x, float y, float angle)
    {
        if (Options is { Particles: false })
            return;

        int colour = shot is { ParticleTrail: true } ? shot.ParticleTrailColor : 0xFFE9A8;
        _particles.Muzzle(x, y, 0.35f, angle, colour);
    }

    private void LeaveTrail(Projectile projectile, int now)
    {
        const int IntervalMs = 24;

        var shot = projectile.ProjectileDesc;
        if (shot is not { ParticleTrail: true })
            return;

        if (now < projectile.NextTrailMs)
            return;

        // Caught up rather than stepped, so a frame that took a while does not owe a burst of
        // sparks it then emits all at once.
        projectile.NextTrailMs = now + IntervalMs;

        _particles.ProjectileTrail(projectile.X, projectile.Y, projectile.Z,
            shot.ParticleTrailColor, shot.ParticleTrailLifetimeMs);
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

        // Loot bags are the one thing on screen a player has to hit with a mouse, and after a fight
        // they come in heaps. Their own size stat is left alone; this is on top of it.
        if (entity.Desc is { Class: "Container" })
            scale *= Mathf.Clamp(App.ServiceLocator.Settings?.BagSize ?? 1f, 0.5f, 2.5f);
        float cellWidth = sprite.Region.Size.X / (float)Mathf.Max(regionCells, 1);

        draw.WidthTiles = cellWidth * cellsWide / PixelsPerTile * scale;
        draw.HeightTiles = sprite.Region.Size.Y / PixelsPerTile * scale;
    }

    /// <summary>
    /// Where the sprite's anchor sits horizontally.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Normally the middle. The extended attack frame is two cells of artwork — the character and
    /// the weapon reaching past it — so the anchor moves to the middle of the character's own cell,
    /// which keeps it standing over its tile while the weapon overhangs. Mirroring flips the pair,
    /// putting the character in the other cell.
    /// </para>
    /// <para>
    /// The original composites those two cells into a three-cell image with a blank cell on the
    /// far side, so that the character lands in the middle of it. The blank cell draws nothing, so
    /// it is left out here and the anchor does the same job — but the quad has to be sized to the
    /// artwork rather than to the original's three cells, or two cells of picture get stretched
    /// across three tiles.
    /// </para>
    /// </remarks>
    private static float AnchorFor(in CharFrame frame)
    {
        if (frame.RegionCells <= 1)
            return 0.5f;

        // Centre of the cell the character occupies. It is the first of the region, or the last
        // once the quad has been flipped.
        float characterCell = frame.Mirrored ? frame.RegionCells - 1f : 0f;
        return (characterCell + 0.5f) / frame.RegionCells;
    }

    private static CharFrame SelectFrame(Entity entity, AnimatedChar animated, int now, float cameraAngle)
    {
        // Attacking wins over walking, and holds the pose for a fixed window after the shot.
        // Turning to face the shot is a lasting change, not a choice made for this frame: the
        // original assigns facing_ here, so a character that stops after firing goes on facing
        // the way it fired.
        if (entity.IsAttacking(now))
        {
            if (entity.Desc is not { DontFaceAttacks: true })
                entity.Facing = entity.AttackAngle;

            float phase = (now - entity.AttackStartMs) % Entity.AttackPeriodMs / (float)Entity.AttackPeriodMs;
            return animated.Frame(entity.Facing, cameraAngle, CharAction.Attack, phase);
        }

        // Which way a character faces is which way it is going. Every character has this, not just
        // the one being driven from the keyboard -- for everything else the movement comes from the
        // server, as the step between the last two ticks.
        if (entity.IsMoving)
        {
            entity.Facing = Mathf.Atan2(entity.MoveVecY, entity.MoveVecX);

            // A fixed cadence rather than one tied to speed; the original quantised it to a
            // multiple of 400ms, which amounts to the same thing for every practical speed.
            const float WalkPeriodMs = 400f;
            float phase = now % WalkPeriodMs / WalkPeriodMs;
            return animated.Frame(entity.Facing, cameraAngle, CharAction.Walk, phase);
        }

        // Standing keeps whatever it was last facing, rather than snapping back to due east.
        return animated.Frame(entity.Facing, cameraAngle, CharAction.Stand, 0f);
    }

    /// <summary>
    /// The whole-sprite colour treatment a status effect calls for.
    /// </summary>
    /// <remarks>
    /// Greyscale reads as "this thing is not currently participating", which covers paused, stasis
    /// and petrified alike. Curse gets its own red wash so it is distinguishable at a glance.
    /// </remarks>
    private SpriteTint TintFor(Entity entity)
    {
        if (entity.Has(ConditionEffects.Curse) && Options is { CurseIndication: true })
            return SpriteTint.Red;

        if (entity.IsPaused || entity.IsStasis || entity.IsPetrified)
            return SpriteTint.Greyscale;

        return SpriteTint.None;
    }

    private Color Modulate(Entity entity)
    {
        // Invisible players are drawn faintly rather than hidden, so allies can still be followed.
        if (entity.IsInvisible)
            return new Color(1f, 1f, 1f, 0.7f);

        return new Color(1f, 1f, 1f, OpacityOf(entity));
    }

    /// <summary>
    /// How solid another player is drawn, so a crowd can be seen through.
    /// </summary>
    /// <remarks>
    /// Only ever applied to other people. Fading your own character, or the monsters shooting at
    /// you, would make the game harder to read rather than easier -- which is the whole point of
    /// the setting.
    /// </remarks>
    private float OpacityOf(Entity entity)
    {
        var options = Options;
        if (options == null || entity.Desc is not { IsPlayer: true } || ReferenceEquals(entity, _map.Player))
            return 1f;

        var player = _map.Player;
        bool guildmate = player != null && !string.IsNullOrEmpty(player.Guild) && entity.Guild == player.Guild;

        if (guildmate ? !options.FadeGuildMembers : !options.FadePlayers)
            return 1f;

        return Mathf.Clamp(options.Opacity, 0.1f, 1f);
    }
}

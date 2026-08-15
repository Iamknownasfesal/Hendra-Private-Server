//! One connection, from hello to goodbye.
//!
//! A session owns its link and nothing else. It decodes what arrives, turns it into a message for
//! whichever world the player is in, and hands it on; it never touches a world directly. Snapshots
//! travel the other way without passing through here at all. The world writes to the player's
//! connection itself.
//!
//! # Where items live
//!
//! Equipment and the backpack are one table. Slots 0 to 3 are worn and 4 upwards are carried, which
//! is how the game has always numbered them, and it means every move between them is a move within
//! one container, so it goes through the same transactional path a vault move does rather than
//! needing a second mechanism.
//!
//! # Bags cross a boundary
//!
//! A bag lives in the world, in memory and gone in a minute, while an inventory lives in the
//! database. A move between them cannot be one transaction, so it is two, and the order they happen
//! in decides what a crash between them costs.
//!
//! Removing from the ephemeral side first means a crash loses the item. Writing to the durable side
//! first means a crash duplicates it. Both are bad and they are not equally bad: a lost item is one
//! player's bad evening, and a duplicated one is an economy. So the ephemeral side always goes
//! first, and if the second step fails the first is undone.
//!
//! The window is a few milliseconds, on an item that was free and would have expired in a minute
//! regardless.
//!
//! # Changing world
//!
//! A session outlives the world it is in. Stepping through a portal leaves one world and joins
//! another, and the player's identity changes with it: a handle names a slot in a particular
//! world's storage and means nothing anywhere else. The client is told by a second `Welcome`, which
//! it must treat as "forget everything", because its snapshot history was measured against a world that no
//! longer applies.

use std::sync::Arc;

use tokio::sync::mpsc;

use hendra_content::ObjectType;
use hendra_net::message::{
    ClientMessage, ContainerId, PROTOCOL_VERSION, RejectReason, ServerMessage, SlotLocation,
};
use hendra_net::{Delivery, EntityId, Reader, Writer};
use hendra_sim::Handle;
use hendra_store::{Location, Store};
use hendra_transport::{Link, Received};

use crate::world_task::{ToWorld, WorldHandle};
use crate::worlds::Worlds;

/// Where a player currently is.
struct Placement {
    world: WorldHandle,
    handle: Handle,
}

/// How many slots are worn rather than carried.
pub(crate) const EQUIPPED_SLOTS: u8 = hendra_net::slot::EQUIPPED_SLOTS as u8;

/// How often a character is written down while it is being played.
///
/// Three seconds, from `wServer/realm/entities/player/Player.KeepAlive.cs:11`, where `PingPeriod`
/// is 3000 and the pong that answers each ping is what drives `UpdateOnPing` into saving the
/// character (`Player.KeepAlive.cs:113-134`). Ours is a timer rather than a reply to a ping, which
/// writes on the same cadence for a player whose client has gone quiet, but the interval is the
/// original's and so is what it costs a session to lose.
const CHECKPOINT: std::time::Duration = std::time::Duration::from_secs(3);

/// How long a connection may go without answering a ping before it is dropped.
///
/// Twelve seconds, from `Player.DcThresold` (`wServer/realm/entities/player/Player.KeepAlive.cs:12`),
/// measured from the last pong rather than from the last ping — so a client that misses one ping and
/// answers the next is never dropped, and only one that has stopped answering entirely is. With a
/// ping every three seconds that is four missed answers in a row.
const KEEPALIVE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(12);

/// One connection's account of the clock it shares with its client.
///
/// Mirrors the running averages in `Player.Pong` (`Player.KeepAlive.cs:100-111`): every pong carries
/// back the server uptime the ping was stamped with and the client's own uptime as it answered, so
/// the round trip is the first difference and the offset between the two clocks is the second. Both
/// are averaged over the whole session, which is what makes them steady enough to subtract from a
/// timestamp; a single sample is mostly jitter.
/// Nothing here remembers which ping is outstanding, because the original does not either: a pong
/// carries its serial back, and `Player.Pong` reads it from the packet rather than comparing it
/// against a stored one. Only the queue's keepalive keeps a serial to check
/// (`networking/Client.KeepAlive.cs:11, 50`).
#[derive(Debug, Default, Clone, Copy)]
struct Clocks {
    /// Pongs counted, which is the divisor of both averages.
    count: u32,

    /// Sum of `server_now - client_time`, whose average is the original's `TimeMap`.
    offset_sum: i64,

    /// Sum of half-round-trips, whose average is the original's `Latency`.
    latency_sum: i64,
}

impl Clocks {
    /// Records an answered ping, given the server's uptime as the pong landed.
    ///
    /// Wrapping arithmetic throughout, because both clocks are milliseconds in a 32-bit field and a
    /// server up for seven weeks would otherwise start reading its own uptime as a huge jump.
    fn note(&mut self, server_now_ms: u32, serial: u32, client_time_ms: u32) {
        self.count += 1;
        self.offset_sum += server_now_ms.wrapping_sub(client_time_ms) as i32 as i64;
        self.latency_sum += (server_now_ms.wrapping_sub(serial) as i32 as i64) / 2;
    }

    /// What has to be added to a client timestamp to read it in server time: `Player.TimeMap`.
    fn offset_ms(&self) -> i64 {
        self.offset_sum / self.count.max(1) as i64
    }

    /// One-way latency in milliseconds: `Player.Latency`.
    fn latency_ms(&self) -> i64 {
        self.latency_sum / self.count.max(1) as i64
    }
}

/// The server's own uptime in milliseconds, which is what a ping is stamped with.
///
/// `Player.KeepAlive.cs:95` uses `RealmTime.TotalElapsedMs`, the logic loop's stopwatch. Ours counts
/// from the same place: when the process started serving.
fn uptime_ms(context: &Context) -> u32 {
    context.started.elapsed().as_millis() as u32
}

/// Stamps a ping with the server's uptime and sends it.
async fn send_ping(link: &mut Link, context: &Context) {
    let mut buffer = Vec::new();
    ServerMessage::Ping {
        serial: uptime_ms(context),
    }
    .encode(&mut Writer::new(&mut buffer));

    // Reliable, because a ping lost on the wire is a step towards a disconnect that the player did
    // nothing to earn. The original's transport has no unreliable half at all.
    let _ = link.send(Delivery::Stream, &buffer).await;
}

/// Everything a session needs to answer for one player.
pub struct Context {
    pub worlds: Arc<Worlds>,
    pub store: Store,
    pub catalog: Arc<hendra_content::Catalog>,
    pub kit: crate::accounts::StartingKit,

    /// Verifies session tokens. This process cannot mint one.
    pub key: hendra_auth::TokenKey,

    /// Who is trading with whom, and who is online.
    pub trades: Arc<crate::trades::Trades>,

    /// When the server started, which is all `/uptime` needs.
    pub started: std::time::Instant,

    /// How many may play at once. Anybody arriving past it waits.
    pub capacity: usize,

    /// Who is waiting for a place.
    pub queue: std::sync::Mutex<crate::queue::Queue>,

    /// Standing guild invitations, by the invitee's name folded to lower case.
    ///
    /// `Player.GuildInvite` (`Player.cs:201`), which is one nullable guild id held on the invited
    /// player rather than a row anywhere. It lives for as long as they stay connected and is
    /// checked by `/join`, which is what makes an invitation an offer rather than an act: the
    /// officer proposes, and the invitee decides.
    ///
    /// Held here rather than on the session because the officer's session is the one that writes it
    /// and the invitee's is the one that reads it.
    pub guild_invites: std::sync::Mutex<std::collections::HashMap<String, i64>>,

    /// Every track the deployment can play, by name and without its extension.
    ///
    /// `Resources.MusicNames`, which is the stems of every mp3 under `web/music`
    /// (`common/resources/Resources.cs:98-111`). Read once at startup for the same reason the
    /// original reads it once: it is a directory listing that does not change while the process
    /// runs. `/music` lists it and refuses anything not in it, so a mistyped name cannot leave a
    /// world playing nothing.
    ///
    /// Empty for a deployment whose music directory is not there, which turns the refusal off
    /// rather than refusing every name.
    pub music: Vec<String>,
}

/// Turns a wire slot into a durable one.
///
/// Returns `None` for a container that is not durable, which today means a bag. Where the two ends
/// of the character's own table sit is `hendra_net::slot`, so the client's extension and this agree
/// about it by construction rather than by both having been written the same way.
fn locate(where_: SlotLocation, character_id: i64, account_id: i64) -> Option<Location> {
    Some(match where_ {
        SlotLocation::Equipment { .. } | SlotLocation::Inventory { .. } => Location::Inventory {
            character_id,
            slot: hendra_net::slot::durable(where_)?,
        },

        SlotLocation::Vault { slot } => Location::Vault {
            account_id,
            slot: slot as i16,
        },

        // Neither a bag nor the ground is durable; both are handled before this is reached.
        SlotLocation::Gift { slot } => Location::Gift {
            account_id,
            slot: slot as i16,
        },

        SlotLocation::Bag { .. } | SlotLocation::Ground => return None,
    })
}

/// Handles one connection for its lifetime.
pub async fn serve(mut link: Link, context: Arc<Context>, entry: WorldHandle) {
    let peer = link.remote_address();

    // A world can ask this session for something only a session can do, such as leaving for the
    // castle when a realm closes. One slot is enough: there is one such order, and it happens once.
    let (to_session, mut orders) = mpsc::channel(4);

    // One slot: a character dies once, and the session ends when it does.
    let (to_death, mut died) = mpsc::channel(1);

    // One slot as well: what a refresh reads is whatever is true when it reads it, so a second
    // request waiting behind the first would read the same thing twice.
    let (to_refresh, mut refresh) = mpsc::channel(1);

    let Some((player, lock, mut placement)) =
        handshake(&mut link, &context, &entry, &to_session, &to_death).await
    else {
        link.close("handshake refused");
        return;
    };

    let name = player.character.name.clone();
    tracing::info!(
        %peer, %name,
        account = player.account.id,
        character = player.character.id,
        world = %placement.world.name,
        "session started"
    );

    // The server may be full. Whoever is waiting waits here, before anything is claimed or joined:
    // a place in a line is not a place in a world, and taking either before the other would be
    // holding a world open for somebody who has not got in yet.
    if !wait_for_room(&mut link, &context, &player).await {
        let _ = context.store.release_lock(&lock).await;
        link.close("gave up waiting");
        return;
    }

    // Whatever else was playing on this account has already been ended, by `take_lock` on the way
    // in. This is the same claim run once more, for the registry rather than for the lock: a
    // session that arrived while this one was waiting for room is one the lock never saw.
    let ended = context.trades.claim(player.account.id);
    if ended > 0 {
        tracing::info!(%name, account = player.account.id, ended, "took over an account");
    }

    // Reachable by name from now on, which is what lets somebody else ask them to trade.
    context.trades.arrived(
        &name,
        player.account.id,
        player.character.id,
        player.account.admin_rank,
        link.sender(),
        to_refresh,
    );
    context.trades.moved(&name, &placement.world.key);

    // What it is carrying, before it can move any of it. The ground is not sent here: it follows
    // the player from the world's own tick, a sight circle at a time.
    send_containers(&mut link, &context.catalog, &context.store, &player).await;

    // One per connection. A limit shared between players would let a busy world silence a quiet
    // one, and a limit that outlived a connection would follow the wrong person.
    let mut limit = crate::chat::Limit::new();

    // What the vault is at, which every move quotes back. Ours: the original has no version to keep,
    // because two clients on one account each get their own `Vault` world with its own chest
    // entities over the same redis fields (`realm/worlds/logic/Vault.cs:23-28`, `:47-50`) and it
    // does not reconcile them. The account lock admits one session at a time here, so this belongs
    // to the session and there is nobody to broadcast to. See `crate::vault`.
    let mut vault_version: u32 = 0;

    // Whether the last pass round the loop was spent in the vault, so arriving in one hands the
    // panel the whole of it exactly once. On the tick rather than on entry, because the panel has
    // nothing to draw until this lands and sending it during the handshake sends it to a connection
    // not yet listening. The original faces the same ordering and answers it the same way: its
    // chests are placed in `Init` but only reach the client as ordinary entities on a later tick
    // (`realm/worlds/logic/Vault.cs:52-58`).
    let mut in_vault = false;

    // Whether this session ended by giving the character up. A prestige rewrites the row to a
    // level-one character while the world is still holding the body that earned the fame, so the
    // save on the way out is skipped: the body no longer stands for anything that should be
    // written down.
    let mut prestiged = false;

    // The first tick fires immediately, which would write a character that has just been read, so
    // it is spent here rather than in the loop.
    let mut checkpoint = tokio::time::interval(CHECKPOINT);
    checkpoint.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    checkpoint.tick().await;

    // The first ping goes out now rather than in three seconds, which is what the original does:
    // `_pingTime` starts a whole period in the past so the first `Player.Tick` sends one
    // (`Player.KeepAlive.cs:43-46, 88-96`). The deadline starts here too, from `_pongTime` being
    // set to the same instant on that first tick.
    let mut clocks = Clocks::default();
    send_ping(&mut link, &context).await;

    let deadline = tokio::time::sleep(KEEPALIVE_DEADLINE);
    tokio::pin!(deadline);

    loop {
        let received = tokio::select! {
            received = link.recv() => received,

            // A client that has stopped answering. Four pings have gone unanswered by the time this
            // fires, so this is a connection that is gone rather than one that is slow.
            _ = &mut deadline => {
                tracing::info!(
                    %name,
                    account = player.account.id,
                    "connection timeout (keepalive)"
                );
                break;
            }

            // A character is written down on a timer as well as when it leaves, so a server that
            // stops without warning costs a player three seconds of play rather than a session of
            // it. Waited on beside the client rather than counted between messages, because a
            // player who has stopped sending anything is exactly the one whose last minutes would
            // otherwise go unwritten.
            _ = checkpoint.tick() => {
                // The ping rides the same beat as the save, because in the original they are the
                // same event: `Player.KeepAlive` sends the ping and then calls `UpdateOnPing`,
                // which renews the lock and writes the character (`Player.KeepAlive.cs:88-134`).
                send_ping(&mut link, &context).await;

                // The lock is pushed out on the same beat. The original renews on every ping
                // (`player/Player.KeepAlive.cs:118-126`) and disconnects when the renewal is
                // refused, which is what being taken over looks like from inside: another session
                // holds the account, and this one must stop writing to it before its next
                // checkpoint does.
                match context.store.renew_lock(&lock).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::info!(%name, account = player.account.id, "lost the account lock");
                        break;
                    }
                    // A renewal that could not be asked leaves this session unable to say whether
                    // it still holds the account, and one that has stopped renewing loses it
                    // within the minute either way. The original ends the connection on the same
                    // footing — its `catch` around `RenewLock` disconnects exactly as a refusal
                    // does (`player/Player.KeepAlive.cs:113-126`) — and the final save on the way
                    // out is conditional, so a session that turns out to still hold the lock
                    // writes and one that does not writes nothing.
                    Err(err) => {
                        tracing::warn!(%err, %name, "could not renew the account lock");
                        break;
                    }
                }

                if let Err(err) = save_progress(&context, &player, &placement, Written::CharacterOnly).await {
                    tracing::warn!(%err, %name, "a checkpoint could not be written");
                }
                continue;
            }

            // Somebody else changed what this player holds: a trade completed, an item sold, a
            // gift arrived. Waited on beside the client, because a player looking at a stale vault
            // is not going to send anything that would prompt a re-read.
            _ = refresh.recv() => {
                send_containers(&mut link, &context.catalog, &context.store, &player).await;

                // The pack changed under this session, so the body is recomputed from it as well.
                // The original's recalculation hangs off the inventory rather than off whoever
                // caused the change (`Player.cs:463`), so it does not matter that the write came
                // from somewhere else.
                refresh_equipment(&context, &player, &placement).await;
                continue;
            }

            // A death, which ends the session. Waited on beside the client rather than checked
            // between messages, because somebody who dies standing still sends nothing.
            departed = died.recv() => {
                let Some(departed) = departed else { break };

                match die(
                    &mut link,
                    &context,
                    &player,
                    &placement,
                    &name,
                    &to_session,
                    &to_death,
                    departed,
                )
                .await
                {
                    AfterDeath::Over => break,
                    AfterDeath::SentHome(next) => {
                        placement = next;
                    }

                    // Nowhere to put somebody who should not have died. The world that killed them
                    // has already let them go, so a session left open here is a client watching a
                    // map load for a world it will never be admitted to. Ending it puts them back
                    // at the character list, which is somewhere.
                    AfterDeath::Nowhere => {
                        tracing::error!(%name, "a death left nowhere to send the player");
                        break;
                    }
                }
                continue;
            }

            // A world asking for something only a session can do. Waited on beside the client
            // rather than checked between messages, because a player standing still sends nothing
            // and a closing realm should still empty.
            order = orders.recv() => {
                let Some(order) = order else { break };
                match order {
                    crate::world_task::Order::GoTo(destination) => {
                        if let Some(next) = go_to(
                            &mut link,
                            &context,
                            &player,
                            &placement,
                            &name,
                            &destination,
                            &to_session,
                            &to_death,
                        )
                        .await
                        {
                            placement = next;
                        }
                    }
                }
                continue;
            }
        };

        let Some(received) = received else {
            break;
        };

        match dispatch(&received, &placement).await {
            Outcome::Continue => {}
            Outcome::Stop => break,

            Outcome::Pong {
                serial,
                client_time_ms,
            } => {
                // Any pong refreshes the deadline, matching `Player.Pong`, which sets `_pongTime`
                // without looking at the serial at all (`Player.KeepAlive.cs:100-111`). Only the
                // queue's own keepalive checks it (`networking/Client.KeepAlive.cs:50`). Being
                // answered is the proof of life; being answered with the newest serial is not.
                deadline
                    .as_mut()
                    .reset(tokio::time::Instant::now() + KEEPALIVE_DEADLINE);

                clocks.note(uptime_ms(&context), serial, client_time_ms);

                tracing::trace!(
                    %name,
                    latency_ms = clocks.latency_ms(),
                    offset_ms = clocks.offset_ms(),
                    "pong"
                );
            }

            Outcome::Move { from, to } => {
                move_item(&mut link, &context, &player, &placement, from, to).await;
            }

            Outcome::Chat(line) => {
                if let Some((command, rest)) =
                    say_something(&mut link, &context, &player, &placement, &mut limit, &line).await
                {
                    run_command(
                        &mut link,
                        &context,
                        &player,
                        &name,
                        &mut placement,
                        &to_session,
                        &to_death,
                        &command,
                        &rest,
                    )
                    .await;
                }
            }

            Outcome::UseItem {
                container,
                slot,
                x,
                y,
            } => {
                use_item(
                    &mut link,
                    &context,
                    &player,
                    &placement,
                    container,
                    slot,
                    (x, y),
                )
                .await;
            }

            Outcome::VaultMove {
                version,
                from_chest,
                from_slot,
                to_chest,
                to_slot,
            } => {
                vault_move(
                    &mut link,
                    &context,
                    &player,
                    &placement,
                    &mut vault_version,
                    version,
                    (from_chest, from_slot),
                    (to_chest, to_slot),
                )
                .await;
            }

            Outcome::VaultBuy { chest_count } => {
                vault_buy(
                    &mut link,
                    &context,
                    &player,
                    &placement,
                    &mut vault_version,
                    chest_count,
                )
                .await;
            }

            Outcome::Trade(asked) => {
                trade(&mut link, &context, &player, &name, asked).await;
            }

            Outcome::Say(line) => say(&mut link, &line).await,

            Outcome::TeleportTo(to) => {
                teleport(&mut link, &context, &player, &placement, &to).await;
            }

            Outcome::Escape => {
                if let Some(next) = go_to(
                    &mut link,
                    &context,
                    &player,
                    &placement,
                    &name,
                    crate::commands::NEXUS,
                    &to_session,
                    &to_death,
                )
                .await
                {
                    placement = next;
                    context.trades.moved(&name, &placement.world.key);
                }
            }

            Outcome::Buy(sale) => buy(&mut link, &context, &player, &placement, sale).await,

            Outcome::BuyHallUpgrade(upgrade) => {
                buy_hall_upgrade(&mut link, &context, &player, upgrade).await;
            }

            Outcome::Prestige => {
                match context
                    .store
                    .prestige(
                        player.account.id,
                        player.character.id,
                        &starting_stats(&context.catalog, player.character.class),
                    )
                    .await
                {
                    Ok(earned) => {
                        say(&mut link, &format!("You earned {earned} prestige.")).await;

                        // The character is level one with nothing now, so the world is holding a
                        // body that no longer matches what is stored. Leaving is the honest end,
                        // and the body must not be written down on the way out: it still carries
                        // the level and the fame the prestige has just taken, and saving it would
                        // hand both back while the prestige stayed paid. The original closes the
                        // same gap from the other side, resetting the live player and saving that
                        // before it disconnects (`networking/handlers/PrestigeHandler.cs:41-64`).
                        prestiged = true;
                        break;
                    }
                    Err(hendra_store::StoreError::Refused(why)) => say(&mut link, why).await,
                    Err(_) => say(&mut link, "try again shortly").await,
                }
            }

            Outcome::PrestigeBuy(offer) => {
                buy_with_prestige(&mut link, &context, &player, offer).await;
            }

            Outcome::Guild(asked) => {
                guild(&mut link, &context, &player, &name, asked).await;
            }

            Outcome::Market(command) => {
                market(&mut link, &context, &player, &placement, command).await;
            }

            Outcome::EditList { list, name, add } => {
                edit_list(&mut link, &context, &player, list, &name, add).await;
            }

            Outcome::Travel {
                portal_type,
                portal,
            } => {
                // Stepping into the guild hall portal has to mean what typing the command means,
                // or the two are different doors into different rooms.
                if context.worlds.destination_of(portal_type) == Some(crate::commands::GUILD_HALL) {
                    if let Some(next) = enter_guild_hall(
                        &mut link,
                        &context,
                        &player,
                        &placement,
                        &name,
                        &to_session,
                        &to_death,
                    )
                    .await
                    {
                        placement = next;
                        context.trades.moved(&name, &placement.world.key);
                    }
                    continue;
                }

                match travel(
                    &mut link,
                    &context,
                    &player,
                    &placement,
                    &name,
                    portal_type,
                    portal,
                    &to_session,
                    &to_death,
                )
                .await
                {
                    Some(next) => {
                        tracing::info!(
                            %name,
                            from = %placement.world.name,
                            to = %next.world.name,
                            "travelled"
                        );
                        placement = next;

                        // Arriving in the vault means arriving at your own chests.
                        if placement.world.name.as_ref() == "Vault" {
                            place_chests(&context, &player, &placement).await;
                        }
                    }
                    // Staying put is the right answer when a destination cannot be opened: the
                    // player keeps playing where they are rather than being disconnected over a
                    // dungeon that failed to load.
                    None => tracing::warn!(%name, "portal led nowhere; staying put"),
                }
            }
        }

        // Arriving in the vault hands the panel the whole of it. Checked here rather than at each
        // of the doors into the room, because there are several — a portal, `/vault`, a command
        // that moves somebody else — and one of them forgetting is a vault that draws itself empty.
        let here = placement.world.name.as_ref() == "Vault";
        if here && !in_vault {
            send_vault(&mut link, &context, &player, vault_version).await;
        }
        in_vault = here;
    }

    // Before anything else: a trade left half-agreed by a disconnection is the shape a duplication
    // is built on.
    context.trades.left(&name);

    // A standing invitation lives on the invited player and so leaves with them, as
    // `Player.GuildInvite` does when the client that held it is gone.
    if let Ok(mut invites) = context.guild_invites.lock() {
        invites.remove(&name.to_lowercase());
    }

    // Written before the body is dropped, because the body is the only copy: leaving first would
    // ask a world that no longer holds this player what it became, and the answer would be nothing.
    // The original saves in the same order, in `wServer/networking/Client.cs:215-234`, where
    // `Save()` runs before `Manager.Disconnect(this)` takes the player out of its world.
    //
    // Items are not saved here; they are written as they move, so a checkpoint that rewrote slots
    // wholesale could undo a move that had committed.
    //
    // A character that was given up for prestige is the one thing not written: the row has already
    // been reset and the body is what it was before, so writing it would undo the reset.
    if !prestiged
        && let Err(err) = save_progress(&context, &player, &placement, Written::WithClass).await
    {
        tracing::warn!(%err, %name, "could not save the character");
    }

    placement
        .world
        .send(ToWorld::Leave {
            handle: placement.handle,
        })
        .await;

    // Last, after the write it was guarding. Released rather than left to expire so a player who
    // logs out can log straight back in, and conditional on this session's own token so one that
    // was taken over cannot free the lock its replacement is holding. The original releases in the
    // same place, at the end of `Client.Save()` (`networking/Client.cs:236`).
    if let Err(err) = context.store.release_lock(&lock).await {
        tracing::warn!(%err, %name, "could not release the account lock");
    }

    tracing::info!(%peer, %name, "session ended");
}

/// Takes the lock that lets this session write to the account, or refuses the login.
///
/// Follows `ConnectManager.cs:202-216`: try once, and if the account is held, end whatever is
/// playing on it here and try again. The retry is what makes a dropped connection recoverable —
/// the common refusal is not somebody cheating but somebody reconnecting — and the session being
/// taken over needs a moment to finish its own shutdown and release what it holds, so the retry is
/// given one rather than being a single immediate second attempt.
///
/// A lock still held after that is a session somewhere this process cannot end, which is exactly
/// what the lock is for. Whoever asked is told, as the original tells them, how long until it
/// lapses.
async fn take_lock(
    link: &mut Link,
    context: &Context,
    account_id: i64,
) -> Option<hendra_store::AccountLock> {
    /// How long the session being taken over is given to release what it holds.
    const HANDOVER: std::time::Duration = std::time::Duration::from_millis(250);

    /// How many times the handover is waited out before the login is refused.
    const TRIES: usize = 4;

    match context.store.acquire_lock(account_id).await {
        Ok(Some(lock)) => return Some(lock),
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(%err, account = account_id, "could not take the account lock");
            refuse(link, RejectReason::AlreadyPlaying).await;
            return None;
        }
    }

    let ended = context.trades.claim(account_id);
    tracing::info!(
        account = account_id,
        ended,
        "account is held; taking it over"
    );

    for _ in 0..TRIES {
        tokio::time::sleep(HANDOVER).await;
        match context.store.acquire_lock(account_id).await {
            Ok(Some(lock)) => return Some(lock),
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(%err, account = account_id, "could not take the account lock");
                break;
            }
        }
    }

    let seconds = context
        .store
        .lock_seconds_left(account_id)
        .await
        .unwrap_or(0);
    tracing::info!(account = account_id, seconds, "refused: account in use");
    refuse(link, RejectReason::AlreadyPlaying).await;
    None
}

/// What a player asked for about a trade.
#[derive(Debug, Clone, PartialEq)]
enum Trade {
    Request(String),
    Change(Vec<bool>),
    Accept { mine: Vec<bool>, theirs: Vec<bool> },
    Cancel,
}

/// A guild command with its names owned, so it can outlive the buffer it was decoded from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GuildAsk {
    Create(String),
    Invite(String),
    Join(String),
    Remove(String),
    SetRank(String, u8),
    SetBoard(String),
    Leave,
}

fn owned_guild(command: hendra_net::GuildCommand<'_>) -> GuildAsk {
    use hendra_net::GuildCommand as From;

    match command {
        From::Create { name } => GuildAsk::Create(name.to_owned()),
        From::Invite { name } => GuildAsk::Invite(name.to_owned()),
        From::Join { name } => GuildAsk::Join(name.to_owned()),
        From::Remove { name } => GuildAsk::Remove(name.to_owned()),
        From::SetRank { name, rank } => GuildAsk::SetRank(name.to_owned(), rank),
        From::SetBoard { text } => GuildAsk::SetBoard(text.to_owned()),
        From::Leave => GuildAsk::Leave,
    }
}

enum Outcome {
    Continue,
    Stop,

    /// The client answered a ping.
    ///
    /// Carried back to the session rather than settled here, because the deadline it refreshes and
    /// the clocks it compares belong to the connection.
    Pong {
        serial: u32,
        client_time_ms: u32,
    },

    /// The player stepped into a portal of this object type.
    ///
    /// The portal itself is carried alongside its type, because a dungeon belongs to the portal
    /// rather than to the kind of portal: `Portal.CreateWorld` builds a fresh `World` and hangs it
    /// off that one entity (`Portal.cs:76-77`, `:90`), so two portals of the same type lead to two
    /// different rooms and everybody through one portal lands in the same one.
    Travel { portal_type: u16, portal: EntityId },

    /// The player wants to move an item.
    Move {
        from: SlotLocation,
        to: SlotLocation,
    },

    /// The player said something.
    ///
    /// Handled by the caller rather than sent straight to the world, because whether they may speak
    /// at all is an account question and the world does not hold accounts.
    Chat(String),

    /// Something to do with trading.
    ///
    /// Handled by the caller for the same reason a move is: a trade is two connections and a
    /// durable inventory, and the world holds neither.
    Trade(Trade),

    /// The player is leaving for the nexus.
    Escape,

    /// The player wants to be where another player is.
    ///
    /// Handled by the caller rather than sent straight to the world, because whether the other
    /// player has locked them out is an account question and the world holds no accounts.
    TeleportTo(String),

    /// Something to say to the player and nothing else.
    Say(String),

    /// The player is buying what a merchant is selling.
    Buy(crate::world_task::Sale),

    /// The player is buying a larger hall for their guild.
    BuyHallUpgrade(crate::world_task::HallUpgrade),

    /// The player is giving up this character's fame for prestige.
    Prestige,

    /// The player is buying one of the things prestige buys.
    PrestigeBuy(u8),

    /// Something to do with a guild.
    Guild(GuildAsk),

    /// Something to do with the market.
    Market(hendra_net::MarketCommand),

    /// Add or remove somebody from one of the account's lists.
    EditList {
        list: hendra_net::AccountList,
        name: String,
        add: bool,
    },

    /// The player wants to use what is in a slot.
    ///
    /// Handled by the caller rather than sent straight to the world, because using an item can
    /// consume it and the inventory is durable.
    UseItem {
        container: hendra_net::EntityId,
        slot: u16,
        x: f32,
        y: f32,
    },

    /// The player moved something in the vault panel.
    VaultMove {
        version: u32,
        from_chest: i16,
        from_slot: i16,
        to_chest: i16,
        to_slot: i16,
    },

    /// The player asked to buy one more vault chest.
    VaultBuy {
        chest_count: u32,
    },
}

/// Reads the opening message and either admits the player or explains why not.
async fn handshake(
    link: &mut Link,
    context: &Context,
    entry: &WorldHandle,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<(
    crate::accounts::Session,
    hendra_store::AccountLock,
    Placement,
)> {
    let received = link.recv().await?;
    let payload = received.into_payload();
    let mut reader = Reader::new(&payload);

    let message = match ClientMessage::decode(&mut reader) {
        Ok(message) => message,
        Err(err) => {
            tracing::warn!(%err, "undecodable opening message");
            return None;
        }
    };

    let ClientMessage::Hello {
        protocol,
        token,
        character,
    } = message
    else {
        tracing::warn!("first message was not a hello");
        return None;
    };

    if protocol != PROTOCOL_VERSION {
        refuse(link, RejectReason::VersionMismatch).await;
        tracing::info!(
            protocol,
            expected = PROTOCOL_VERSION,
            "refused: wrong version"
        );
        return None;
    }

    let player = match crate::accounts::log_in(
        &context.store,
        &context.catalog,
        &context.kit,
        &context.key,
        token,
        character as i64,
    )
    .await
    {
        Ok(player) => player,
        Err(crate::accounts::LoginError::Banned) => {
            refuse(link, RejectReason::Banned).await;
            return None;
        }
        Err(crate::accounts::LoginError::NoToken)
        | Err(crate::accounts::LoginError::BadToken(_)) => {
            refuse(link, RejectReason::BadToken).await;
            return None;
        }
        Err(err) => {
            tracing::warn!(%err, "login failed");
            refuse(link, RejectReason::BadToken).await;
            return None;
        }
    };

    // Before the world is joined and before the character is trusted. Everything after this point
    // writes to the account, and nothing may until this session is the only thing doing so.
    let lock = take_lock(link, context, player.account.id).await?;

    // Read again now the lock is held. The session being taken over writes its character on the way
    // out, and that write lands during the handover: the copy read a moment ago is from before it.
    // Playing on the stale copy would hand back whatever the previous session had earned and then
    // make it permanent at the first checkpoint. The original re-reads in the same place and says
    // why — `acc.Reload(); // make sure we have the latest data` (`realm/ConnectManager.cs:219`).
    let mut player = player;

    // Carried on the session from here on, because every write this session makes to the character
    // is conditional on it. Handed back alongside as well, for the renewal and the release, which
    // are the two things that act on the lock itself rather than write under it.
    player.lock = Some(lock);

    match context.store.character(player.character.id).await {
        Ok(fresh) => player.character = fresh,
        Err(err) => {
            tracing::warn!(%err, character = player.character.id, "could not re-read a character");
            let _ = context.store.release_lock(&lock).await;
            refuse(link, RejectReason::NoSuchCharacter).await;
            return None;
        }
    }

    let handle = match join(
        link,
        entry,
        &player.character.name,
        arrival_of(&player, context).await,
        orders,
        died,
    )
    .await
    {
        Some(handle) => handle,

        // Given back rather than left to lapse: a world that would not take this player should not
        // also cost them a minute of not being able to log in anywhere.
        None => {
            let _ = context.store.release_lock(&lock).await;
            return None;
        }
    };

    // Both lists, immediately after the welcome, as `ConnectManager` sends them immediately after
    // `MapInfo` (`ConnectManager.cs:355-368`). Sent once per session rather than per world: they
    // belong to the account, and walking through a portal does not change who is on them.
    send_account_list(link, context, &player, hendra_net::AccountList::Locked).await;
    send_account_list(link, context, &player, hendra_net::AccountList::Ignored).await;

    Some((
        player,
        lock,
        Placement {
            world: entry.clone(),
            handle,
        },
    ))
}

/// Sends the player everything they are carrying and storing.
/// Recomputes what a player wears and tells the world.
///
/// Called after any move that succeeded rather than only after one touching a worn slot, because
/// the cost is one query and the failure mode of getting the condition wrong is a ring that keeps
/// working after it has been taken off.
async fn refresh_equipment(
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
) {
    let Ok(character) = context.store.character(player.character.id).await else {
        return;
    };

    placement
        .world
        .send(ToWorld::Equipment {
            handle: placement.handle,
            bonuses: worn_bonuses(&context.catalog, &character.inventory),
            weapon_damage: worn_weapon_damage(&context.catalog, &character.inventory),
            weapon: worn_weapon(&context.catalog, &character.inventory),
            set_skin: worn_set_skin(&context.catalog, &character.inventory),
        })
        .await;
}

async fn send_containers(
    link: &mut Link,
    catalog: &hendra_content::Catalog,
    store: &Store,
    player: &crate::accounts::Session,
) {
    let inventory = store
        .character(player.character.id)
        .await
        .map(|character| character.inventory)
        .unwrap_or_default();

    // Worn and carried live in one table, so they are split back apart on the way out.
    let worn: Vec<(u16, u16)> = inventory
        .iter()
        .filter(|(slot, _)| *slot < EQUIPPED_SLOTS as i16)
        .filter_map(|(slot, item)| Some((*slot as u16, number(catalog, *item)?)))
        .collect();
    let carried: Vec<(u16, u16)> = inventory
        .iter()
        .filter(|(slot, _)| *slot >= EQUIPPED_SLOTS as i16)
        .filter_map(|(slot, item)| {
            Some((
                (*slot - EQUIPPED_SLOTS as i16) as u16,
                number(catalog, *item)?,
            ))
        })
        .collect();

    let vault: Vec<(u16, u16)> = store
        .vault(player.account.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(slot, item)| Some((slot as u16, number(catalog, item)?)))
        .collect();

    for (container, slots) in [
        (ContainerId::Equipment, worn),
        (ContainerId::Inventory, carried),
        (ContainerId::Vault, vault),
    ] {
        let mut buf = Vec::new();
        ServerMessage::Container { container, slots }.encode(&mut Writer::new(&mut buf));
        let _ = link.send(Delivery::Stream, &buf).await;
    }

    // And the two stacks, which live outside the pack. A player who cannot see how many potions
    // they hold cannot decide whether to drink one.
    let held = store
        .character(player.character.id)
        .await
        .map(|character| (character.health_potions, character.magic_potions))
        .unwrap_or((0, 0));

    let mut buf = Vec::new();
    ServerMessage::Stacks {
        health: held.0.clamp(0, u16::MAX as i32) as u16,
        magic: held.1.clamp(0, u16::MAX as i32) as u16,
    }
    .encode(&mut Writer::new(&mut buf));
    let _ = link.send(Delivery::Stream, &buf).await;
}

/// Carries out a move, or explains why not.
async fn move_item(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    from: SlotLocation,
    to: SlotLocation,
) {
    let character_id = player.character.id;
    let account_id = player.account.id;

    // Everything that can be settled from the pair of ends alone, before either is read and before
    // the bag dispatch below: a bag is the one route into the player's pack that does not pass
    // through the rest of this, and it is the route a client would use to reach a slot it never
    // bought.
    let trading = context.trades.partner(&player.character.name).is_some();
    let reachable = reachable_slots(context, player).await;
    if let Some(why) = move_refusal(from, to, trading, &reachable) {
        snap_back(link, context, player, why).await;
        return;
    }

    // Bags are not durable, so a move touching one is two steps rather than one transaction.
    match (from, to) {
        (SlotLocation::Bag { entity, slot }, destination) => {
            take_from_bag(link, context, player, placement, entity, slot, destination).await;
            return;
        }
        (source, SlotLocation::Bag { entity, .. }) => {
            put_in_bag(link, context, player, placement, source, Some(entity)).await;
            return;
        }
        (source, SlotLocation::Ground) => {
            put_in_bag(link, context, player, placement, source, None).await;
            return;
        }
        (SlotLocation::Ground, _) => {
            snap_back(link, context, player, "there is nothing at your feet to take").await;
            return;
        }
        _ => {}
    }

    // The vault is a place. Reaching into it from a dungeon would make the room decoration, and
    // the check belongs here rather than in the client, which is not in a position to be trusted
    // about where it is standing.
    let touches_vault =
        matches!(from, SlotLocation::Vault { .. }) || matches!(to, SlotLocation::Vault { .. });
    if touches_vault && placement.world.name.as_ref() != "Vault" {
        snap_back(link, context, player, "you are not at your vault").await;
        return;
    }

    let (Some(source), Some(destination)) = (
        locate(from, character_id, account_id),
        locate(to, character_id, account_id),
    ) else {
        snap_back(link, context, player, "that cannot be moved").await;
        return;
    };

    // A worn slot only takes what the class wears in it. Without this a wizard equips a sword and
    // shoots with it, and fourteen classes quietly become one.
    if !worn_slots_would_accept(context, player, from, to).await {
        snap_back(link, context, player, "that does not go there").await;
        return;
    }

    // What the client believed was there. The store refuses the move if it is no longer, which is
    // what stops the same item being moved twice by two requests that both read it first.
    let expected = match read_slot(&context.store, source).await {
        Some(item) => item,
        None => {
            snap_back(link, context, player, "there is nothing there").await;
            return;
        }
    };

    match context
        .store
        .move_item(source, destination, Some(expected))
        .await
    {
        Ok(_) => {
            send_containers(link, &context.catalog, &context.store, player).await;
            refresh_equipment(context, player, placement).await;
        }
        Err(hendra_store::StoreError::Refused(reason)) => {
            snap_back(link, context, player, reason).await;
        }
        Err(err) => {
            tracing::warn!(%err, "a move failed");
            snap_back(link, context, player, "that could not be done").await;
        }
    }
}

/// Why a move is refused before either end is looked at, or `None` if it may go on.
///
/// `ValidateEntities` and the trade test, which `InvSwapHandler.Handle` runs together before the
/// containers are touched (`InvSwapHandler.cs:44`). The parts of `ValidateEntities` about entities
/// — that both ends are containers, that a player end is the acting player, that a bag's owners
/// admit this account, and that the two ends are within a tile of each other — have no counterpart
/// here: the player's own containers are named by tag rather than by entity, so a client cannot
/// address anybody else's pack, and the bag's owner and distance are checked by the world when the
/// bag itself is reached.
///
/// What is left is the rules about what kind of container each end is, plus the slot numbering
/// `ValidateSlotSwap` tests before it audits anything (`InvSwapHandler.cs:177`).
fn move_refusal(
    from: SlotLocation,
    to: SlotLocation,
    trading: bool,
    reachable: &Reachable,
) -> Option<&'static str> {
    // `player.tradeTarget != null` refuses every swap, wherever it is aimed. An offer names slots
    // rather than items and is read again when both sides agree, so an item shifted out of an
    // offered slot after the other side agreed is a swap of what they agreed to.
    if trading {
        return Some("you cannot do that while trading");
    }

    // A gift chest is a `OneWayContainer`. `ValidateEntities` refuses one paired with anything but
    // the player (`InvSwapHandler.cs:162-163`), so a gift comes out into the pack and nowhere else
    // -- not straight into the vault, which would claim it without ever passing through the hands
    // of whoever it was sent to.
    let gift = |where_| matches!(where_, SlotLocation::Gift { .. });
    let players_own = |where_| {
        matches!(
            where_,
            SlotLocation::Inventory { .. } | SlotLocation::Equipment { .. }
        )
    };
    if gift(from) && !players_own(to) {
        return Some("a gift only comes out into your pack");
    }

    // And nothing goes in. `AuditItem` returns false for any item at all bound for a
    // `OneWayContainer` (`realm/Utils.cs:415-416`), from every source there is.
    if gift(to) {
        return Some("nothing goes into a gift chest");
    }

    // Both slot numbers, whichever container each belongs to: `slotA < 16 && slotB < 16 ||
    // player.HasBackpack` reads the pair rather than only the player's end. A bag's own slots never
    // reach sixteen, so what the test says anything about is the player's -- and it says it about a
    // move out of a bag as much as about one within the pack, which is what stops an item being
    // taken from a bag straight into a slot nobody paid for.
    if !within_bounds(from, reachable) || !within_bounds(to, reachable) {
        return Some("there is no such slot");
    }

    None
}

/// Refuses a move and tells the client what the two slots really hold.
///
/// The client draws a swap the instant it sends it, so the panel does not lag a round trip behind
/// the click (`godot-client/src/World/Inventory.cs:590-604`). A refusal it is not corrected about
/// therefore leaves the item drawn where it never went. `InvSwapHandler` force-updates both slots
/// alongside every `InvResult { Result = 1 }` it sends (`:46-48`, `:83-85`, `:134-135`) for exactly
/// that reason; ours sends the whole container, which says the same about both ends and about
/// anything else that has changed since.
async fn snap_back(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    why: &str,
) {
    deny(link, why).await;
    send_containers(link, &context.catalog, &context.store, player).await;
}

/// How many slots of each kind this player can actually reach.
#[derive(Debug, Clone, Copy)]
struct Reachable {
    /// Carried slots, counted from the first one past the worn four.
    carried: u16,

    /// Vault slots, which is eight for every chest the account owns.
    vault: u16,
}

/// What this player owns, in slots.
async fn reachable_slots(context: &Context, player: &crate::accounts::Session) -> Reachable {
    let last = last_carried_slot(&context.store, player.character.id).await;

    Reachable {
        carried: (last - EQUIPPED_SLOTS as i16 + 1).max(0) as u16,
        vault: (player.account.vault_chests.max(0) as u16).saturating_mul(crate::vault::SLOTS_PER_CHEST as u16),
    }
}

/// Whether a slot the client named is one that exists.
fn within_bounds(where_: SlotLocation, reachable: &Reachable) -> bool {
    match where_ {
        SlotLocation::Equipment { slot } => u16::from(slot) < u16::from(EQUIPPED_SLOTS),
        SlotLocation::Inventory { slot } => u16::from(slot) < reachable.carried,
        SlotLocation::Vault { slot } => slot < reachable.vault,
        SlotLocation::Gift { slot } => slot < hendra_store::GIFT_SLOTS as u16,
        SlotLocation::Bag { .. } | SlotLocation::Ground => true,
    }
}

/// Which moves `InvSwapHandler` allows at all, before either end is read.
#[cfg(test)]
mod which_moves_are_allowed {
    use super::{Reachable, SlotLocation, move_refusal};
    use hendra_net::EntityId;

    /// A character with the eight carried slots every class has and four vault chests.
    fn ordinary() -> Reachable {
        Reachable {
            carried: 8,
            vault: 32,
        }
    }

    /// The same character, having bought the backpack.
    fn with_a_backpack() -> Reachable {
        Reachable {
            carried: 16,
            vault: 32,
        }
    }

    fn bag(slot: u8) -> SlotLocation {
        SlotLocation::Bag {
            entity: EntityId(7),
            slot,
        }
    }

    #[test]
    fn a_trade_in_progress_refuses_every_move() {
        // `InvSwapHandler.cs:44` refuses the whole handler on `player.tradeTarget != null`, before
        // it has looked at what either end is. An offer names slots and is read again when both
        // sides agree, so a move out of an offered slot swaps what the other side agreed to.
        let pairs = [
            (
                SlotLocation::Inventory { slot: 0 },
                SlotLocation::Inventory { slot: 1 },
            ),
            (
                SlotLocation::Inventory { slot: 0 },
                SlotLocation::Vault { slot: 0 },
            ),
            (
                SlotLocation::Vault { slot: 0 },
                SlotLocation::Equipment { slot: 0 },
            ),
            (bag(0), SlotLocation::Inventory { slot: 0 }),
            (SlotLocation::Inventory { slot: 0 }, bag(0)),
            (SlotLocation::Inventory { slot: 0 }, SlotLocation::Ground),
        ];

        for (from, to) in pairs {
            assert_eq!(
                move_refusal(from, to, true, &ordinary()),
                Some("you cannot do that while trading"),
                "{from:?} -> {to:?}"
            );

            // And that the same move is otherwise fine, so the test is about the trade.
            assert_eq!(move_refusal(from, to, false, &ordinary()), None);
        }
    }

    #[test]
    fn a_gift_comes_out_into_the_pack_and_nowhere_else() {
        // The gift chest is a `OneWayContainer`, and `ValidateEntities` refuses one paired with
        // anything but the player (`InvSwapHandler.cs:162-163`).
        for to in [
            SlotLocation::Vault { slot: 0 },
            SlotLocation::Gift { slot: 1 },
            bag(0),
            SlotLocation::Ground,
        ] {
            assert!(
                move_refusal(SlotLocation::Gift { slot: 0 }, to, false, &ordinary()).is_some(),
                "a gift went to {to:?}"
            );
        }

        assert_eq!(
            move_refusal(
                SlotLocation::Gift { slot: 0 },
                SlotLocation::Inventory { slot: 3 },
                false,
                &ordinary()
            ),
            None
        );
        assert_eq!(
            move_refusal(
                SlotLocation::Gift { slot: 0 },
                SlotLocation::Equipment { slot: 0 },
                false,
                &ordinary()
            ),
            None
        );
    }

    #[test]
    fn nothing_goes_into_a_gift_chest_from_anywhere() {
        // `AuditItem` returns false for any item at all bound for a `OneWayContainer`
        // (`realm/Utils.cs:415-416`), whatever the other end is.
        for from in [
            SlotLocation::Inventory { slot: 0 },
            SlotLocation::Equipment { slot: 0 },
            SlotLocation::Vault { slot: 0 },
            bag(0),
        ] {
            assert_eq!(
                move_refusal(from, SlotLocation::Gift { slot: 0 }, false, &ordinary()),
                Some("nothing goes into a gift chest"),
                "{from:?} reached the gift chest"
            );
        }
    }

    #[test]
    fn a_bag_cannot_be_emptied_into_a_slot_nobody_bought() {
        // `ValidateSlotSwap` (`InvSwapHandler.cs:177`) reads both slot numbers whichever container
        // each belongs to, so the backpack is as unreachable from a bag as it is from the pack.
        // Carried slot eight is the first square of the backpack in the durable numbering.
        let backpack = SlotLocation::Inventory { slot: 8 };

        assert_eq!(
            move_refusal(bag(0), backpack, false, &ordinary()),
            Some("there is no such slot")
        );
        assert_eq!(move_refusal(bag(0), backpack, false, &with_a_backpack()), None);

        // And the last square of the pack itself is reachable either way.
        let carried = SlotLocation::Inventory { slot: 7 };
        assert_eq!(move_refusal(bag(0), carried, false, &ordinary()), None);
    }

    #[test]
    fn a_slot_number_no_container_has_is_refused_from_either_end() {
        let nowhere = SlotLocation::Inventory { slot: 200 };
        assert_eq!(
            move_refusal(nowhere, bag(0), false, &with_a_backpack()),
            Some("there is no such slot")
        );
        assert_eq!(
            move_refusal(bag(0), nowhere, false, &with_a_backpack()),
            Some("there is no such slot")
        );

        // A vault slot past the chests the account owns, from a bag, is the same refusal.
        assert_eq!(
            move_refusal(bag(0), SlotLocation::Vault { slot: 32 }, false, &ordinary()),
            Some("there is no such slot")
        );
    }
}

/// Hands the vault panel the whole vault.
///
/// On arrival, after every accepted move, and after a refusal. No counterpart in the original,
/// which has no vault message: its chests are `Container` entities and the client sees their eight
/// slots through the ordinary object updates (`realm/worlds/logic/Vault.cs:96-107`), refreshed on
/// refusal by `ForceUpdate` on each end (`networking/handlers/InvSwapHandler.cs:83-84`, `:134-135`).
async fn send_vault(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    version: u32,
) {
    let update = crate::vault::snapshot(context, player.account.id, version).await;

    let mut buf = Vec::new();
    update.encode(&mut Writer::new(&mut buf));
    let _ = link.send(Delivery::Stream, &buf).await;
}

/// Moves something in the vault panel.
///
/// Only from inside the vault, and a refusal is answered with the whole vault, which both corrects
/// the optimistic move the client already drew and gives it the version to try again with.
///
/// The packet is ours; the shape is the original's. `InvSwapHandler` also refuses and then repairs
/// the client's picture rather than leaving it wrong — `ForceUpdate` on both ends plus
/// `InvResult { Result = 1 }` (`networking/handlers/InvSwapHandler.cs:83-86`, `:134-136`) — and
/// being inside the vault is implied there by the chest being an entity you have to stand next to
/// (`:166-169`).
#[allow(clippy::too_many_arguments)]
async fn vault_move(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    version: &mut u32,
    quoted: u32,
    from: (i16, i16),
    to: (i16, i16),
) {
    // The panel closes when the player walks away from the access object, and a move arriving from
    // anywhere else is either a stale packet or somebody reaching into their storage from a dungeon.
    if placement.world.name.as_ref() != "Vault" {
        deny(link, "you are not at your vault").await;
        send_vault(link, context, player, *version).await;
        return;
    }

    // The panel is the other half of the swap `InvSwapHandler` performs, and `:44` refuses every
    // swap while a trade is open. Without it an item is offered, agreed to, and put in the vault
    // before the offer -- which names slots and is read again when both sides agree -- is settled.
    if context.trades.partner(&player.character.name).is_some() {
        deny(link, "you cannot do that while trading").await;
        send_containers(link, &context.catalog, &context.store, player).await;
        send_vault(link, context, player, *version).await;
        return;
    }

    match crate::vault::try_move(
        context, player, *version, quoted, from.0, from.1, to.0, to.1,
    )
    .await
    {
        Ok(crate::vault::Moved::Applied) => {
            *version = version.wrapping_add(1);

            // Both sides of the move, because either end may have been the player's own pack.
            send_containers(link, &context.catalog, &context.store, player).await;
            refresh_equipment(context, player, placement).await;
            send_vault(link, context, player, *version).await;
        }

        // Nothing happened, so the version stands. The client is still told, because it drew the
        // move before asking.
        Ok(crate::vault::Moved::Nothing) => {
            send_vault(link, context, player, *version).await;
        }

        Err(refusal) => {
            deny(link, refusal).await;
            send_containers(link, &context.catalog, &context.store, player).await;
            send_vault(link, context, player, *version).await;
        }
    }
}

/// Buys one more vault chest.
///
/// Only inside the vault, then the purchase, then what happened, then the vault as it now is.
///
/// The packet is ours, but the answer is the original's word for word: `ClosedVaultChest.Buy` sends
/// `BuyResult { Result = 0, ResultString = "Vault chest purchased!" }`
/// (`git show 94615c4:Server-Side/wServer/realm/entities/vendors/ClosedVaultChest.cs`, `:47-51`),
/// and being inside the vault is implied there by the thing you buy being an entity standing on the
/// vault floor.
async fn vault_buy(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    version: &mut u32,
    believed: u32,
) {
    if placement.world.name.as_ref() != "Vault" {
        deny(link, "you are not at your vault").await;
        return;
    }

    match crate::vault::try_buy(context, player, believed).await {
        Ok(()) => {
            *version = version.wrapping_add(1);

            // The fame the chest cost has left the account, and the number over the player's head
            // has to say so or the next purchase is priced against a figure that is no longer true.
            // `ClosedVaultChest.Buy` does the same thing in the same place -- `acc.Reload` and then
            // `player.CurrentFame = acc.Fame` before it answers
            // (`git show 94615c4:Server-Side/wServer/realm/entities/vendors/ClosedVaultChest.cs`,
            // `:43-44`).
            resend_purse(context, player, placement).await;

            say(link, "Vault chest purchased!").await;
        }
        Err(refusal) => deny(link, refusal).await,
    }

    send_vault(link, context, player, *version).await;
}

/// Puts the object the vault panel opens from into the room the player has just entered.
async fn place_chests(context: &Context, player: &crate::accounts::Session, placement: &Placement) {
    let (reply, answer) = tokio::sync::oneshot::channel();
    if placement
        .world
        .send(ToWorld::PlaceVaultAccess { reply })
        .await
    {
        let placed = answer.await.unwrap_or(0);
        tracing::debug!(placed, account = player.account.id, "placed the vault access");
    }

    // A gift chest stands in the same room wherever a map marks one, which the vault's own no
    // longer does: gifts arrive in the panel with everything else, so there is nothing to place
    // there and nothing marked to place it on. The original deals `Account.Gifts` eight at a time
    // into `GiftChest` entities on the `Gifting_Chest` tiles, four of them in the 2020 `Vault.jm`
    // (pristine `realm/worlds/logic/Vault.cs:115-130`). Kept for the
    // maps that do mark a gifting square, where it is still the only way to open one.
    let gifts: Vec<(u16, u16)> = context
        .store
        .gifts(player.account.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(slot, item)| Some((slot as u16, number(&context.catalog, item)?)))
        .collect();

    let (reply, answer) = tokio::sync::oneshot::channel();
    if placement
        .world
        .send(ToWorld::PlaceGiftChest {
            slots: gifts,
            reply,
        })
        .await
    {
        let placed = answer.await.unwrap_or(0);
        tracing::debug!(placed, account = player.account.id, "placed the gift chest");
    }
}

/// Claims one of an account's gifts into the character's pack.
async fn claim_gift(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    slot: u8,
    identity: uuid::Uuid,
) {
    // Conditional on the gift still being in that slot, which is what makes two requests for the
    // same gift resolve to one item rather than two.
    if let Err(err) = context
        .store
        .take_gift(player.account.id, slot as i16, identity)
        .await
    {
        snap_back(link, context, player, &err.to_string()).await;
        return;
    }

    let last = last_carried_slot(&context.store, player.character.id).await;
    let given = context
        .store
        .give_item(player.character.id, identity, EQUIPPED_SLOTS as i16, last)
        .await;

    if given.is_err() {
        // The gift is already off the account, so handing it back is the only honest answer: an item
        // that fell between two writes is the failure this ordering exists to avoid.
        let _ = context.store.add_gift(player.account.id, identity).await;
        deny(link, "you have nowhere to put that").await;
    }

    send_containers(link, &context.catalog, &context.store, player).await;
    place_chests(context, player, placement).await;
}

/// Takes an item out of a bag and into the player's inventory.
///
/// The bag gives it up first. If the durable write then fails the item goes back, and if the server
/// dies in between it is lost, which is the trade this order buys and the right way round.
async fn take_from_bag(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    bag: EntityId,
    slot: u8,
    destination: SlotLocation,
) {
    let (reply, answer) = tokio::sync::oneshot::channel();
    if !placement
        .world
        .send(ToWorld::TakeFromBag {
            player: placement.handle,
            bag,
            slot,
            reply,
        })
        .await
    {
        return;
    }

    let Some(taken) = answer.await.ok().flatten() else {
        snap_back(link, context, player, "there is nothing there to take").await;
        return;
    };
    let item = taken.item;

    // The world names items by runtime number; the durable side names them by identity. This is
    // the seam, and an item the catalog cannot name is one that must not be written down.
    let Some(identity) = context
        .catalog
        .object(ObjectType(item))
        .map(|desc| desc.uuid)
    else {
        snap_back(link, context, player, "that is not something you can carry").await;
        return;
    };

    // A gift is claimed rather than picked up. The chest standing in the vault is a drawing of the
    // account's rows and still holds what it held, so the row goes first and the item is only handed
    // over once it has: a failure that way round leaves the gift unclaimed, and the other way round
    // leaves the player holding a gift they can claim again. The original orders the two the other
    // way — the swap out of the `GiftChest` entity commits first and `RemoveGift` follows
    // (`networking/handlers/InvSwapHandler.cs:111-127`) — which is safe there only because the item
    // has already left the in-memory chest. Without that chest, this order is the only safe one.
    if taken.kind == hendra_sim::ContainerKind::Merchant {
        claim_gift(link, context, player, placement, slot, identity).await;
        return;
    }

    // A stacking potion goes to its own place rather than into the pack, which is what gives it a
    // ceiling of six and keeps it out of the eight slots everything else competes for.
    if let Some(magic) = stacks_as(&context.catalog, ObjectType(item)) {
        return match context.store.add_potion(player.character.id, magic).await {
            Ok(true) => {
                send_containers(link, &context.catalog, &context.store, player).await;
            }
            Ok(false) => snap_back(link, context, player, "you cannot carry any more of those").await,
            Err(_) => snap_back(link, context, player, "try again shortly").await,
        };
    }

    let outcome = match locate(destination, player.character.id, player.account.id) {
        // A worn slot only takes what the class wears in it. Checked here as well as in `move_item`
        // because a bag is the one route into a worn slot that does not pass through it: without
        // this a wizard takes a sword straight out of a bag and shoots with it.
        Some(Location::Inventory { slot, .. })
            if slot < EQUIPPED_SLOTS as i16
                && !worn_slot_accepts(context, player, slot, ObjectType(item)) =>
        {
            Err(hendra_store::StoreError::Refused("that does not go there"))
        }

        // A named durable slot: it has to be free, because there is nothing to swap with.
        Some(Location::Inventory { character_id, slot }) => context
            .store
            .give_item(character_id, identity, slot, slot)
            .await
            .map(|_| ()),

        // A vault slot, or a destination the wire cannot name durably: the first free carried slot
        // instead. A gift chest never reaches this, `move_refusal` having already refused anything
        // aimed at one.
        Some(Location::Vault { .. }) | Some(Location::Gift { .. }) | None => {
            // Anywhere else, or nowhere in particular: the first free carried slot.
            context
                .store
                .give_item(
                    player.character.id,
                    identity,
                    EQUIPPED_SLOTS as i16,
                    last_carried_slot(&context.store, player.character.id).await,
                )
                .await
                .map(|_| ())
        }
    };

    if let Err(err) = outcome {
        // Undo the first step. The bag may have gone if it emptied, in which case this makes a new
        // one where the player stands. The item comes back either way.
        let (reply, _) = tokio::sync::oneshot::channel();
        let _ = placement
            .world
            .send(ToWorld::PutInBag {
                player: placement.handle,
                bag: Some(bag),
                item,

                // Going back where it came from, which is a bag that already exists and already
                // belongs to whoever it belongs to, so no diversion is wanted.
                soulbound: false,

                reply,
            })
            .await;

        tracing::debug!(%err, "returned an item to its bag");
        say(
            link,
            match err {
                hendra_store::StoreError::Refused(why) => why,
                _ => "there is no room for that",
            },
        )
        .await;

        // The bag corrects itself in the next snapshot, being a thing in the world; the pack does
        // not, and the client has already drawn the item arriving in it.
        send_containers(link, &context.catalog, &context.store, player).await;
        refresh_equipment(context, player, placement).await;
        return;
    }

    send_containers(link, &context.catalog, &context.store, player).await;

    // A bag is the one route into a worn slot that does not go through `move_item`, so the
    // recalculation the original gets for free from `Inventory.InventoryChanged`
    // (`Player.cs:463`) has to be asked for here. Without it a weapon taken straight out of a bag
    // is drawn in the slot and shoots nothing of its own.
    refresh_equipment(context, player, placement).await;
}

/// Puts an item from the player's inventory into a bag.
async fn put_in_bag(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    source: SlotLocation,
    bag: Option<EntityId>,
) {
    // Nothing is dropped in the nexus. `InvDropHandler.cs:24` returns on the world's name before it
    // looks at anything else, which is what keeps the town floor clear of other people's rubbish.
    if bag.is_none() && placement.world.name.as_ref() == "Nexus" {
        snap_back(link, context, player, "you cannot drop things here").await;
        return;
    }

    // Nor while trading. An item offered in a trade and dropped in the same breath is the shape of
    // every duplication bug, so `InvDropHandler.cs:27` refuses the whole handler mid-trade and
    // `InvSwapHandler.cs:44` refuses a swap the same way.
    if context.trades.partner(&player.character.name).is_some() {
        snap_back(link, context, player, "you cannot do that while trading").await;
        return;
    }

    let Some(location) = locate(source, player.character.id, player.account.id) else {
        snap_back(link, context, player, "that cannot be dropped").await;
        return;
    };

    let Location::Inventory { character_id, slot } = location else {
        snap_back(
            link,
            context,
            player,
            "things cannot be dropped straight from the vault",
        )
        .await;
        return;
    };

    let Some(item) = read_slot(&context.store, location).await else {
        snap_back(link, context, player, "there is nothing there").await;
        return;
    };

    // The durable side gives it up first here too, for the same reason in reverse. Otherwise an
    // item exists in a bag and in the database at once.
    if let Err(err) = context.store.take_item(character_id, slot, item).await {
        snap_back(link, context, player, &err.to_string()).await;
        return;
    }

    let (reply, answer) = tokio::sync::oneshot::channel();
    let sent = placement
        .world
        .send(ToWorld::PutInBag {
            player: placement.handle,
            bag,
            item: number(&context.catalog, item).unwrap_or(0),

            // A soulbound item goes into a bag only whoever dropped it may open, which is what makes
            // dropping one a way to move it rather than a way to give it away. Told to the world for
            // a named bag as well as for a new one, because a soulbound item aimed at somebody
            // else's bag is diverted into one of the player's own rather than refused.
            soulbound: is_soulbound(&context.catalog, item),

            reply,
        })
        .await;

    let accepted = sent && answer.await.unwrap_or(false);
    if !accepted {
        // Put it back where it came from.
        let _ = context
            .store
            .give_item(character_id, item, slot, slot)
            .await;
        deny(link, "there is nowhere to put that").await;
    }

    send_containers(link, &context.catalog, &context.store, player).await;

    // The other direction out of a worn slot. Dropping the weapon leaves the slot empty, and a body
    // that was not told keeps shooting the thing it is no longer holding.
    refresh_equipment(context, player, placement).await;
}

/// The two potions that stack rather than taking a slot.
///
/// `HealthPots` and `MagicPots` in the original, which gives each its own place outside the pack and
/// a ceiling of six. Named rather than numbered, since a runtime number means nothing once the
/// content is reordered.
pub const HEALTH_POTION: &str = "Health Potion";
pub const MAGIC_POTION: &str = "Magic Potion";

/// Where the two stacks are addressed from.
///
/// The original's slots 254 and 255, kept because they are already what a client names when it
/// drinks from a stack: they are outside the eight the pack has, which is what makes them a place
/// of their own rather than two more slots to compete for.
pub const HEALTH_STACK_SLOT: u16 = 254;
pub const MAGIC_STACK_SLOT: u16 = 255;

/// Which stack an item belongs in, if either.
pub(crate) fn stacks_as(catalog: &hendra_content::Catalog, item: ObjectType) -> Option<bool> {
    let id = catalog.object(item)?.id.as_str();

    match id {
        HEALTH_POTION => Some(false),
        MAGIC_POTION => Some(true),
        _ => None,
    }
}

/// Whether a worn slot takes this item, for the routes that do not go through `move_item`.
///
/// A class whose description is missing accepts anything, for the same reason `move_item` allows
/// the move: refusing everything a character does is a worse answer than allowing it.
pub(crate) fn worn_slot_accepts(
    context: &Context,
    player: &crate::accounts::Session,
    slot: i16,
    item: ObjectType,
) -> bool {
    let Some(class) = context
        .catalog
        .type_of_uuid(player.character.class)
        .and_then(|found| context.catalog.class(found))
    else {
        return true;
    };

    hendra_characters::slot_accepts(&context.catalog, class, slot, item)
}

/// Whether using this item would give a backpack.
fn grants_a_backpack(catalog: &hendra_content::Catalog, kind: ObjectType) -> bool {
    use hendra_content::activate::Unlock;

    catalog
        .object(kind)
        .and_then(|object| object.item.as_ref())
        .is_some_and(|item| {
            item.activate.iter().any(|activate| {
                matches!(
                    hendra_content::Effect::of(activate),
                    hendra_content::Effect::Unlock {
                        kind: Unlock::Backpack,
                        ..
                    }
                )
            })
        })
}

/// The highest carried slot a player has without a backpack.
///
/// Four worn and eight carried, which is what a character starts with.
const LAST_CARRIED_SLOT: i16 = 11;

/// The highest carried slot a backpack buys, which is eight more.
const LAST_BACKPACK_SLOT: i16 = 19;

/// The last slot this character can put something in.
///
/// Asked of the store rather than of the session, because a backpack is used mid-session and the
/// slots it buys should be usable straight away rather than after logging out. A character whose
/// row cannot be read is treated as having no backpack: refusing to place an item is recoverable,
/// and putting one in a slot that does not exist is not.
pub(crate) async fn last_carried_slot(store: &Store, character_id: i64) -> i16 {
    match store.has_backpack(character_id).await {
        Ok(true) => LAST_BACKPACK_SLOT,
        _ => LAST_CARRIED_SLOT,
    }
}

/// What is currently in a durable slot.
pub(crate) async fn read_slot(store: &Store, at: Location) -> Option<uuid::Uuid> {
    match at {
        Location::Inventory { character_id, slot } => store
            .character(character_id)
            .await
            .ok()?
            .inventory
            .iter()
            .find(|(index, _)| *index == slot)
            .map(|(_, item)| *item),
        Location::Vault { account_id, slot } => store
            .vault(account_id)
            .await
            .ok()?
            .iter()
            .find(|(index, _)| *index == slot)
            .map(|(_, item)| *item),
        Location::Gift { account_id, slot } => store
            .gifts(account_id)
            .await
            .ok()?
            .iter()
            .find(|(index, _)| *index == slot)
            .map(|(_, item)| *item),
    }
}

/// Tells the player something they asked for.
///
/// `SendInfo` (`Player.Chat.cs:110-119`), which the client draws in the yellow it keeps for the
/// server's own voice (`ChatListItemFactory.as:282-284`).
async fn say(link: &mut Link, message: &str) {
    let mut buf = Vec::new();
    ServerMessage::Refused { message }.encode(&mut Writer::new(&mut buf));
    let _ = link.send(Delivery::Stream, &buf).await;
}

/// Sends a word the client acts on rather than reads.
///
/// `GlobalNotification`, which the original follows every market withdrawal
/// (`networking/handlers/MarketCommandHandler.cs:79-82`) and every gifted purchase with
/// (`realm/entities/vendors/Merchant.cs:204-208`): the item went to the gift chest rather than to
/// the pack, and the chest is somewhere else entirely.
async fn notify(link: &mut Link, text: &str) {
    let mut buf = Vec::new();
    ServerMessage::Notification {
        text: text.to_string(),
    }
    .encode(&mut Writer::new(&mut buf));
    let _ = link.send(Delivery::Stream, &buf).await;
}

/// What the original says when something has landed in the gift chest.
const GIFT_CHEST_OCCUPIED: &str = "giftChestOccupied";

/// The name the original speaks a refusal under.
///
/// `SendError` (`Player.Chat.cs:132-141`) differs from `SendInfo` (`:110-119`) in exactly one
/// field, and this is it. There is no error flag and no colour anywhere on the original's wire
/// either: the sender name is the whole mechanism, and the client turns it into red by string
/// equality (`ChatListItemFactory.as:291-293`).
const ERROR_SPEAKER: &str = "*Error*";

/// Refuses something, in the red the client keeps for a refusal.
///
/// Sent as chat rather than as a [`ServerMessage::Refused`] because the sender name is the only
/// thing that separates a refusal from a report, and `Refused` carries no name to put it in. The
/// speaker is nobody, so nothing is drawn over anybody's head — as with every starred line in the
/// original, whose `ObjectId` is left at a value no entity ever has.
async fn deny(link: &mut Link, message: &str) {
    let mut buf = Vec::new();
    ServerMessage::Chat {
        speaker: hendra_net::EntityId(0),
        from: ERROR_SPEAKER,
        text: message,
    }
    .encode(&mut Writer::new(&mut buf));
    let _ = link.send(Delivery::Stream, &buf).await;
}

/// Puts the player into a world.
///
/// The welcome itself is written by the world task, in the same step that puts the body in the
/// world and before it -- see the `ToWorld::Join` handler. Sending it from here instead left a
/// window between the body becoming eligible for the tick's terrain pass and the welcome reaching
/// the queue, and a client that is sent terrain before its welcome throws that terrain away.
async fn join(
    link: &mut Link,
    world: &WorldHandle,
    name: &str,
    arrival: crate::world_task::Arrival,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<Handle> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    if !world
        .send(ToWorld::Join {
            name: name.to_string(),
            arrival,
            sender: link.sender(),
            music: playing_in(world),
            orders: orders.clone(),
            died: died.clone(),
            reply,
        })
        .await
    {
        refuse(link, RejectReason::Full).await;
        return None;
    }

    answer.await.ok()
}

/// What a world is playing at this moment.
///
/// Read rather than taken from the definition, because `/music` and the `ChangeMusic` behaviours
/// write `World.Music` while people are standing in the room: somebody arriving afterwards is told
/// the track that is playing, not the one the definition named.
fn playing_in(world: &WorldHandle) -> String {
    world
        .music
        .read()
        .map(|music| music.clone())
        .unwrap_or_default()
}

/// Whether both ends of a move would leave every worn slot holding something it accepts.
///
/// Both ends, because a move is a swap: dragging a sword onto a wand puts the wand where the sword
/// was, and checking only the destination would let the second half through.
async fn worn_slots_would_accept(
    context: &Context,
    player: &crate::accounts::Session,
    from: SlotLocation,
    to: SlotLocation,
) -> bool {
    let Some(class) = context
        .catalog
        .type_of_uuid(player.character.class)
        .and_then(|found| context.catalog.class(found))
    else {
        // A character whose class is not in the catalog cannot have its slots judged, and refusing
        // every move it makes would be a worse answer than allowing them.
        return true;
    };

    // Judged on the durable slot, not the one the client named. A wire `Inventory` slot is a
    // carried one and sits four above the worn four in the same table, so asking whether wire slot
    // nought is worn answers about the first backpack slot -- and the answer that comes back is
    // about the weapon slot, which is why a robe would not go into the top-left square of the pack.
    let character_id = player.character.id;
    let account_id = player.account.id;
    let (Some(source), Some(destination)) = (
        locate(from, character_id, account_id),
        locate(to, character_id, account_id),
    ) else {
        return true;
    };

    let worn = |location: Location| {
        matches!(location, Location::Inventory { slot, .. } if slot < hendra_characters::EQUIPPED_SLOTS)
    };

    if !worn(source) && !worn(destination) {
        return true;
    }

    let moving = read_slot(&context.store, source).await;
    let displaced = read_slot(&context.store, destination).await;

    // An empty slot accepts anything, which is what taking an item out of one means.
    let fits = |location: Location, item: Option<uuid::Uuid>| match location {
        Location::Inventory { slot, .. } => {
            let kind = item
                .and_then(|item| context.catalog.type_of_uuid(item))
                .unwrap_or(ObjectType::NONE);
            hendra_characters::slot_accepts(&context.catalog, class, slot, kind)
        }
        _ => true,
    };

    fits(destination, moving) && fits(source, displaced)
}

/// How much of a character's progress one write covers.
///
/// The original draws the same line. Its three-second checkpoint flushes the character alone
/// (`wServer/realm/entities/player/Player.KeepAlive.cs:129-133`, which calls `SaveToCharacter()`
/// and then `_client.Character.FlushAsync()`), while a logout goes through
/// `Database.SaveCharacter` and writes the class record alongside it
/// (`wServer/networking/Client.cs:230`, `common/Database.cs:1058-1069`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Written {
    /// The character row and what it has done. What a checkpoint writes.
    CharacterOnly,

    /// The same, plus what the account has now taken this class to.
    WithClass,
}

/// Asks a world what a character currently is, taking its tally with the answer.
///
/// The world is asked rather than the session remembering, because the world is where levelling,
/// damage and equipment happen and a remembered copy would be one tick stale at best.
///
/// Answers `None` for a world that has already dropped the player, which is not an error: it
/// happens whenever a world closes underneath a session.
async fn snapshot(placement: &Placement) -> Option<crate::world_task::Vitals> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    placement
        .world
        .send(ToWorld::Snapshot {
            handle: placement.handle,
            reply,
        })
        .await;

    answer.await.ok().flatten()
}

/// Writes back what a character became, including what it learned.
async fn save_progress(
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    written: Written,
) -> Result<(), hendra_store::StoreError> {
    let Some(vitals) = snapshot(placement).await else {
        return Ok(());
    };

    save_vitals(context, player, &vitals, written).await
}

/// Writes one already-taken snapshot to the store.
///
/// Separate from taking it so a caller that needs the figures for something else — a world change,
/// which builds the next body out of them — writes exactly what it carries across rather than
/// asking twice and getting two different answers.
async fn save_vitals(
    context: &Context,
    player: &crate::accounts::Session,
    vitals: &crate::world_task::Vitals,
    written: Written,
) -> Result<(), hendra_store::StoreError> {
    // Every write below is conditional on this, so a snapshot taken before the account changed
    // hands lands nowhere once it has. `Client.Save` passes the same token to
    // `Database.SaveCharacter` (`networking/Client.cs:233`, `common/Database.cs:1058-1069`).
    let under = player.lock.as_ref();

    // Held rather than returned on, because taking the snapshot moved the tally off the body: it
    // exists nowhere else now, and leaving before the counts are written would throw away every
    // shot and kill since the last checkpoint over a failure that has nothing to do with them. The
    // two are separate statements against separate columns and neither needs the other to succeed.
    let written_character = context
        .store
        .save_character(
            player.character.id,
            &hendra_store::Saved {
                hp: vitals.hp,
                mp: vitals.mp,
                max_hp: vitals.max_hp,
                max_mp: vitals.max_mp,
                level: vitals.level,
                experience: vitals.experience,
                fame: vitals.fame,
                stats: vitals.stats,
            },
            under,
        )
        .await;

    // A refusal is not a failure: it is this session no longer being the one playing the account.
    // Everything after it would land on a row somebody else is now writing, so nothing after it
    // runs — the original's whole save is one conditional transaction, and a failed condition
    // drops the character and the class record together.
    if matches!(written_character, Ok(false)) {
        tracing::info!(
            character = player.character.id,
            account = player.account.id,
            lost = ?vitals.tally,
            "a save was refused: this session no longer holds the account"
        );
        return Ok(());
    }

    // What the character did since the last write, added to what it had done before. Added rather
    // than set, because the snapshot handed over only the counts since the last checkpoint.
    //
    // The counts themselves go into the log when they cannot be written, so a lost tally is a line
    // somebody can read rather than a number that quietly never grew.
    if let Err(err) = context
        .store
        .add_tally(player.character.id, &vitals.tally, under)
        .await
    {
        tracing::warn!(
            %err,
            character = player.character.id,
            lost = ?vitals.tally,
            "could not save what a character did"
        );
    }

    written_character?;

    if written == Written::CharacterOnly {
        return Ok(());
    }

    // What the account has now taken this class to, which is what opens the next one. Recorded
    // here rather than only at death, because an account whose best warrior is still alive has
    // still levelled a warrior.
    hendra_characters::record_progress(
        &context.store,
        player.account.id,
        &hendra_store::Character {
            level: vitals.level,
            fame: vitals.fame,
            ..player.character.clone()
        },
    )
    .await
}

/// Carries out a moderation command, or explains why not.
///
/// The rank is read fresh rather than taken from the session, so raising or lowering somebody takes
/// effect without waiting for them to reconnect, exactly as a mute does.
/// Carries out what a player typed.
///
/// Reading what they meant is `crate::commands::read`, which needs no world, store or connection.
/// This is the half that does need them, and every one of these goes through the same machinery the
/// protocol does: `/tp` is the world's teleport with all its refusals, `/trade` is the trade
/// registry, `/ignore` is the same list the whisper path reads. A command that reached past those
/// would be a second, weaker door into the same room.
#[allow(clippy::too_many_arguments)]
async fn run_command(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    name: &str,
    placement: &mut Placement,
    to_session: &mpsc::Sender<crate::world_task::Order>,
    to_death: &mpsc::Sender<crate::world_task::Departed>,
    command: &str,
    rest: &str,
) {
    use crate::commands::{Action, GuildAction, MarketAction};
    use hendra_store::Admin;

    let rank = match context.store.account(player.account.id).await {
        Ok(account) => Admin::from_number(account.admin_rank),
        Err(_) => Admin::NONE,
    };

    // Every command is logged before it runs, whether or not it is allowed to, in the format
    // `CommandManager.Execute` logs: the world it was typed in, who typed it, and the whole line.
    tracing::info!(
        "[Command] [{}] <{}> /{command}{}{rest}",
        placement.world.name,
        name,
        if rest.is_empty() { "" } else { " " }
    );

    // A name nothing answers to and a rank too low are two different answers, and both are the
    // original's: it names neither the command nor the rank it wanted.
    let Some(known) = crate::commands::find(command) else {
        return deny(link, crate::commands::UNKNOWN).await;
    };
    if !known.allows(rank) {
        return deny(link, crate::commands::NO_PERMISSION).await;
    }

    let action = match crate::commands::read(command, rest) {
        Ok(action) => action,
        // A refusal with more than one line in it is sent as more than one line, since the original
        // sends each of them as its own message.
        Err(why) => {
            for line in why.split('\n') {
                deny(link, line).await;
            }
            return;
        }
    };

    match action {
        // `/l`, which is the one command that repeats what was typed to everybody, and so is the
        // one that does its own mute check rather than skipping the one commands skip.
        Action::Say(text) => {
            if muted(context, player).await {
                return deny(link, "Muted. You can not local chat at this time.").await;
            }

            placement
                .world
                .send(ToWorld::Chat {
                    handle: placement.handle,
                    text,
                })
                .await;
        }

        Action::Tell { to, text } => {
            tell(link, context, player, placement, &to, &text).await;
        }

        Action::GuildSay(text) => guild_say(link, context, player, &text).await,

        // The guild hall is one room per guild rather than one per player, and which map it loads
        // depends on what the guild has paid for, so it does not go through the ordinary door.
        Action::GoTo(world) if world == crate::commands::GUILD_HALL => {
            if let Some(next) =
                enter_guild_hall(link, context, player, placement, name, to_session, to_death).await
            {
                *placement = next;
                context.trades.moved(name, &placement.world.key);
            }
        }

        Action::GoTo(world) => {
            if let Some(next) = go_to(
                link, context, player, placement, name, world, to_session, to_death,
            )
            .await
            {
                *placement = next;
                context.trades.moved(name, &placement.world.key);

                // Arriving in the vault means arriving at your own chests, whether the door was a
                // portal or the command that stands in for one. Without this `/vault` opens an
                // empty room and the portal does not.
                if placement.world.name.as_ref() == "Vault" {
                    place_chests(context, player, placement).await;
                }
            } else {
                deny(link, "you cannot go there from here").await;
            }
        }

        Action::TeleportTo(to) => teleport(link, context, player, placement, &to).await,

        Action::Trade(with) => {
            trade(link, context, player, name, Trade::Request(with)).await;
        }

        Action::List {
            kind,
            name: who,
            add,
        } => {
            list_person(link, context, player, kind, &who, add).await;
        }

        Action::Guild(what) => {
            let asked = match what {
                // Two halves of one exchange rather than two spellings of one command. `/invite`
                // names a player and offers; `/join` names a guild and accepts an offer already
                // made (`GuildInviteHandler.cs:46-52`, `JoinGuildHandler.cs:22-40`).
                GuildAction::Invite(name) => GuildAsk::Invite(name),
                GuildAction::Join(name) => GuildAsk::Join(name),
                GuildAction::Kick(name) => GuildAsk::Remove(name),
                GuildAction::Rank(name, rank) => GuildAsk::SetRank(name, rank.clamp(0, 40) as u8),
                GuildAction::Who => return guild_who(link, context, player).await,
            };

            guild(link, context, player, name, asked).await;
        }

        Action::Market(what) => match what {
            // One-based on the way in, as the original counts the sixteen slots of a pack, and the
            // four worn ones are added on the way through.
            MarketAction::Sell { slot, price } => {
                market(
                    link,
                    context,
                    player,
                    placement,
                    hendra_net::MarketCommand::List {
                        slot: (slot - 1) as u8,
                        price,
                    },
                )
                .await;
            }

            MarketAction::SellAll { item, price } => {
                market_all(link, context, player, placement, &item, price).await;
            }

            MarketAction::Mine => my_market(link, context, player).await,

            MarketAction::Remove(listing) => {
                market(
                    link,
                    context,
                    player,
                    placement,
                    hendra_net::MarketCommand::Cancel {
                        listing: listing as u64,
                    },
                )
                .await;
            }

            // Taking back the last thing listed, which is what somebody who mistyped a price wants
            // and cannot express any other way: they do not know the number.
            // `RemoveItemFromMarketAsync(player.Client.Account.LastMarketId)` and nothing else.
            // The id is handed over unexamined, so an account that has never listed anything asks
            // about listing zero and is answered by the withdrawal itself.
            MarketAction::Oops => {
                let listing = context
                    .store
                    .last_market_id(player.account.id)
                    .await
                    .unwrap_or(0);

                market(
                    link,
                    context,
                    player,
                    placement,
                    hendra_net::MarketCommand::Cancel {
                        listing: listing as u64,
                    },
                )
                .await;
            }
        },

        Action::Report(what) => report(link, context, player, placement, what).await,

        Action::Dungeon(what) => match what {
            crate::commands::DungeonAction::Accept(id) => {
                if let Some(next) = dungeon_accept(
                    link, context, player, placement, name, id, to_session, to_death,
                )
                .await
                {
                    *placement = next;
                    context.trades.moved(name, &placement.world.key);
                }
            }
            crate::commands::DungeonAction::Invite(who) => {
                dungeon_invite(link, context, player, placement, name, &who).await;
            }
        },

        Action::Wield { what, rest } => {
            wield(link, context, player, placement, what, &rest).await;
        }

        Action::Moderate { what, name, rest } => {
            moderate(link, context, what, &name, &rest).await;
        }
    }
}

/// Carries out what a player asked for about a trade.
///
/// The registry decides who may trade with whom and what was agreed to. The store decides whether
/// the items actually move, and its refusal is an answer rather than a failure: an offer can name an
/// item that has been dropped, sold or traded elsewhere since it was made.
async fn trade(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    name: &str,
    asked: Trade,
) {
    use crate::trades::Step;

    let step = match asked {
        Trade::Request(to) => {
            // Somebody who has ignored you is not asked, and is not told they were asked.
            // `Player.Trade.cs:70` returns without a word, so the player who asked is left with
            // nothing rather than with the news that they have been ignored.
            if ignored_by(context, &to, player.account.id).await {
                return;
            }

            let step = context.trades.request(name, &to);

            // A trade that has just begun needs both sides shown what the other is holding, which
            // is a read of two inventories rather than anything the registry knows.
            if step == Step::Done
                && let Some(partner) = context.trades.partner(name)
            {
                open_trade(context, name, &partner).await;
            }
            step
        }

        Trade::Change(offer) => {
            let (offer, soulbound) = tradeable_only(context, player.character.id, &offer).await;
            let step = context.trades.change(name, &offer);

            // Said after the offer has been corrected and the partner told, which is the order
            // `ChangeTradeHandler.cs:42-54` uses: the offer that goes out is the stripped one and
            // the complaint is an afterthought to the player who sent it.
            if soulbound {
                deny(link, "You can't trade Soulbound items.").await;
            }
            step
        }
        Trade::Accept { mine, theirs } => context.trades.accept(name, &mine, &theirs),
        Trade::Cancel => context.trades.cancel(name, "Trade cancelled."),
    };

    match step {
        Step::Done => {}
        Step::Say(reason) => deny(link, &reason).await,

        Step::Settle {
            partner_character,
            partner_name,
            mine,
            theirs,
        } => {
            settle_trade(
                context,
                name,
                player.character.id,
                &partner_name,
                partner_character,
                mine,
                theirs,
            )
            .await;
        }
    }
}

/// Whether a named player has put this account on their ignore list.
async fn ignored_by(context: &Context, name: &str, account_id: i64) -> bool {
    let Ok(target) = context.store.account_by_name(name).await else {
        return false;
    };

    context
        .store
        .is_listed(target.id, account_id, hendra_store::ListKind::Ignored)
        .await
        .unwrap_or(false)
}

/// An offer with every slot holding a soulbound item turned off, and whether any was.
///
/// A soulbound item belongs to whoever found it, and the client greys it out in the trade window --
/// but the window is the client's and the offer is a bitmask, so a client that does not grey it out
/// would otherwise be trading it. `ChangeTradeHandler.cs:30-40` clears the bit on the server for
/// exactly that reason.
async fn tradeable_only(
    context: &Context,
    character_id: i64,
    offer: &[bool],
) -> (Vec<bool>, bool) {
    let mut offer = offer.to_vec();

    let Ok(character) = context.store.character(character_id).await else {
        // Nothing to check it against. Refusing the whole offer is the safe direction to fail in,
        // since the alternative is letting an unchecked one stand.
        return (vec![false; offer.len()], false);
    };

    let mut stripped = false;
    for (slot, included) in offer.iter_mut().enumerate() {
        if !*included {
            continue;
        }

        let soulbound = character
            .inventory
            .iter()
            .find(|(at, _)| usize::try_from(*at) == Ok(slot))
            .is_some_and(|(_, item)| is_soulbound(&context.catalog, *item));

        if soulbound {
            *included = false;
            stripped = true;
        }
    }

    (offer, stripped)
}

/// Tells both sides what the other is holding, which is what a trade window shows.
async fn open_trade(context: &Context, name: &str, partner: &str) {
    let Some(first) = trade_slots(context, name).await else {
        context.trades.cancel(name, "Trade cancelled.");
        return;
    };
    let Some(second) = trade_slots(context, partner).await else {
        context.trades.cancel(name, "Trade cancelled.");
        return;
    };

    context.trades.send(
        name,
        &ServerMessage::TradeStart {
            mine: first.clone(),
            their_name: partner.to_string(),
            theirs: second.clone(),
        },
    );
    context.trades.send(
        partner,
        &ServerMessage::TradeStart {
            mine: second,
            their_name: name.to_string(),
            theirs: first,
        },
    );
}

/// What one player is holding, as the other side of a trade sees it.
///
/// Read by name rather than passed in, because the other side's inventory belongs to another
/// session and this is the only place that can ask for it.
///
/// The name reaches the character through the roster rather than through the database, since a
/// name belongs to an account and every character on it answers to the same one. The roster knows
/// which of them is being played.
async fn trade_slots(context: &Context, name: &str) -> Option<Vec<hendra_net::TradeSlot>> {
    let playing = context.trades.character_of(name)?;
    let character = context.store.character(playing).await.ok()?;

    let mut slots = vec![
        hendra_net::TradeSlot {
            item: None,
            slot_type: 0,
            included: false,
            tradeable: false,
        };
        crate::trades::TRADE_SLOTS
    ];

    for (slot, item) in &character.inventory {
        let (slot, item) = (*slot, *item);
        let Ok(index) = usize::try_from(slot) else {
            continue;
        };
        if index >= slots.len() {
            continue;
        }

        let desc = context
            .catalog
            .type_of_uuid(item)
            .and_then(|kind| context.catalog.object(kind));

        slots[index] = hendra_net::TradeSlot {
            item: desc.map(|desc| desc.object_type.0),
            slot_type: desc
                .and_then(|desc| desc.item.as_ref())
                .map(|item| item.slot_type)
                .unwrap_or(0),
            included: false,
            // Worn slots are not somewhere a trade may take from, and a soulbound item is one the
            // content says belongs to whoever found it.
            tradeable: index >= crate::trades::FIRST_TRADEABLE
                && desc
                    .and_then(|desc| desc.item.as_ref())
                    .is_some_and(|item| !item.soulbound),
        };
    }

    Some(slots)
}

/// Moves the items, and tells both sides how it went.
#[allow(clippy::too_many_arguments)]
async fn settle_trade(
    context: &Context,
    name: &str,
    character_id: i64,
    partner_name: &str,
    partner_character: i64,
    mine: Vec<i16>,
    theirs: Vec<i16>,
) {
    // What is in each offered slot, read now rather than remembered from when the offer was made.
    // The store refuses a trade whose items have moved since, and this is what it compares against.
    let Some(first) = offered_items(context, character_id, &mine).await else {
        context
            .trades
            .finished(name, partner_name, 2, "Trade unsuccessful.");
        return;
    };
    let Some(second) = offered_items(context, partner_character, &theirs).await else {
        context
            .trades
            .finished(name, partner_name, 2, "Trade unsuccessful.");
        return;
    };

    let outcome = context
        .store
        .trade(
            &hendra_store::Offer::new(
                character_id,
                first,
                last_carried_slot(&context.store, character_id).await,
            ),
            &hendra_store::Offer::new(
                partner_character,
                second,
                last_carried_slot(&context.store, partner_character).await,
            ),
            EQUIPPED_SLOTS as i16,
        )
        .await;

    match outcome {
        Ok(_) => {
            context
                .trades
                .finished(name, partner_name, 0, "Trade successful.");

            // Both packs changed. Both sides are asked to re-read, including this one: the items
            // moved in the database and neither session's view of them survived that.
            context.trades.refresh(name);
            context.trades.refresh(partner_name);
        }
        Err(err) => {
            tracing::info!(%name, %partner_name, %err, "a trade was refused");
            context
                .trades
                .finished(name, partner_name, 2, &err.to_string());
        }
    }
}

/// What is in the slots one side offered.
///
/// A slot that turns out to be empty makes the whole offer wrong rather than smaller: the other
/// side agreed to what they were shown.
///
/// A soulbound item makes it wrong as well. The original strips those from an offer as it is
/// changed but not from the one an accept carries (`AcceptTradeHandler.cs:30` takes the client's
/// bitmask as it stands), which leaves a client that skips the change step able to trade one. The
/// last word is taken here instead, where nothing can go around it.
async fn offered_items(
    context: &Context,
    character_id: i64,
    slots: &[i16],
) -> Option<Vec<(i16, uuid::Uuid)>> {
    let held = context.store.character(character_id).await.ok()?.inventory;

    let offered: Vec<(i16, uuid::Uuid)> = slots
        .iter()
        .map(|slot| {
            held.iter()
                .find(|(at, _)| at == slot)
                .map(|(at, item)| (*at, *item))
        })
        .collect::<Option<Vec<_>>>()?;

    if offered
        .iter()
        .any(|(_, item)| is_soulbound(&context.catalog, *item))
    {
        return None;
    }

    Some(offered)
}

/// The rank from which a mute stops applying.
///
/// `/setrank` sets the account's `Admin` flag at eighty, and `Player.cs` reads that flag when it
/// decides whether the account is muted.
const MUTE_EXEMPT_RANK: i16 = 80;

/// Whether a mute on the record stops this account talking.
///
/// One decision for all four of the refusals, because the original makes it once: `Player.Muted`
/// is settled as `!Client.Account.Admin && IsMuted(...)` (`Player.cs:485-489`) and `/say`,
/// `/tell`, `/guild` and `/local` all read that one flag, so none of them can disagree with the
/// others about who may speak.
///
/// `/setrank` keeps `Admin` as `rank >= 80` (`RankedCommands.cs:1369`), which is what the rank
/// stands in for here.
fn mute_applies(
    admin_rank: i16,
    muted_until: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    admin_rank < MUTE_EXEMPT_RANK && muted_until.is_some_and(|until| until > now)
}

#[cfg(test)]
mod mutes {
    use super::*;

    fn at(minutes: i64) -> Option<chrono::DateTime<chrono::Utc>> {
        Some(chrono::Utc::now() + chrono::Duration::minutes(minutes))
    }

    #[test]
    fn a_mute_that_has_not_run_out_stops_an_ordinary_account_talking() {
        assert!(mute_applies(0, at(5), chrono::Utc::now()));
    }

    #[test]
    fn a_mute_that_has_run_out_stops_nobody() {
        assert!(!mute_applies(0, at(-5), chrono::Utc::now()));
        assert!(!mute_applies(0, None, chrono::Utc::now()));
    }

    #[test]
    fn staff_are_never_muted() {
        // `Muted = !Client.Account.Admin && ...` (`Player.cs:488`), and `/setrank` sets `Admin` at
        // eighty (`RankedCommands.cs:1369`). A mute standing on the record of somebody at that
        // rank does not stop them talking, in any of the four channels.
        assert!(!mute_applies(MUTE_EXEMPT_RANK, at(5), chrono::Utc::now()));
        assert!(!mute_applies(100, at(5), chrono::Utc::now()));
        assert!(mute_applies(MUTE_EXEMPT_RANK - 1, at(5), chrono::Utc::now()));
    }
}

/// Works out what a player meant and, if they may say it, says it.
async fn say_something(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    limit: &mut crate::chat::Limit,
    line: &str,
) -> Option<(String, String)> {
    use crate::chat::Said;

    let said = crate::chat::read(line);
    if said == Said::Nothing {
        return None;
    }

    // Commands are answered before anything else is checked, exactly as `PlayerTextHandler` checks
    // for a leading slash before it checks the name, the mute and the spam. A muted player can
    // still type `/nexus`, and the two commands that repeat what was typed — `/l` and `/g` — do
    // their own checking.
    //
    // Handed back rather than run here, because carrying one out can move the player between worlds
    // and the placement belongs to the caller.
    if let Said::Command { name, rest } = said {
        return Some((name, rest));
    }

    // Read fresh rather than from the session's copy, so a mute applied while someone is playing
    // takes effect without waiting for them to reconnect.
    //
    // Staff are never muted: `Player.cs` settles the flag as `!Client.Account.Admin && ...`, and
    // `/setrank` keeps `Admin` as `rank >= 80`. A mute standing on the record of somebody at that
    // rank does not stop them talking.
    if let Ok(account) = context.store.account(player.account.id).await
        && mute_applies(account.admin_rank, account.muted_until, chrono::Utc::now())
    {
        deny(link, "Muted. You can not talk at this time.").await;
        return None;
    }

    let now = std::time::Instant::now();

    // The last check `PlayerTextHandler` makes, and the only one about how much is being said.
    // Refused in silence: the original returns without answering, so a spammer is not told what
    // caught them and cannot tune around it.
    if limit.repeats(line, now) {
        return None;
    }

    match said {
        Said::Say(text) => {
            placement
                .world
                .send(ToWorld::Chat {
                    handle: placement.handle,
                    text,
                })
                .await;
        }

        // Answered above, both of them.
        Said::Command { .. } | Said::Nothing => {}
    }

    None
}

/// Uses what is in a slot.
///
/// The item is read from the durable side rather than taken from the client, which names a slot
/// and nothing else: a client naming an item is making a claim, and a slot is a fact the server
/// can check.
///
/// The slot arrives in the client's flat numbering, as the vault panel's does, and is turned into
/// the one the table holds by `hendra_net::slot`. The two stacks are the exception: they are not
/// slots in the table at all, and they keep the numbers the original gave them.
async fn use_item(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    container: hendra_net::EntityId,
    flat: u16,
    aim: (f32, f32),
) {
    // Out of a bag on the ground rather than out of the player's own slots. The whole ask goes to
    // the world, because that is where a bag and everything in it lives: `Player.UseItem` resolves
    // the object id it was given and reads it as an `IContainer` (`Player.UseItem.cs:108-132`), and
    // no row in any table is involved.
    if container.0 != 0 && container != placement.handle.to_entity_id() {
        let (reply, answer) = tokio::sync::oneshot::channel();
        placement
            .world
            .send(ToWorld::UseFromContainer {
                handle: placement.handle,
                container,
                slot: flat.min(u8::MAX as u16) as u8,
                aim,
                reply,
            })
            .await;

        for effect in &answer.await.unwrap_or_default() {
            settle(link, context, player, placement, effect).await;
        }
        return;
    }

    // Drinking from a stack rather than from the pack. Taken durably first: a potion that heals and
    // is still in the stack is a potion that heals forever.
    if flat == HEALTH_STACK_SLOT || flat == MAGIC_STACK_SLOT {
        let magic = flat == MAGIC_STACK_SLOT;

        match context.store.take_potion(player.character.id, magic).await {
            Ok(true) => {}
            Ok(false) => return deny(link, "you have none of those").await,
            Err(_) => return deny(link, "try again shortly").await,
        }

        let name = if magic { MAGIC_POTION } else { HEALTH_POTION };
        let Some(kind) = context.catalog.type_of(name) else {
            return;
        };

        let (reply, answer) = tokio::sync::oneshot::channel();
        placement
            .world
            .send(ToWorld::UseItem {
                handle: placement.handle,
                item: kind,
                aim,
                reply,
            })
            .await;

        let _ = answer.await;
        send_containers(link, &context.catalog, &context.store, player).await;
        return;
    }

    let Some(slot) = hendra_net::slot::flat_to_durable(flat) else {
        deny(link, "there is no such slot").await;
        return;
    };

    let Some(identity) = read_slot(
        &context.store,
        Location::Inventory {
            character_id: player.character.id,
            slot,
        },
    )
    .await
    else {
        deny(link, "there is nothing in that slot").await;
        return;
    };

    let Some(kind) = context.catalog.type_of_uuid(identity) else {
        deny(link, "that is not something you can use").await;
        return;
    };

    // A consumable cannot be drunk while a trade is open. `Player.UseItem.cs:159` returns on
    // exactly this pair of conditions: the item in the offer would be spent and the offer would
    // still name it, so the other side agrees to something that is no longer there.
    if context
        .catalog
        .object(kind)
        .and_then(|object| object.item.as_ref())
        .is_some_and(|item| item.consumable)
        && context.trades.partner(&player.character.name).is_some()
    {
        deny(link, "you cannot do that while trading").await;
        return;
    }

    // An ability works where it is worn and nowhere else, or a player carries four tomes in the
    // pack and uses each in turn.
    if let Some(class) = context
        .catalog
        .type_of_uuid(player.character.class)
        .and_then(|found| context.catalog.class(found))
        && !hendra_characters::activates_from_slot(&context.catalog, class, slot, kind)
    {
        deny(link, "that only works when you are wearing it").await;
        return;
    }

    // A backpack somebody already has is handed back rather than eaten. The refusal has to happen
    // before the item is spent, since everything below this point consumes it: a player who used a
    // second one would be told they already had one and be charged for it anyway.
    if grants_a_backpack(&context.catalog, kind)
        && context
            .store
            .has_backpack(player.character.id)
            .await
            .unwrap_or(false)
    {
        deny(link, "you already have a backpack").await;
        return;
    }

    let consumable = context
        .catalog
        .object(kind)
        .and_then(|object| object.item.as_ref())
        .is_some_and(|item| item.consumable);

    // Spent before it is used, which is the order the original settles in and not merely the order
    // it happens to be written in. `UseItem` stages the successor into an inventory transaction,
    // commits it, and calls `Activate` only from inside the branch where the commit came back
    // true — a commit that failed does a `ForceUpdate` on the slot and returns, so the effect never
    // runs (`Player.UseItem.cs:193-227`, with the original's own comment on :205 that this "can
    // result in the loss of an item if inv trans fails").
    //
    // Which way round it is decides which way a failure falls. Used first and spent afterwards, a
    // write that is refused leaves the player with the heal and with the potion still in the slot,
    // and that is a duplication: the same drink can be had again. Spent first, a refused write
    // costs the drink and gives nothing back, which is a loss. The original chose the loss, and
    // losing one potion is the smaller wrong than an unlimited supply of them.
    //
    // Asking the world first is what keeps a refusal from costing anything at all: the three
    // reasons it would do nothing — dead, nothing to activate, not enough magic — are the original's
    // own gates and all of them sit before the commit there too.
    if consumable {
        let (reply, answer) = tokio::sync::oneshot::channel();
        placement
            .world
            .send(ToWorld::MayUseItem {
                handle: placement.handle,
                item: kind,
                reply,
            })
            .await;

        if !answer.await.unwrap_or(false) {
            return;
        }

        // An item that names a successor turns into it rather than disappearing. That is how the
        // game spells a consumable with several uses: an Elixir of Health 7 becomes a 6, and the 1
        // has no successor and goes. Removing it outright would turn every elixir into a single
        // drink.
        let settled = match context.catalog.successor_of(kind) {
            Some(successor) => {
                context
                    .store
                    .succeed_item(player.character.id, slot, identity, successor)
                    .await
            }
            None => {
                context
                    .store
                    .take_item(player.character.id, slot, identity)
                    .await
            }
        };

        // The counterpart of `ForceUpdate(slot)`: the slot is told again as it really stands and
        // nothing is activated. A write is refused when the row moved underneath — the item was
        // already spent by another ask, or moved out of the slot — so the client's guess about what
        // it holds is the thing that needs correcting.
        if settled.is_err() {
            send_containers(link, &context.catalog, &context.store, player).await;
            return;
        }
    }

    let (reply, answer) = tokio::sync::oneshot::channel();
    placement
        .world
        .send(ToWorld::UseItem {
            handle: placement.handle,
            item: kind,
            aim,
            reply,
        })
        .await;

    let ran = answer.await.unwrap_or_default();

    // What the world handed back. It carries out what belongs to the room and returns what does
    // not, so this is the other half of using an item rather than an afterthought: without it a
    // dye is read, returned and dropped.
    for effect in &ran {
        settle(link, context, player, placement, effect).await;
    }

    if consumable {
        send_containers(link, &context.catalog, &context.store, player).await;
        refresh_equipment(context, player, placement).await;
    }
}

/// Carries out one effect the world handed back.
///
/// These change something no world owns: an account's currency, a character's wardrobe, a pet that
/// outlives the room. Each is durable, which is why the world refuses to guess at them.
async fn settle(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    effect: &hendra_content::Effect,
) {
    use hendra_content::Effect;
    use hendra_content::activate::{Appearance, Boost, Currency, Unlock};

    match effect {
        Effect::Currency { kind, amount } => {
            let currency = match kind {
                Currency::Fame => hendra_store::Currency::Fame,
                Currency::Token => hendra_store::Currency::Tokens,
            };
            let _ = context
                .store
                .credit(player.account.id, currency, *amount)
                .await;
        }

        Effect::Appearance { kind, value } => match kind {
            // The world resolves a dye into the layer the item declared before it hands it back,
            // so what arrives here already names cloth or accessory. The bare `Dye` cannot reach
            // this point; it is the unresolved form, and writing on a guess is what put every
            // solid-colour dye in the accessory slot.
            Appearance::Dye => {}

            Appearance::DyeCloth | Appearance::DyeAccessory => {
                let slot = if matches!(kind, Appearance::DyeCloth) {
                    hendra_store::DyeSlot::Cloth
                } else {
                    hendra_store::DyeSlot::Accessory
                };

                let _ = context
                    .store
                    .set_dye_layer(player.character.id, slot, *value as i32)
                    .await;
            }
            Appearance::Skin => {
                // Granted before it is worn, because using a skin item is how it is obtained. The
                // identity is the content's, which is what every other path that grants a skin --
                // the purchase over HTTP, the reskin command -- writes into the account.
                let Some(skin) = context
                    .catalog
                    .object(hendra_content::ObjectType(*value as u16))
                    .map(|desc| desc.uuid)
                else {
                    return;
                };

                let _ = context.store.grant_skin(player.account.id, skin).await;
                let _ = context
                    .store
                    .wear_skin(player.account.id, player.character.id, *value as i32, skin)
                    .await;
            }
            Appearance::PetSkin => {
                // Applied to the first pet, which is the one a player has out.
                if let Ok(pets) = context.store.pets(player.account.id).await
                    && let Some(pet) = pets.first()
                {
                    let _ = context
                        .store
                        .set_pet_skin(player.account.id, pet.id, *value as i32)
                        .await;
                }
            }
        },

        Effect::Pet { name, permanent } => {
            let kind = name
                .as_deref()
                .and_then(|name| context.catalog.type_of(name))
                .and_then(|found| context.catalog.object(found))
                .map(|desc| desc.uuid);

            if let Some(kind) = kind {
                match context
                    .store
                    .add_pet(player.account.id, kind, *permanent)
                    .await
                {
                    Ok(_) => {}
                    Err(hendra_store::StoreError::Refused(why)) => deny(link, why).await,
                    Err(_) => {}
                }
            }
        }

        Effect::Boost {
            kind,
            duration_ms,
            multiplier,
        } => {
            let name = match kind {
                Boost::Experience => "experience",
                Boost::LootDrop => "loot_drop",
                Boost::LootTier => "loot_tier",
            };
            let _ = context
                .store
                .add_boost(
                    player.account.id,
                    name,
                    *multiplier,
                    (*duration_ms / 1000).max(1) as i64,
                )
                .await;

            // The durable row is what survives a relog; the body has to be told separately, or a
            // boost drunk in front of a boss would not double the boss. `AEXPBoost` sets the clock
            // on the player it was used by (`Player.UseItem.cs:531-539`).
            if matches!(kind, Boost::Experience) {
                placement
                    .world
                    .send(crate::world_task::ToWorld::ExperienceBoost {
                        handle: placement.handle,
                        milliseconds: (*duration_ms).min(i32::MAX as u32) as i32,
                    })
                    .await;
            }
        }

        Effect::Unlock { kind, value } => match kind {
            Unlock::Backpack => match context.store.grant_backpack(player.character.id).await {
                Ok(true) => send_containers(link, &context.catalog, &context.store, player).await,
                Ok(false) => deny(link, "you already have a backpack").await,
                Err(_) => {}
            },
            Unlock::Class => {
                if let Some(class) = context
                    .catalog
                    .type_of(value)
                    .and_then(|found| context.catalog.object(found))
                {
                    let _ = context
                        .store
                        .purchase_class(player.account.id, class.uuid)
                        .await;
                }
            }
            // A common crate, whose contents are eighteen names written into the server rather
            // than into the content: `AECommonCrate` (`Player.UseItem.cs:377-431`) reads nothing
            // off the activation at all, picks one of the eighteen at random and puts it in the
            // first free carried slot. A crate opened with a full pack gives nothing and says
            // nothing, which is the original's `if (frislot != -1)` and worth keeping: it is what
            // stops the item from being consumed into a full inventory.
            Unlock::LootBox => {
                let pick = COMMON_CRATE[rand_index(COMMON_CRATE.len())];
                let Some(inside) = context
                    .catalog
                    .type_of(pick)
                    .and_then(|found| context.catalog.object(found))
                else {
                    tracing::warn!(item = pick, "a common crate names an item the content lacks");
                    return;
                };

                let last = last_carried_slot(&context.store, player.character.id).await;
                if context
                    .store
                    .give_item(player.character.id, inside.uuid, EQUIPPED_SLOTS as i16, last)
                    .await
                    .is_ok()
                {
                    send_containers(link, &context.catalog, &context.store, player).await;
                    say(link, &format!("Awesome you found {pick}")).await;

                    // Announced to the room in the original by a loop that calls the opener's own
                    // `SendInfo` once per player standing there (`Player.UseItem.cs:428-429`), so
                    // nobody but the opener ever sees it and they see it as many times as there
                    // are people. Said once here: the repetition is the bug and carries nothing.
                    deny(
                        link,
                        &format!(
                            "{} unboxed a {pick} from a Common Crate!",
                            player.character.name
                        ),
                    )
                    .await;
                }
            }
            // A receipt for a purchase made elsewhere. `UnlockSlot` (`Player.UseItem.cs:541-544`)
            // is one `SendInfo` and nothing else; the capacity it claims to buy is bought through
            // the vault panel.
            Unlock::VaultSlot => {
                deny(link, "New vault chest unlocked successfully.").await;
            }
            Unlock::MysteryDye => {
                // What is inside is decided by content that names it, and nothing names one yet.
                deny(link, "there is nothing inside").await;
            }
        },

        // A key that changes a door rather than an account.
        //
        // `AEUnlockPortal` (`Player.UseItem.cs:433-510`) takes the locked portal out of the room
        // and stands the one that leads to the named dungeon in its place, then tells everybody
        // there. Recording the unlock and stopping — which is all this used to do — left the locked
        // door standing and the player holding a spent item.
        //
        // The portal to build is the first one the destination world's definition claims
        // (`:456-463`, `proto.portals[0]`), which is the same table `destination_of` is built from
        // read the other way round.
        Effect::UnlockPortal { dungeon, locked } => {
            let Some(locked_type) = context.catalog.type_of(locked) else {
                tracing::warn!(%locked, "a key names a locked portal the content does not have");
                return;
            };
            let Some(unlocked_type) = context.worlds.portal_to(dungeon) else {
                tracing::warn!(%dungeon, "unable to unlock portal: no world of that name");
                return;
            };

            // A world definition can name a portal type the content does not carry, and the
            // original throws when it does: `Entity.Resolve` indexes `ObjectTypeToElement`
            // directly (`Entity.cs:589`). This leaves the locked door standing and says so.
            //
            // The shipped Wine Cellar is that case. `WineCellar.jw` names `0x63f4`, which appears
            // in no object file, so the Wine Cellar Incantation — the only item with this effect —
            // could never open its door. Our own copy of the content says `0x242`, the Wine Cellar
            // Portal (`content/worlds/WineCellar.jw`); the specification keeps its `0x63f4`, and
            // this check keeps the next such definition from being a crash.
            if context.catalog.object(ObjectType(unlocked_type)).is_none() {
                tracing::warn!(
                    %dungeon,
                    portal = format!("{unlocked_type:#06x}"),
                    "unable to unlock portal: the world's portal is in no content file"
                );
                return;
            }

            let (reply, answer) = tokio::sync::oneshot::channel();
            let sent = placement
                .world
                .send(ToWorld::UnlockPortal {
                    at: placement.handle,
                    locked: locked_type,
                    unlocked: ObjectType(unlocked_type),
                    duration_ms: PORTAL_TIMEOUT_MS,
                    announced: crate::world_task::Announced {
                        opener: player.character.name.clone(),
                        dungeon: context
                            .worlds
                            .scoreboard_name(dungeon)
                            .unwrap_or(dungeon.as_str())
                            .to_string(),
                    },
                    reply,
                })
                .await;
            if !sent {
                return;
            }

            let Ok(Some(portal)) = answer.await else {
                return;
            };

            // Kept because it is already there and costs nothing, though nothing in the original
            // has it: a record that this account has opened this door.
            let _ = context.store.unlock_portal(player.account.id, dungeon).await;

            // The same key `travel` builds when somebody steps through, so the room behind this
            // door is the one the opener was let into.
            let key = format!(
                "{dungeon}#{}:{}",
                placement.world.key,
                portal.0
            );
            context
                .worlds
                .dungeons()
                .claim(&key, &player.character.name);
        }

        Effect::Portal { name, duration_ms } => {
            // Opened where the player stands, which is what a portal item means.
            let kind = context.catalog.type_of(name).unwrap_or(ObjectType::NONE);
            if !kind.is_none() {
                placement
                    .world
                    .send(ToWorld::OpenPortal {
                        at: placement.handle,
                        kind,
                        duration_ms: *duration_ms,

                        // A mystery portal names nobody: `AEMysteryPortal` opens the door and says
                        // nothing, and the world behind it is not anybody's to invite into.
                        opened_by: None,
                        reply: None,
                    })
                    .await;
            }
        }

        // A dungeon key. `AECreate` (`Player.UseItem.cs:591-624`) resolves the named object, refuses
        // anything that is not a portal, stands it where the player is, gives it a lifetime, marks
        // it as opened by them and announces it to the room. The world behind it belongs to whoever
        // used the key, which is what `/dinvite` and `/daccept` are about.
        Effect::Create { child } => {
            let Some(kind) = context.catalog.type_of(child) else {
                return;
            };
            let Some(object) = context.catalog.object(kind) else {
                return;
            };
            if hendra_sim::Kind::of_class(&object.class) != hendra_sim::Kind::Portal {
                return;
            }

            // One rank may only open a key in the vault, and nowhere else: `AECreate`
            // (`Player.UseItem.cs:600-603`) returns without opening anything when the account's rank
            // is exactly forty and the world it is standing in is not a `Vault`. Forty is also the
            // rank at and above which an account may not trade (`Player.Trade.cs:37`), so this is
            // that restriction reaching the one item that could carry goods out of a room.
            if context
                .store
                .account(player.account.id)
                .await
                .is_ok_and(|account| account.admin_rank == VAULT_ONLY_KEYS_RANK)
                && placement.world.name.as_ref() != "Vault"
            {
                return;
            }

            let opener = player.character.name.clone();
            let (reply, answer) = tokio::sync::oneshot::channel();

            let sent = placement
                .world
                .send(ToWorld::OpenPortal {
                    at: placement.handle,
                    kind,
                    duration_ms: PORTAL_TIMEOUT_MS,
                    opened_by: Some(crate::world_task::Announced {
                        opener: opener.clone(),
                        dungeon: object
                            .dungeon_name
                            .clone()
                            .or_else(|| object.display_id.clone())
                            .unwrap_or_else(|| object.id.clone()),
                    }),
                    reply: Some(reply),
                })
                .await;

            if !sent {
                return;
            }

            // The door's own number is what ties it to the room it will open onto: `travel` builds
            // the same key from the same three parts when somebody steps through
            // (`Portal.WorldInstance`, `Portal.cs:90`).
            let Ok(Some(portal)) = answer.await else {
                return;
            };
            let Some(destination) = context.worlds.destination_of(kind.0) else {
                return;
            };

            let key = format!("{destination}#{}:{}", placement.world.key, portal.0);
            context.worlds.dungeons().claim(&key, &opener);
        }

        Effect::Unsupported { name } => {
            tracing::debug!(%name, "an unimplemented activate was used");
        }

        // Everything else was the world's, and it has already done it.
        _ => {}
    }
}

/// What is inside a common crate.
///
/// `AECommonCrate` (`Player.UseItem.cs:379-408`), in the original's order. Eighteen names written
/// into the server rather than into the content, so a crate's activation carries no attributes and
/// two different crates hold the same eighteen things.
const COMMON_CRATE: [&str; 18] = [
    "Ravenheart Sword",
    "Demon Edge",
    "Staff of Horror",
    "Golden Bow",
    "Fire Dagger",
    "Wand of Death",
    "Cloak of the Night Thief",
    "Elvencraft Quiver",
    "Scorching Blast Spell",
    "Tome of Rejuvenation",
    "Red Iron Helm",
    "Reinforced Shield",
    "Seal of the Aspirant",
    "Stingray Poison",
    "Soul Siphon Skull",
    "Savage Trap",
    "Neutralization Orb",
    "Hallucination Prism",
];

/// One index out of `count`, from the clock.
///
/// `new Random(Guid.NewGuid().GetHashCode())` (`Player.UseItem.cs:413`) — a fresh stream for every
/// crate, seeded from something unrepeatable, so two crates opened in the same instant do not agree.
fn rand_index(count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.subsec_nanos() as usize)
        .unwrap_or(0);
    now % count
}

/// The one rank that may only use a dungeon key inside the vault.
///
/// `AECreate` (`Player.UseItem.cs:601-603`) tests `Client.Account.Rank == 40` exactly, not
/// `>=`, so a rank above it is unrestricted again.
const VAULT_ONLY_KEYS_RANK: i16 = 40;

/// How long a portal a key opened stands for.
///
/// Thirty seconds. `AECreate` reads `gameData.Portals[objType].Timeout` and multiplies by a
/// thousand (`Player.UseItem.cs:601`, `:614`); `PortalDesc` (`XmlDescriptors.cs:493-494`) defaults
/// that to thirty when the object carries no `<Timeout>`, and not one of the portals in the shipped
/// content carries one.
const PORTAL_TIMEOUT_MS: u32 = 30_000;

/// The runtime number an item identity currently has.
///
/// Resolved on the way to the wire rather than stored, because a runtime number is assigned at load
/// and only the identity survives content changing.
pub(crate) fn number(catalog: &hendra_content::Catalog, item: uuid::Uuid) -> Option<u16> {
    catalog.type_of_uuid(item).map(|found| found.0)
}

/// What the worn slots add, one bonus at a time.
///
/// Read from what is worn rather than accumulated as items move, so a missed change cannot leave a
/// stat permanently wrong: the answer is always a function of the inventory as it stands.
///
/// Separate entries rather than a summed layer, because the floor the original puts under a negative
/// bonus is applied to each one on its own as it goes in (`BoostStatManager.cs:168-171`). Summing
/// here would throw away the only information that floor needs.
fn worn_bonuses(
    catalog: &hendra_content::Catalog,
    inventory: &[(i16, uuid::Uuid)],
) -> Vec<(hendra_content::Stat, i32)> {
    let worn = inventory
        .iter()
        .filter(|(slot, _)| *slot < hendra_characters::EQUIPPED_SLOTS)
        .filter_map(|(_, item)| catalog.type_of_uuid(*item))
        .filter_map(|found| catalog.object(found))
        .filter_map(|desc| desc.item.as_ref());

    let mut bonuses = hendra_sim::stats::equipment_bonuses(worn);

    // And what a completed set adds on top, which is on none of its pieces: a set gives nothing for
    // three of its four, so this can only be answered by looking at all of them together.
    //
    // The first completed set and no other, since `ApplySetBonus` `return`s as soon as it has
    // applied one (`BoostStatManager.cs:88`) -- the same set the skin below is taken from, so the
    // stats and the look can never come from two different ones.
    if let Some(set) = catalog
        .sets_worn(&|slot| in_slot(catalog, inventory, slot))
        .first()
    {
        for activate in &set.gives {
            // A set's `IncrementStat` is a boost rather than the permanent rise the same effect
            // means on a potion: `ApplySetBonus` calls `IncrementBoost` for it. So it lasts exactly
            // as long as the set is worn, and taking a piece off recomputes this and takes it away.
            //
            // Only that one, because only four names reach anything in that switch and the rest
            // fall through it silently (`BoostStatManager.cs:71-88`): a `StatBoostSelf` written on a
            // set raises nothing at all there, whatever it would mean on an item.
            let raised = match hendra_content::Effect::of(activate) {
                hendra_content::Effect::IncrementStat { stat, amount } => Some((stat, amount)),
                _ => None,
            };

            if let Some((stat, amount)) = raised
                && let Some(stat) = hendra_content::ALL_STATS.get(stat as usize)
            {
                bonuses.push((*stat, amount));
            }
        }
    }

    bonuses
}

/// The look a completed set puts on, and `None` when what is worn completes none.
///
/// The half of `ApplySetBonus` that is not a stat: `ChangeSkin` writes the set's own skin and size
/// straight onto the player (`BoostStatManager.cs:73-76`), which is what makes a finished set
/// visible to everybody in the room rather than only on the character sheet. It also changes what
/// the client draws for the weapon, since the bullet a set names is read off the skin it puts on.
///
/// Only the first completed set counts. `ApplySetBonus` `return`s as soon as it has applied one
/// (`:88`), so a second set every piece of which was somehow also in place is never reached.
fn worn_set_skin(
    catalog: &hendra_content::Catalog,
    inventory: &[(i16, uuid::Uuid)],
) -> Option<hendra_content::SetSkin> {
    catalog
        .sets_worn(&|slot| in_slot(catalog, inventory, slot))
        .first()
        .and_then(|set| set.changes_skin())
}

/// The equipped weapon's first projectile, as damage stats 8 and 9.
///
/// `SetWeaponDamage` reads `Inventory[0].Projectiles[0]` and writes its bounds into the base layer
/// on every recalculation, so the pair is always the weapon currently held rather than anything
/// remembered — and zero when the slot is empty or what is in it fires nothing
/// (`BaseStatManager.cs:40-53`).
fn worn_weapon_damage(
    catalog: &hendra_content::Catalog,
    inventory: &[(i16, uuid::Uuid)],
) -> (i32, i32) {
    inventory
        .iter()
        .find(|(slot, _)| *slot == 0)
        .and_then(|(_, item)| catalog.type_of_uuid(*item))
        .and_then(|found| catalog.object(found))
        .and_then(|desc| desc.projectiles.first())
        .map(|shot| (shot.min_damage, shot.max_damage))
        .unwrap_or((0, 0))
}

/// What is in the weapon slot, by object type, and `None` when nothing the catalog knows is.
///
/// The same read the damage above comes from and the same one the original does on every shot:
/// `PlayerShootHandler` refuses a shot naming anything other than `Inventory[0]`
/// (`Player.AntiCheat.cs:88-90`) and takes the projectile from that item
/// (`PlayerShootHandler.cs:46`). The world holds a weapon on the body instead, so it has to be told
/// each time the slot changes or a swapped wand keeps firing the wand before it.
fn worn_weapon(
    catalog: &hendra_content::Catalog,
    inventory: &[(i16, uuid::Uuid)],
) -> Option<hendra_content::ObjectType> {
    inventory
        .iter()
        .find(|(slot, _)| *slot == 0)
        .and_then(|(_, item)| catalog.type_of_uuid(*item))
        .filter(|item| catalog.object(*item).is_some())
}

/// What is worn in one slot, by object type.
fn in_slot(
    catalog: &hendra_content::Catalog,
    inventory: &[(i16, uuid::Uuid)],
    slot: u16,
) -> Option<hendra_content::ObjectType> {
    inventory
        .iter()
        .find(|(at, _)| *at == slot as i16)
        .and_then(|(_, item)| catalog.type_of_uuid(*item))
}

/// The eight base stats a character arrives with.
///
/// Three sources, weakest first, each overwriting the last.
///
/// The weakest is the class at this level: its starting value plus the average of its per-level
/// range for every level the row says the character has gained. That is the same averaging
/// `Progress::jump_to_max_level` grants for a skipped level, and at level one it is exactly the
/// starting line, so a character that has only just been made is unaffected.
///
/// Next are the row's own maxima, which are two of the eight stats kept in columns of their own.
/// What is stored there is what the body was playing with, equipment included, so what equipment
/// contributes is taken back off: only the layer levelling produced belongs in the base.
///
/// Strongest is whatever the `stats` column actually records, slot by slot. A row that records all
/// eight — every character saved since the column existed — is therefore restored exactly, and the
/// two weaker sources never show through.
///
/// The order is what makes an unreadable column survivable. A level-twenty character whose stats
/// were never recorded arrives with a level-twenty line and its own stored maxima, and the next
/// checkpoint writes that down. Seeding it from the class's starting line instead would hand it the
/// numbers it had at level one and then make them permanent.
/// The eight stats a class begins with, which is what giving a character up puts it back to.
///
/// `pd.Stats[i].StartingValue` (`networking/handlers/PrestigeHandler.cs:56-63`). Zeroes for a class
/// the catalog cannot name, which is the same nothing a character with no readable class is already
/// treated as having.
fn starting_stats(catalog: &hendra_content::Catalog, class: uuid::Uuid) -> [i32; 8] {
    let Some(described) = catalog.type_of_uuid(class).and_then(|kind| catalog.class(kind)) else {
        return [0; 8];
    };

    let mut starting = [0i32; 8];
    for stat in hendra_content::STATS {
        starting[stat.index()] = described.stat(stat).starting;
    }
    starting
}

fn levelled_stats(
    class: &hendra_content::PlayerDesc,
    character: &hendra_store::Character,
    worn: [i32; hendra_content::STAT_COUNT],
) -> hendra_sim::stats::Stats {
    use hendra_content::{STATS, Stat};

    let levels = (character.level.max(1) as i32) - 1;

    let mut base = [0i32; 8];
    for stat in STATS {
        let growth = class.stat(stat);
        let gained = (growth.min_increase + growth.max_increase) * levels / 2;
        base[stat.index()] = (growth.starting + gained).clamp(0, growth.maximum.max(0));
    }

    for (stat, stored) in [
        (Stat::MaxHitPoints, character.max_hp),
        (Stat::MaxMagicPoints, character.max_mp),
    ] {
        let index = stat.index();
        let ceiling = class.stat(stat).maximum.max(0);
        base[index] = (stored - worn[index]).clamp(0, ceiling);
    }

    for (index, recorded) in character.stats.iter().enumerate() {
        if let (Some(slot), Some(value)) = (base.get_mut(index), recorded) {
            *slot = *value;
        }
    }

    hendra_sim::stats::Stats::from_base(&base)
}

/// The body a character arrives in.
///
/// Health travels with the player rather than resetting at every door, which is the difference
/// between a dungeon and a series of unrelated rooms. The weapon is whatever is in slot zero, so
/// unequipping one and walking through a portal does not hand it back.
/// What is left of a deadline, in milliseconds, or zero when there is none.
fn remaining_ms(ends: Option<std::time::Instant>) -> i32 {
    ends.map(|ends| {
        ends.saturating_duration_since(std::time::Instant::now())
            .as_millis()
            .min(i32::MAX as u128) as i32
    })
    .unwrap_or(0)
}

async fn arrival_of(
    player: &crate::accounts::Session,
    context: &Context,
) -> crate::world_task::Arrival {
    // Read here rather than carried on the session, because a guild changes while somebody is
    // playing: being invited, promoted or thrown out are all things somebody else does. `Player`'s
    // constructor reads the account's own fields at the same moment, once per body
    // (`Player.cs:405-406`), so this is one query per door rather than one per tick.
    let guild = match context.store.guild_of(player.account.id).await {
        Ok(Some((id, rank))) => context
            .store
            .guild(id)
            .await
            .ok()
            .map(|guild| (guild.name, rank.number())),
        _ => None,
    };

    // The class is stored as an identity; the world draws by number.
    let avatar = context
        .catalog
        .type_of_uuid(player.character.class)
        .unwrap_or(ObjectType::NONE);

    let bonuses = worn_bonuses(&context.catalog, &player.character.inventory);

    // The best fame this account has ever reached with this class, which is the figure the class
    // quest measures against -- not the fame of the character standing here.
    // `FameCounter.ClassStats[ObjectType].BestFame` (`Player.cs:537`).
    let best_class_fame = hendra_characters::Unlocks::load(&context.store, player.account.id)
        .await
        .map(|unlocks| unlocks.best(&context.catalog, avatar).1)
        .unwrap_or(0)
        .max(player.character.fame);

    let stats = context
        .catalog
        .class(avatar)
        .map(|class| levelled_stats(class, &player.character, hendra_sim::stats::summed(&bonuses)))
        .unwrap_or_default();

    crate::world_task::Arrival {
        avatar,

        // Read when the session started rather than at every door: a boost lasts half an hour and a
        // player walks through a dozen doors in one, so re-reading it per world would be a query
        // per door for a number that has not moved.
        loot_drop: player.loot_drop,

        // What is left of the boost at this door. The world counts down from here, and the next
        // door measures the same deadline again, so a boost that lapsed while the player was in a
        // room does not come back when they leave it.
        experience_boost_ms: player
            .experience_boost_ends
            .map(|ends| {
                ends.saturating_duration_since(std::time::Instant::now())
                    .as_millis()
                    .min(i32::MAX as u128) as i32
            })
            .unwrap_or(0),
        stars: player.stars,

        // What the account may spend. `Player`'s constructor seeds all three from the account
        // (`Player.cs:399-404`), which is what every shop in the game charges against.
        purse: hendra_sim::world::Purse {
            // `Credits` in the original is the gold a shop takes, which is this account's `gold`;
            // `credits` here is a separate column nothing in a world charges against.
            credits: player.account.gold,
            fame: player.account.fame,
            prestige: player.prestige,
        },

        // The icon over a muted player's head, which is the only sign of a mute they get before
        // they try to speak. Read from the session's copy of the account, so a mute applied while
        // they are already playing shows on their next world rather than at once; the chat refusal
        // (`command_or_speech`) reads the store fresh and takes effect immediately either way.
        muted: player
            .account
            .muted_until
            .is_some_and(|until| until > chrono::Utc::now()),

        // What the wardrobe wrote. `Player.Init` reads the character's skin on every arrival
        // (`Player.cs:431-436`), which is what makes a skin survive a portal; the world checks it
        // against the class before it puts it on.
        skin: player.character.skin.clamp(0, u16::MAX as i32) as u16,

        // And the set that dresses over it, if all four pieces are still on. A body is new in every
        // world and `ApplySetBonus` runs when its stat manager is built, so a player who walked into
        // a dungeon wearing a set is wearing it on the far side of the portal too.
        set_skin: worn_set_skin(&context.catalog, &player.character.inventory),

        guild,

        // `Account.Admin`, which `/setrank` sets from rank eighty upward
        // (`market::ADMIN_FLAG_RANK`). All it decides on screen is the colour of the star beside
        // the name (`Player.as:750-756`), and it is read here rather than carried so a rank granted
        // mid-session shows at the next door.
        admin: player.account.admin_rank >= hendra_store::market::ADMIN_FLAG_RANK,

        // `Account.NameChosen`, which a reserved name stands for here: those are the names an
        // unnamed account is handed and the ones no account may choose, so wearing one means never
        // having chosen (`common/Database.cs:82-84`). It colours the name over the head.
        name_chosen: !hendra_store::is_guest_name(&player.account.name),

        // What a dye put on, read off the character the way `Player`'s constructor reads it
        // (`Player.cs:412-413`). Kept per character rather than per account, since two characters
        // on one account are dyed separately.
        dyes: (player.character.dye_cloth, player.character.dye_accessory),

        // The ceiling the fame bar fills towards, off the best run this account has had with this
        // class (`Player.cs:537`). Zero once every star has been earned, which is what the original
        // answers past two thousand.
        fame_goal: crate::accounts::fame_goal(best_class_fame),

        // The other two boost clocks, measured at this door like the experience one above.
        loot_drop_boost_ms: remaining_ms(player.loot_drop_boost_ends),
        loot_tier_boost_ms: remaining_ms(player.loot_tier_boost_ends),

        // Read fresh for the same reason the guild above is: a backpack is bought while somebody is
        // playing, and a body carrying a stale answer either hides slots that were paid for or
        // offers slots the server will refuse.
        has_backpack: match context.store.has_backpack(player.character.id).await {
            Ok(held) => held,
            Err(err) => {
                tracing::warn!(%err, character = player.character.id, "could not read a backpack");
                player.character.has_backpack
            }
        },

        stats,

        // What the stored character had reached. A session that goes on to level up carries the
        // live figures between worlds instead; this is only the starting point.
        progress: hendra_sim::leveling::Progress {
            level: player.character.level.max(1),
            experience: player.character.experience,
            fame: player.character.fame,
        },

        bonuses,
        weapon_damage: worn_weapon_damage(&context.catalog, &player.character.inventory),

        // Health as it was stored, floored at one rather than clamped to the row's maximum: the
        // maximum is a stat, and the world recomputes it from these stats and what is worn before
        // it seats the health in it. `Player.cs:424` reads the stored figure the same way, without
        // a clamp of its own.
        hp: player.character.hp.max(1),
        max_hp: stats.max_hp().max(1),
        mp: player.character.mp.max(0),
        weapon: worn_weapon(&context.catalog, &player.character.inventory),
    }
}

/// Moves a player from one world to another.
///
/// The order matters. The destination is opened and joined *before* the old world is left, so a
/// world that fails to start leaves the player where they were rather than nowhere at all.
#[allow(clippy::too_many_arguments)]
async fn travel(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    from: &Placement,
    name: &str,
    portal_type: u16,
    portal: EntityId,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<Placement> {
    let destination = match context.worlds.destination_of(portal_type) {
        Some(destination) => destination.to_string(),

        // Six portal objects name no world at all, and they are the ones a dungeon boss leaves
        // behind: `UsePortalHandler` keeps them in `_realmPortals` and answers them with
        // `GetRandomGameWorld()` (`UsePortalHandler.cs:13`, `:70-74`), which is a realm that has
        // not closed. Without this the way out of every cleared dungeon does nothing.
        None if crate::worlds::REALM_PORTALS.contains(&portal_type) => {
            crate::worlds::REALM.to_string()
        }

        None => return None,
    };

    // A dungeon belongs to the portal that opened it, not to the kind of portal it is.
    // `Portal.CreateWorld` (`Portal.cs:66-92`) hands back the one instance of a static world
    // (`p.id < 0`) and builds `new World(p)` for anything else, hanging it off that portal
    // (`Portal.cs:90`), so two parties walking into two Undead Lair portals are in two Undead
    // Lairs. The key stands in for the portal's `WorldInstance`: asking for it twice answers the
    // same room, which is what makes a party that arrives one at a time arrive together.
    if context.worlds.is_instanced(&destination) {
        // Keyed off the room the door stands in rather than off its name, because there can be two
        // of that room and each has a door of its own with the same entity number in it.
        let key = format!("{destination}#{}:{}", from.world.key, portal.0);
        let world = context.worlds.get_or_start_instance(&destination, &key)?;
        let arrived = enter(link, context, player, from, name, world, orders, died).await?;

        // Walking through the door of a dungeon somebody opened spends the invitation and marks
        // them as having been in, exactly as `UsePortalHandler` (`:86-92`) does on every use of a
        // portal that has a world hanging off it. The clock the ninety seconds are measured against
        // starts here too, because this is the moment the original's world is built.
        let dungeons = context.worlds.dungeons();
        if dungeons.is_player_dungeon(&key) {
            dungeons.began(&key);
            dungeons.entered(&key, name);
        }

        return Some(arrived);
    }

    let arrived = go_to(
        link,
        context,
        player,
        from,
        name,
        &destination,
        orders,
        died,
    )
    .await;

    // Somebody already standing in the realm asked for the realm, and got nothing because there is
    // nowhere to go: that is not a closed realm and must not become a trip to the Nexus.
    if arrived.is_some()
        || destination != crate::worlds::REALM
        || from.world.name.as_ref() == crate::worlds::REALM
    {
        return arrived;
    }

    // No realm would take them. `GetRandomGameWorld` (`RealmManager.cs:387-395`) answers the Nexus
    // when every realm is closed, and a closed realm is exactly what refuses a join here.
    go_to(
        link,
        context,
        player,
        from,
        name,
        crate::commands::NEXUS,
        orders,
        died,
    )
    .await
}

/// Moves a player to a named world.
///
/// The same move a portal makes, by name rather than by what was stepped into, because a world can
/// also send somebody somewhere: a realm that has closed sends everybody still in it to the castle.
#[allow(clippy::too_many_arguments)]
async fn go_to(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    from: &Placement,
    name: &str,
    destination: &str,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<Placement> {
    // Going back into the world you are already in is a no-op, not a rejoin. Rejoining would move
    // the player to the spawn point for no reason.
    if destination == from.world.name.as_ref() {
        return None;
    }

    // A personal world gets one instance per account, so two players in the vault are in two
    // rooms rather than looking at each other's chests.
    let world = context
        .worlds
        .get_or_start_for(destination, player.account.id)?;

    let arrival = handover(context, player, from).await;
    let handle = join(link, &world, name, arrival, orders, died).await?;

    from.world
        .send(ToWorld::Leave {
            handle: from.handle,
        })
        .await;

    Some(Placement { world, handle })
}

/// Writes a character down and builds the body it walks through the door in.
///
/// Everything a player becomes while playing — levels, the stats those levels granted, the damage
/// they are carrying — lives on the body in the world rather than in the store. Walking through a
/// door builds a new body, so without this the new one is built from the row read at login and the
/// whole session's progress is silently undone.
///
/// The store is written on the way through as well as the body being copied. The original has no
/// choice about this — a portal there is a reconnect, so `Client.Disconnect` saves and the new
/// connection loads the character back out of the database (`wServer/networking/Client.cs:215-234`,
/// `wServer/networking/handlers/LoadHandler.cs:20-57`) — and the effect is worth keeping: a server
/// that stops while somebody is between two rooms costs them the room, not the session.
///
/// A world that has already dropped the player answers nothing, and then the stored figures stand.
async fn handover(
    context: &Context,
    player: &crate::accounts::Session,
    from: &Placement,
) -> crate::world_task::Arrival {
    let mut arrival = arrival_of(player, context).await;

    // Taken before the old world is told to drop them, because after that there is nothing left to
    // ask, and used both to write the row and to seat the new body, so the two cannot disagree.
    let Some(vitals) = snapshot(from).await else {
        return arrival;
    };

    if let Err(err) = save_vitals(context, player, &vitals, Written::WithClass).await {
        tracing::warn!(%err, character = player.character.id, "could not save on the way through");
    }

    arrival.hp = vitals.hp;
    arrival.mp = vitals.mp;
    arrival.max_hp = vitals.max_hp;
    arrival.stats = hendra_sim::stats::Stats::from_base(&vitals.stats);
    arrival.progress = hendra_sim::leveling::Progress {
        level: vitals.level.max(1),
        experience: vitals.experience,
        fame: vitals.fame,
    };

    // What is worn now, not what was worn at login. The session's copy of the character was read
    // once and never again, so without this a ring picked up an hour ago stops counting the moment
    // its owner walks through a door, and the health it was granting goes with it.
    if let Ok(character) = context.store.character(player.character.id).await {
        arrival.bonuses = worn_bonuses(&context.catalog, &character.inventory);
        arrival.weapon_damage = worn_weapon_damage(&context.catalog, &character.inventory);
        arrival.weapon = worn_weapon(&context.catalog, &character.inventory);
        arrival.set_skin = worn_set_skin(&context.catalog, &character.inventory);
    }

    arrival
}

async fn refuse(link: &mut Link, reason: RejectReason) {
    let mut buf = Vec::new();
    ServerMessage::Rejected { reason }.encode(&mut Writer::new(&mut buf));
    let _ = link.send(Delivery::Stream, &buf).await;
}

/// Turns one arriving payload into a world command.
async fn dispatch(received: &Received, placement: &Placement) -> Outcome {
    let mut reader = Reader::new(received.payload());

    let message = match ClientMessage::decode(&mut reader) {
        Ok(message) => message,
        Err(err) => {
            // One undecodable packet is not worth dropping a player over, since a datagram can
            // arrive corrupted, but it is worth knowing about.
            tracing::debug!(%err, "ignoring an undecodable message");
            return Outcome::Continue;
        }
    };

    let handle = placement.handle;
    let world = &placement.world;

    let delivered = match message {
        ClientMessage::Input(input) => {
            world
                .send(ToWorld::Input {
                    handle,
                    x: input.x,
                    y: input.y,
                    client_time_ms: input.client_time_ms,
                    ack: input.ack,
                })
                .await
        }

        ClientMessage::Chat { text } => {
            return Outcome::Chat(text.to_owned());
        }

        ClientMessage::Shoot {
            angle,
            client_time_ms,
        } => {
            world
                .send(ToWorld::Shoot {
                    handle,
                    angle,
                    client_time_ms,
                })
                .await
        }

        ClientMessage::UseItem {
            container,
            slot,
            x,
            y,
        } => {
            return Outcome::UseItem {
                container,
                slot,
                x,
                y,
            };
        }

        // Handled by the caller, which owns the connection and the store.
        ClientMessage::MoveItem { from, to } => return Outcome::Move { from, to },

        ClientMessage::UsePortal { entity } => {
            return match ask_portal(world, handle, entity).await {
                Some(portal_type) => Outcome::Travel {
                    portal_type,
                    portal: entity,
                },
                None => Outcome::Continue,
            };
        }

        // A second hello on an established session is a client fault, not an attack; ignoring it is
        // safer than re-joining someone who is already in a world.
        ClientMessage::Hello { .. } => {
            tracing::debug!("ignoring a repeated hello");
            true
        }

        // Handled by the caller, which owns the connection, the store and the trade registry.
        ClientMessage::RequestTrade { name } => {
            return Outcome::Trade(Trade::Request(name.to_owned()));
        }
        ClientMessage::ChangeTrade { offer } => return Outcome::Trade(Trade::Change(offer)),
        ClientMessage::AcceptTrade { mine, theirs } => {
            return Outcome::Trade(Trade::Accept { mine, theirs });
        }
        ClientMessage::CancelTrade => return Outcome::Trade(Trade::Cancel),

        // Not a portal: there is nothing to step into, and somebody stuck has to be able to leave.
        ClientMessage::Escape => return Outcome::Escape,

        ClientMessage::Teleport { name } => {
            // The locked-out list is an account question, so it is answered by the caller rather
            // than by the world.
            return Outcome::TeleportTo(name.to_owned());
        }

        ClientMessage::Buy { merchant } => {
            // A guild hall upgrade is bought from something standing in the world like anything
            // else, but what it buys is the hall rather than an item, so it is asked about first.
            let (reply, answer) = tokio::sync::oneshot::channel();
            if world
                .send(ToWorld::BuyHallUpgrade {
                    handle,
                    merchant,
                    reply,
                })
                .await
                && let Ok(Some(upgrade)) = answer.await
            {
                return Outcome::BuyHallUpgrade(upgrade);
            }

            let (reply, answer) = tokio::sync::oneshot::channel();
            if !world
                .send(ToWorld::Buy {
                    handle,
                    merchant,
                    reply,
                })
                .await
            {
                return Outcome::Stop;
            }

            // What is for sale is the world's to say. A client that named the item could name a
            // cheaper one.
            return match answer.await {
                Ok(Some(sale)) => Outcome::Buy(sale),
                _ => Outcome::Say("There is nothing to buy here.".to_string()),
            };
        }

        ClientMessage::Prestige => return Outcome::Prestige,
        ClientMessage::PrestigeBuy { offer } => return Outcome::PrestigeBuy(offer),
        ClientMessage::Guild(command) => return Outcome::Guild(owned_guild(command)),
        ClientMessage::Market(command) => return Outcome::Market(command),
        ClientMessage::EditList { list, name, add } => {
            return Outcome::EditList {
                list,
                name: name.to_owned(),
                add,
            };
        }

        // Handled by the caller, which owns the connection and the store, and which is the only
        // thing that knows where the player is standing.
        ClientMessage::VaultMove {
            version,
            from_chest,
            from_slot,
            to_chest,
            to_slot,
        } => {
            return Outcome::VaultMove {
                version,
                from_chest,
                from_slot,
                to_chest,
                to_slot,
            };
        }
        ClientMessage::VaultBuy { chest_count } => return Outcome::VaultBuy { chest_count },

        ClientMessage::Pong {
            serial,
            client_time_ms,
        } => {
            return Outcome::Pong {
                serial,
                client_time_ms,
            };
        }
    };

    if delivered {
        Outcome::Continue
    } else {
        Outcome::Stop
    }
}

/// Asks the world whether the player may use a portal, and what kind it is.
async fn ask_portal(world: &WorldHandle, handle: Handle, portal: EntityId) -> Option<u16> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    if !world
        .send(ToWorld::UsePortal {
            handle,
            portal,
            reply,
        })
        .await
    {
        return None;
    }
    answer.await.ok().flatten()
}

/// Buys what a merchant is selling.
///
/// The price and the item come from the world, which read them from where the merchant stands. The
/// payment and the item move together in one transaction, so a purchase that cannot be delivered is
/// a purchase that was not paid for.
async fn buy(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    sale: crate::world_task::Sale,
) {
    // A player's listing is bought from the market, which moves the item from whoever owns it and
    // pays them. Buying it as though it were a shop's stock would mint a second copy and pay nobody.
    if let Some(listing) = sale.listing {
        return market(
            link,
            context,
            player,
            placement,
            hendra_net::MarketCommand::Buy {
                listing: listing as u64,
            },
        )
        .await;
    }

    if !hendra_sim::shop::admits(sale.rank, player.stars, player.account.admin_rank) {
        return deny(link, "Insufficient rank.").await;
    }

    let item = match context.catalog.object(sale.item) {
        Some(desc) => desc.uuid,
        None => return deny(link, "There is nothing to buy here.").await,
    };

    let bought = context
        .store
        .buy_item(hendra_store::Purchase {
            account_id: player.account.id,
            character_id: player.character.id,
            item,
            currency: sale.currency,
            price: sale.price,
            first_slot: EQUIPPED_SLOTS as i16,
            last_slot: last_carried_slot(&context.store, player.character.id).await,
        })
        .await;

    match bought {
        Ok(_) => {
            send_containers(link, &context.catalog, &context.store, player).await;

            // What was paid with is gone, and the body being played is what the client reads its
            // purse from: `TransactionItemComplete` writes all three back
            // (`realm/entities/vendors/Merchant.cs:192-195`).
            resend_purse(context, player, placement).await;
            say(link, "Purchase successful.").await;
        }
        Err(hendra_store::StoreError::Refused(why)) => deny(link, why).await,
        Err(err) => {
            tracing::error!(%err, "a purchase failed");
            deny(link, "try again shortly").await;
        }
    }
}

/// Tells the body being played what the account can spend now.
///
/// `Merchant.TransactionItemComplete` writes all three back onto the player the moment a purchase
/// goes through (`realm/entities/vendors/Merchant.cs:192-195`), and
/// `RemoveItemFromMarketAsync` clamps the fame down after the five it charges
/// (`realm/entities/player/Player.Market.cs:145-146`). Re-read rather than adjusted by hand,
/// because the store is what decided the amount: a second copy of the arithmetic here is a second
/// place for it to be wrong.
async fn resend_purse(context: &Context, player: &crate::accounts::Session, placement: &Placement) {
    let Ok(account) = context.store.account(player.account.id).await else {
        return;
    };

    placement
        .world
        .send(crate::world_task::ToWorld::Purse {
            handle: placement.handle,
            purse: hendra_sim::world::Purse {
                credits: account.gold,
                fame: account.fame,
                prestige: player.prestige,
            },
        })
        .await;
}

/// Carries out a guild command.
///
/// Every rule lives in the store, which decides them against rows rather than against what a client
/// claims: who is in which guild, who outranks whom, and whether a name is taken.
async fn guild(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    name: &str,
    asked: GuildAsk,
) {
    use hendra_store::Rank;

    let me = player.account.id;

    let outcome = match asked {
        GuildAsk::Create(guild_name) => context
            .store
            .found_guild(me, guild_name.trim())
            .await
            .map(|guild| format!("{} founded.", guild.name)),

        // An invitation is an offer, not an act.
        //
        // `GuildInviteHandler` records the guild on the invited player and sends them an
        // `InvitedToGuild`; nothing changes in the database until they answer with `/join`
        // (`GuildInviteHandler.cs:46-52`). Adding them outright is the same rows in the end but a
        // different game: it lets anybody with an officer's rank put a stranger in their guild
        // without being asked.
        //
        // Only somebody connected can be invited, because the original walks `Manager.Clients` to
        // find them (`:29`) and the offer it leaves lives in memory on their session.
        GuildAsk::Invite(who) => {
            let who = who.trim();

            match context.store.guild_of(me).await {
                Ok(Some((guild_id, rank))) if rank >= Rank::Officer => {
                    match context.store.account_by_name(who).await {
                        Ok(target) => {
                            let theirs = context.store.guild_of(target.id).await;
                            if matches!(theirs, Ok(Some(_))) {
                                Err(hendra_store::StoreError::Refused(
                                    "Player is already in a guild.",
                                ))
                            } else {
                                match context.trades.named(&target.name) {
                                    Some(online) => {
                                        let guild = context.store.guild(guild_id).await;
                                        let guild_name = guild
                                            .map(|found| found.name)
                                            .unwrap_or_else(|_| String::new());

                                        if let Ok(mut held) = context.guild_invites.lock() {
                                            held.insert(online.to_lowercase(), guild_id);
                                        }

                                        context.trades.send(
                                            &online,
                                            &ServerMessage::InvitedToGuild {
                                                name: player.account.name.clone(),
                                                guild: guild_name,
                                            },
                                        );

                                        // The original tells the officer nothing at all: the whole
                                        // of what the command does is put the offer in front of the
                                        // other player.
                                        return;
                                    }
                                    None => Err(hendra_store::StoreError::Refused(
                                        "Could not find the player to invite.",
                                    )),
                                }
                            }
                        }
                        Err(_) => Err(hendra_store::StoreError::Refused(
                            "Could not find the player to invite.",
                        )),
                    }
                }
                Ok(Some(_)) => Err(hendra_store::StoreError::Refused("Insufficient privileges.")),
                Ok(None) => Err(hendra_store::StoreError::Refused("you are not in a guild")),
                Err(err) => Err(err),
            }
        }

        // And a join is the answer to one. The guild has to be named, and named correctly:
        // `JoinGuildHandler` compares what was typed against the guild the standing invitation
        // points at and refuses anything else (`JoinGuildHandler.cs:33-40`), which is what stops a
        // single invitation from becoming a way into any guild on the server.
        GuildAsk::Join(guild_name) => {
            let invited = context
                .guild_invites
                .lock()
                .ok()
                .and_then(|held| held.get(&name.to_lowercase()).copied());

            match invited {
                None => Err(hendra_store::StoreError::Refused(
                    "You have not been invited to a guild.",
                )),
                Some(guild_id) => match context.store.guild(guild_id).await {
                    Err(_) => Err(hendra_store::StoreError::Refused("Internal server error.")),
                    Ok(guild) if !guild.name.eq_ignore_ascii_case(guild_name.trim()) => {
                        Err(hendra_store::StoreError::Refused(
                            "You have not been invited to join that guild.",
                        ))
                    }
                    Ok(_) => match context.store.join_guild(me, guild_id).await {
                        Ok(()) => {
                            if let Ok(mut held) = context.guild_invites.lock() {
                                held.remove(&name.to_lowercase());
                            }
                            Ok(format!("<{name}> has joined the guild!"))
                        }
                        Err(_) => Err(hendra_store::StoreError::Refused("Could not join guild.")),
                    },
                },
            }
        }

        GuildAsk::Remove(who) => match context.store.account_by_name(who.trim()).await {
            Ok(target) => context
                .store
                .remove_from_guild(me, target.id)
                .await
                .map(|()| format!("{} was removed.", target.name)),
            Err(_) => Err(hendra_store::StoreError::Refused("no such account")),
        },

        GuildAsk::SetRank(who, rank) => match context.store.account_by_name(who.trim()).await {
            Ok(target) => {
                let rank = Rank::from_number(rank as i16);
                context
                    .store
                    .set_guild_rank(me, target.id, rank)
                    .await
                    .map(|()| format!("{} is now {rank:?}.", target.name))
            }
            Err(_) => Err(hendra_store::StoreError::Refused("no such account")),
        },

        GuildAsk::SetBoard(text) => context
            .store
            .set_guild_board(me, text.trim())
            .await
            .map(|()| "The board was changed.".to_string()),

        GuildAsk::Leave => context
            .store
            .leave_guild(me)
            .await
            .map(|()| format!("{name} left the guild.")),
    };

    match outcome {
        Ok(said) => say(link, &said).await,
        Err(hendra_store::StoreError::Refused(why)) => deny(link, why).await,
        Err(hendra_store::StoreError::NameTaken) => deny(link, "that name is taken").await,
        Err(err) => {
            tracing::warn!(%err, "a guild command failed");
            deny(link, "try again shortly").await;
        }
    }
}

/// Carries out a market command.
///
/// Listing takes the item out of the inventory and buying puts it into another, both in one
/// transaction each, so an item is never in two places and never in none.
async fn market(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    command: hendra_net::MarketCommand,
) {
    use hendra_net::MarketCommand as Ask;

    // The marketplace is a place, as the vault is. Listing an item from the middle of a dungeon
    // would make the room decoration, and where somebody is standing is not something a client is in
    // a position to be trusted about.
    //
    // Browsing is allowed from anywhere: reading what is for sale takes nothing out of the world.
    if !matches!(command, Ask::Browse) && placement.world.name.as_ref() != "Marketplace" {
        return deny(link, "Can only market items in Marketplace.").await;
    }

    // `if (tradeTarget != null)`, checked before the slot and the price are looked at, so a trade
    // in progress is the reason given even when the slot is empty as well.
    if matches!(command, Ask::List { .. })
        && context.trades.partner(&player.character.name).is_some()
    {
        return deny(link, "Can't market items while trading.").await;
    }

    match command {
        Ask::Browse => {
            let listings = context
                .store
                .listings(MARKET_PAGE)
                .await
                .unwrap_or_default();

            for listing in listings {
                let name = context
                    .catalog
                    .type_of_uuid(listing.item)
                    .and_then(|kind| context.catalog.object(kind))
                    .map(|desc| desc.id.as_str())
                    .unwrap_or("something");

                say(
                    link,
                    &format!("#{} {} for {} fame", listing.id, name, listing.price),
                )
                .await;
            }
        }

        Ask::List { slot, price } => {
            let slot = (slot as i16).saturating_add(EQUIPPED_SLOTS as i16);

            match list_one(context, player, slot, price).await {
                Ok(()) => {
                    send_containers(link, &context.catalog, &context.store, player).await;
                    say(link, "Success! Your item has been placed on the market.").await;
                }
                Err(why) => deny(link, why).await,
            }
        }

        Ask::Cancel { listing } => {
            match context
                .store
                .cancel_listing(player.account.id, listing as i64, player.account.admin_rank)
                .await
            {
                Ok(_) => {
                    // The chest the item landed in is in the vault, so what is on screen here does
                    // not change; the pack is resent anyway because the fee came out of the fame
                    // the client is showing.
                    send_containers(link, &context.catalog, &context.store, player).await;

                    // And the fame itself: `if (acc.Fame < CurrentFame) CurrentFame = acc.Fame`
                    // (`realm/entities/player/Player.Market.cs:145-146`). Without it the client
                    // keeps showing the five that has already been taken.
                    resend_purse(context, player, placement).await;

                    // `RemoveOffer` follows the line with `giftChestOccupied`
                    // (`networking/handlers/MarketCommandHandler.cs:79-82`): the item is in the
                    // vault's chest, which is not where the player is standing.
                    notify(link, GIFT_CHEST_OCCUPIED).await;
                    say(
                        link,
                        "Removal succeeded. The item has been placed in your gift chest.",
                    )
                    .await;
                }
                Err(hendra_store::StoreError::Refused(why)) => deny(link, why).await,
                Err(_) => {
                    deny(
                        link,
                        "Failed to remove item from Market. Please try again later.",
                    )
                    .await
                }
            }
        }

        Ask::Buy { listing } => {
            let bought = context
                .store
                .buy_listing(hendra_store::MarketPurchase {
                    buyer_id: player.account.id,
                    character_id: player.character.id,
                    listing_id: listing as i64,
                    first_slot: EQUIPPED_SLOTS as i16,
                    last_slot: last_carried_slot(&context.store, player.character.id).await,
                })
                .await;

            match bought {
                Ok(where_it_went) => {
                    send_containers(link, &context.catalog, &context.store, player).await;

                    // A full pack does not refuse the sale: `Merchant.TransactionItem` gifts the
                    // item instead, and `SendNotifications` says so with `giftChestOccupied`
                    // (`realm/entities/vendors/Merchant.cs:204-208`).
                    if matches!(where_it_went, hendra_store::Delivered::ToGifts(_)) {
                        notify(link, GIFT_CHEST_OCCUPIED).await;
                    }

                    // The fame is gone, and the seller has been paid out of it.
                    resend_purse(context, player, placement).await;
                    say(link, "Bought.").await;
                }
                Err(hendra_store::StoreError::Refused(why)) => deny(link, why).await,
                Err(_) => deny(link, "try again shortly").await,
            }
        }
    }
}

/// Puts one carried slot on the market, answering with the line the original would say.
///
/// `AddToMarket(slot, price)`, and the reason it says nothing on its own is that the original's does
/// not either: `/market` announces one item and `/marketall` announces a count, and both would be
/// wrong if the listing itself also spoke.
///
/// The soulbound check `AddToMarket` would make is commented out in the original, so an item that
/// cannot be traded can still be sold here. Kept as it is.
async fn list_one(
    context: &Context,
    player: &crate::accounts::Session,
    slot: i16,
    price: i32,
) -> Result<(), &'static str> {
    // Both checked before the slot is read, in the original's order: an empty slot named with a
    // negative price is answered about the price.
    if slot > LAST_BACKPACK_SLOT {
        return Err("Invalid slot.");
    }
    if price < 0 {
        return Err("Your asking price should be greater than or equal to 0.");
    }

    let held = context
        .store
        .character(player.character.id)
        .await
        .ok()
        .and_then(|character| {
            character
                .inventory
                .iter()
                .find(|(at, _)| *at == slot)
                .map(|(_, item)| *item)
        });

    let Some(item) = held else {
        return Err("Can't market nothing...");
    };

    // Fame, which is what the market runs on: `PlayerMerchant` is built with
    // `Currency = CurrencyType.Fame`, the withdrawal fee is charged in fame, and `/myMarket` prints
    // its prices under the heading `fame`.
    match context
        .store
        .list_item(
            player.account.id,
            player.character.id,
            slot,
            item,
            hendra_store::Currency::Fame,
            price,
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(hendra_store::StoreError::Refused(why)) => Err(why),
        Err(_) => Err("Failed to add item to Market. Please try again later."),
    }
}

/// How many listings one browse shows.
const MARKET_PAGE: i64 = 20;

/// Adds or removes somebody from one of the account's lists.
async fn edit_list(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    list: hendra_net::AccountList,
    name: &str,
    add: bool,
) {
    let Ok(target) = context.store.account_by_name(name.trim()).await else {
        return deny(link, "no such account").await;
    };

    if target.id == player.account.id {
        return deny(link, "you cannot list yourself").await;
    }

    let kind = match list {
        hendra_net::AccountList::Ignored => hendra_store::ListKind::Ignored,
        hendra_net::AccountList::Locked => hendra_store::ListKind::LockedOut,
    };

    let outcome = if add {
        context
            .store
            .add_to_list(player.account.id, target.id, kind)
            .await
    } else {
        context
            .store
            .remove_from_list(player.account.id, target.id, kind)
            .await
    };

    match outcome {
        Ok(()) => {
            let what = if add { "added to" } else { "removed from" };
            say(link, &format!("{} was {what} your list.", target.name)).await;

            // And the list itself, so the marker beside the name appears at once. Each of the four
            // list commands answers with an `AccountList` of its own for the same reason
            // (`UnrankedCommands.cs:491`, `:537`, `:583`, `:630`): enforcement is the server's
            // either way, and without this a player has no way to see who they have blocked.
            send_account_list(link, context, player, list).await;
        }
        Err(hendra_store::StoreError::Refused(why)) => deny(link, why).await,
        Err(_) => deny(link, "try again shortly").await,
    }
}

/// Tells a client who is on one of its lists.
///
/// Both lists go out on arrival and one after every edit, which is what `ConnectManager` does
/// (`ConnectManager.cs:355-368`) and what the four list commands do. Names rather than the
/// original's account ids: this protocol says who somebody is by name everywhere else, and the
/// snapshot carries no account id to match an id against.
async fn send_account_list(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    list: hendra_net::AccountList,
) {
    let kind = match list {
        hendra_net::AccountList::Ignored => hendra_store::ListKind::Ignored,
        hendra_net::AccountList::Locked => hendra_store::ListKind::LockedOut,
    };

    let Ok(listed) = context.store.listed(player.account.id, kind).await else {
        return;
    };

    let mut buf = Vec::new();
    ServerMessage::AccountList {
        list,
        names: listed.into_iter().map(|(_, name)| name).collect(),
    }
    .encode(&mut Writer::new(&mut buf));

    let _ = link.send(Delivery::Stream, &buf).await;
}

/// Moves a player to another player, if the other player allows it.
///
/// Two checks, in two places, because they answer different questions. Whether the other player has
/// locked this one out is an account question, answered here. Whether the move itself is allowed,
/// which is the world's rules about cooldowns, invisibility and where teleporting is permitted at
/// all, is the world's, and answered there.
async fn teleport(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    to: &str,
) {
    if let Ok(target) = context.store.account_by_name(to).await
        && context
            .store
            .is_listed(
                target.id,
                player.account.id,
                hendra_store::ListKind::LockedOut,
            )
            .await
            .unwrap_or(false)
    {
        // Said as though they were not there. Telling somebody they have been locked out is telling
        // them to come back on another account.
        return deny(link, &format!("Unable to find player: {to}")).await;
    }

    let (reply, answer) = tokio::sync::oneshot::channel();
    if !placement
        .world
        .send(ToWorld::Teleport {
            handle: placement.handle,
            to: to.to_string(),
            reply,
        })
        .await
    {
        return;
    }

    if let Ok(Some(why)) = answer.await {
        deny(link, &why).await;
    }
}

/// Buys one of the things prestige buys.
///
/// The offer is named by its place in the list rather than by item, so a client cannot ask for
/// something expensive at a cheap price. Paid first: a purchase that cannot be delivered leaves the
/// prestige alone, and the reverse order would need a refund that can itself fail.
async fn buy_with_prestige(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    offer: u8,
) {
    let Some((name, price)) = hendra_sim::shop::PRESTIGE_OFFERS.get(offer as usize) else {
        return deny(link, "there is no such offer").await;
    };

    let Some(item) = context
        .catalog
        .type_of(name)
        .and_then(|kind| context.catalog.object(kind))
    else {
        return deny(link, "that is not for sale here").await;
    };

    if let Err(err) = context
        .store
        .spend_prestige(player.account.id, *price)
        .await
    {
        return match err {
            hendra_store::StoreError::Refused(why) => deny(link, why).await,
            _ => deny(link, "try again shortly").await,
        };
    }

    // The vault rather than the inventory, as the original does: it goes to the gift chest, which
    // is where something bought outside the world arrives.
    match context.store.add_gift(player.account.id, item.uuid).await {
        Ok(_) => say(link, &format!("{name} was sent to your gift chest.")).await,
        Err(err) => {
            // Paid for and not delivered is the one outcome worth shouting about, because the
            // player is now owed something the server cannot hand over.
            tracing::error!(
                %err,
                account = player.account.id,
                %name,
                "prestige was spent and the item could not be delivered"
            );
            deny(link, "try again shortly").await;
        }
    }
}

/// Whispers to a named player.
///
/// Split out because two things reach it: `/tell` and the older bare form the chat reader still
/// understands. Somebody who has ignored you does not hear you, and is answered as though they had.
async fn tell(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    to: &str,
    text: &str,
) {
    if muted(context, player).await {
        return deny(link, "Muted. You can not tell at this time.").await;
    }

    if to.eq_ignore_ascii_case(&player.character.name) {
        return deny(link, "Quit telling yourself!").await;
    }

    // Whispering to somebody who is not on the server is answered rather than swallowed, which is
    // the difference between a typo and a silence.
    if context.trades.world_of(to).is_none() {
        return deny(link, &format!("{to} not found.")).await;
    }

    if let Ok(target) = context.store.account_by_name(to).await
        && context
            .store
            .is_listed(
                target.id,
                player.account.id,
                hendra_store::ListKind::Ignored,
            )
            .await
            .unwrap_or(false)
    {
        return;
    }

    placement
        .world
        .send(ToWorld::Tell {
            to: to.to_string(),
            from: player.character.name.clone(),
            text: text.to_string(),
        })
        .await;
}

/// Says something to everybody in the caller's guild, wherever they are.
///
/// Guild chat crosses worlds, which is what makes it different from saying something aloud, so it
/// goes through the roster rather than through any one world.
async fn guild_say(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    text: &str,
) {
    if muted(context, player).await {
        return deny(link, "Muted. You can not guild chat at this time.").await;
    }

    let Ok(Some((guild, _))) = context.store.guild_of(player.account.id).await else {
        return deny(link, "You need to be in a guild to guild chat.").await;
    };

    let Ok(members) = context.store.guild_members(guild).await else {
        return deny(link, "try again shortly").await;
    };

    let line = ServerMessage::Chat {
        speaker: hendra_net::EntityId(0),
        from: &format!("{} [guild]", player.character.name),
        text,
    };

    for member in members {
        context.trades.send(&member.name, &line);
    }
}

/// Who is in the caller's guild.
async fn guild_who(link: &mut Link, context: &Context, player: &crate::accounts::Session) {
    let Ok(Some((guild, _))) = context.store.guild_of(player.account.id).await else {
        return deny(link, "You are not in a guild!").await;
    };

    let Ok(members) = context.store.guild_members(guild).await else {
        return deny(link, "try again shortly").await;
    };

    // A heading and then one line per server, of which there is one here: the original groups the
    // answer by the server each member is playing on.
    say(link, "Guild members online:").await;

    let online: Vec<&str> = members
        .iter()
        .map(|member| member.name.as_str())
        .filter(|name| context.trades.world_of(name).is_some())
        .collect();

    say(link, &format!("[{}]: {}", SERVER_NAME, online.join(", "))).await;
}

/// What this server calls itself when it names itself in an answer.
const SERVER_NAME: &str = "Hendra";

/// Adds somebody to one of the account's lists, or takes them off it.
async fn list_person(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    kind: hendra_store::ListKind,
    name: &str,
    add: bool,
) {
    use hendra_store::ListKind;

    // Listing yourself is answered with a joke rather than a refusal, and a different one for each
    // of the four commands.
    if name.eq_ignore_ascii_case(&player.character.name) {
        return say(
            link,
            match (kind, add) {
                (ListKind::Ignored, true) => "Can't ignore yourself!",
                (ListKind::Ignored, false) => "You are no longer ignoring yourself. Good job.",
                (_, true) => "Can't lock yourself!",
                (_, false) => "You are no longer locking yourself. Nice!",
            },
        )
        .await;
    }

    let Ok(target) = context.store.account_by_name(name).await else {
        return deny(link, "Player not found.").await;
    };

    let outcome = if add {
        context
            .store
            .add_to_list(player.account.id, target.id, kind)
            .await
    } else {
        context
            .store
            .remove_from_list(player.account.id, target.id, kind)
            .await
    };

    match outcome {
        Ok(()) => {
            let said = match (kind, add) {
                (ListKind::Ignored, true) => format!("{name} has been added to your ignore list."),
                (ListKind::Ignored, false) => format!("{name} no longer ignored."),
                (_, true) => format!("{name} has been locked."),
                (_, false) => format!("{name} no longer locked."),
            };
            say(link, &said).await;

            // And the list itself, as each of the four commands answers with an `AccountList` of
            // its own (`UnrankedCommands.cs:491`, `:537`, `:583`, `:630`). The line says what
            // happened; this is what puts the marker beside the name.
            let which = match kind {
                ListKind::Ignored => hendra_net::AccountList::Ignored,
                _ => hendra_net::AccountList::Locked,
            };
            send_account_list(link, context, player, which).await;
        }
        Err(hendra_store::StoreError::Refused(why)) => deny(link, why).await,
        Err(_) => deny(link, "Player not found.").await,
    }
}

/// What the caller has listed on the market.
async fn my_market(link: &mut Link, context: &Context, player: &crate::accounts::Session) {
    let listings = context
        .store
        .listings_of(player.account.id)
        .await
        .unwrap_or_default();

    if listings.is_empty() {
        return deny(link, "You have no items currently listed on the market.").await;
    }

    say(
        link,
        &format!("Your items ({}): (format: [id] Name, fame)", listings.len()),
    )
    .await;

    for listing in listings {
        // `Items[shopItem.ItemId].DisplayName`, which is the display id where there is one and the
        // plain id otherwise.
        let name = context
            .catalog
            .type_of_uuid(listing.item)
            .and_then(|kind| context.catalog.object(kind))
            .map(|desc| desc.display_id.as_deref().unwrap_or(&desc.id))
            .unwrap_or("something");

        say(
            link,
            &format!("[{}] {}, {}", listing.id, name, listing.price),
        )
        .await;
    }
}

/// Tells a player something the server knows.
async fn report(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    what: crate::commands::Report,
) {
    use crate::commands::Report;

    match what {
        Report::World => {
            let (reply, answer) = tokio::sync::oneshot::channel();
            placement.world.send(ToWorld::Who { reply }).await;
            let here = answer.await.unwrap_or_default();

            // `[{Owner.Id}] {DisplayName} ({Players.Count} players)`. A world here is named rather
            // than numbered, so the name stands where the original puts its id.
            say(
                link,
                &format!(
                    "[{}] {} ({} players)",
                    placement.world.name,
                    placement.world.name,
                    here.len()
                ),
            )
            .await;
        }

        Report::Quest => {
            let (reply, answer) = tokio::sync::oneshot::channel();
            placement
                .world
                .send(ToWorld::Quest {
                    handle: placement.handle,
                    reply,
                })
                .await;

            match answer.await {
                Ok(Some((_, x, y))) => {
                    say(link, &format!("Quest location: ({x}, {y})")).await;
                }
                _ => deny(link, "Player does not have a quest!").await,
            }
        }

        Report::Position => {
            let (reply, answer) = tokio::sync::oneshot::channel();
            placement
                .world
                .send(ToWorld::Where {
                    handle: placement.handle,
                    reply,
                })
                .await;

            match answer.await {
                Ok(Some((x, y))) => say(link, &format!("Current Position: {x}, {y}")).await,
                _ => say(link, "Current Position: 0, 0").await,
            }
        }

        Report::Who => {
            let (reply, answer) = tokio::sync::oneshot::channel();
            placement.world.send(ToWorld::Who { reply }).await;
            let names = answer.await.unwrap_or_default();

            say(
                link,
                &format!(
                    "Players in current area ({}): {}",
                    names.len(),
                    names.join(", ")
                ),
            )
            .await;
        }

        Report::Online => {
            let names = context.trades.present();
            say(
                link,
                &format!("Players online ({}): {}", names.len(), names.join(", ")),
            )
            .await;
        }

        Report::Uptime => {
            // Hours within the day rather than hours in total, which is what `TimeSpan.Hours`
            // answers and what the original prints.
            let up = context.started.elapsed().as_secs();
            let (hours, minutes, seconds) = (up / 3600 % 24, up / 60 % 60, up % 60);
            say(
                link,
                &format!("The server has been up for {hours:02}h:{minutes:02}m:{seconds:02}s."),
            )
            .await;
        }

        Report::Commands => {
            let rank = context
                .store
                .account(player.account.id)
                .await
                .map(|account| hendra_store::Admin::from_number(account.admin_rank))
                .unwrap_or(hendra_store::Admin::NONE);

            // One line, sorted by name, of what they may use and what the help lists. Only what
            // they may actually use, as the original filters it.
            let mut names: Vec<&str> = crate::commands::ALL
                .iter()
                .filter(|command| command.listed && command.allows(rank))
                .map(|command| command.name)
                .collect();

            // Letter by letter with capitals ignored, which is how `String.CompareTo` orders them
            // there: `Set` sits between `rmarket` and `setfame` rather than ahead of everything.
            names.sort_unstable_by_key(|name| name.to_lowercase());

            say(link, &format!("Available commands: {}", names.join(", "))).await;
        }

        Report::Prestige => match context.store.prestige_of(player.account.id).await {
            Ok((held, _)) => {
                say(link, &format!("{held} is your prestiege number.")).await;
            }
            Err(_) => say(link, "0 is your prestiege number.").await,
        },

        Report::LeftToMax => {
            let Some(class) = context
                .catalog
                .type_of_uuid(player.character.class)
                .and_then(|kind| {
                    context
                        .catalog
                        .classes()
                        .iter()
                        .find(|class| class.object_type == kind)
                })
            else {
                return deny(link, "nothing to say about that class").await;
            };

            let (reply, answer) = tokio::sync::oneshot::channel();
            placement
                .world
                .send(ToWorld::Stats {
                    handle: placement.handle,
                    reply,
                })
                .await;

            let Ok(Some(base)) = answer.await else {
                return deny(link, "try again shortly").await;
            };

            // One line each, all eight, whether or not there is anything left: the original prints
            // `HP: 0` at maximum rather than saying nothing.
            for (index, name) in STAT_NAMES.iter().enumerate() {
                let ceiling = class.stats.get(index).map(|stat| stat.maximum).unwrap_or(0);
                say(link, &format!("{name}: {}", ceiling - base[index])).await;
            }
        }

        // The world's own name for what is playing, which is what it told the client to fetch. The
        // artist and title the original reads out of the mp3's tags are not read here, so it falls
        // back to the same "Unknown" the original does for a file that carries none.
        Report::CurrentSong => {
            let music = playing_in(&placement.world);
            say(
                link,
                &format!("Current Song: {music} by Unknown ({music}.mp3)."),
            )
            .await;
        }

        // A joke rather than a fact, and there is nothing behind it to answer with instead.
        Report::Time => say(link, "Time for you to get a watch!!").await,
    }
}

/// What each of the eight stats is called, in the order the content lists them.
const STAT_NAMES: [&str; 8] = [
    "HP",
    "MP",
    "Attack",
    "Defense",
    "Speed",
    "Dexterity",
    "Vitality",
    "Wisdom",
];

/// Something a moderator or administrator does to somebody.
async fn moderate(
    link: &mut Link,
    context: &Context,
    what: crate::commands::Moderation,
    name: &str,
    rest: &str,
) {
    use crate::commands::Moderation;

    // An announcement names nobody, so it is answered before anybody is looked up. Silent to
    // whoever sent it, as `Chat.Announce` is: they hear it with everybody else.
    if what == Moderation::Announce {
        let line = ServerMessage::Chat {
            speaker: hendra_net::EntityId(0),
            from: "#Oryx the Mad God",
            text: rest,
        };
        for who in context.trades.present() {
            context.trades.send(&who, &line);
        }
        return;
    }

    let Ok(target) = context.store.account_by_name(name).await else {
        // Each of these has its own way of saying nobody is called that.
        let refused = match what {
            Moderation::Kick => format!("Player '{name}' could not be found!"),
            Moderation::Ban | Moderation::BanAddress => "Account not found...".to_string(),
            Moderation::Unban => "Account doesn't exist...".to_string(),
            Moderation::Rename | Moderation::Unname => "Player account not found!".to_string(),
            _ => "Account not found!".to_string(),
        };

        return deny(link, &refused).await;
    };

    // An address ban is an account ban and its address, and the address is the account's.
    if what == Moderation::BanAddress {
        let _ = context.store.set_banned(target.id, true).await;
        context.trades.kick(&target.name);

        return match context.store.ban_address(&target.name).await {
            Ok(()) | Err(hendra_store::StoreError::Refused(_)) => {
                say(
                    link,
                    &format!("Banned {} (both account and ip).", target.name),
                )
                .await
            }
            Err(_) => deny(link, "Failed to ip ban player. IP not logged...").await,
        };
    }

    let outcome = match what {
        Moderation::Mute | Moderation::Unmute => {
            // `if (acc.Admin) { "Cannot mute other admins." }` (`RankedCommands.cs:1429-1433`),
            // checked before anything is written. Only a mute is refused: the original's `/unmute`
            // makes no such check, and lifting a mute from somebody who cannot be muted does
            // nothing anyway.
            if what == Moderation::Mute && target.admin_rank >= MUTE_EXEMPT_RANK {
                return deny(link, "Cannot mute other admins.").await;
            }

            // Minutes when a number was given, and until somebody lifts it when none was, which is
            // the original's default rather than ours.
            let minutes = rest.trim().parse::<i64>().ok();
            let until = (what == Moderation::Mute).then(|| {
                let minutes = minutes
                    .unwrap_or(FOREVER_IN_MINUTES)
                    .clamp(1, FOREVER_IN_MINUTES);
                chrono::Utc::now() + chrono::Duration::minutes(minutes)
            });

            context
                .store
                .mute(target.id, until)
                .await
                .map(|()| match minutes {
                    _ if what == Moderation::Unmute => format!("{name} successfully unmuted."),
                    Some(minutes) => format!("{name} successfully muted for {minutes} minutes."),
                    None => format!("{name} successfully muted indefinitely."),
                })
        }

        Moderation::Ban | Moderation::Unban => {
            let banned = what == Moderation::Ban;
            context.store.set_banned(target.id, banned).await.map(|()| {
                if banned {
                    context.trades.kick(&target.name);
                    format!("{} successfully banned.", target.name)
                } else {
                    format!("Success! {}'s account no longer banned.", target.name)
                }
            })
        }

        Moderation::Kick => {
            // Ending the connection is the session's, and the roster is what can reach it.
            context.trades.kick(&target.name);
            Ok("Player disconnected!".to_string())
        }

        Moderation::Rank => {
            // A number, as the original takes one: eighty and over is what makes an administrator.
            let asked = rest.trim().parse::<i16>().unwrap_or(0);
            let rank = hendra_store::Admin::from_number(asked);

            context
                .store
                .set_admin_rank(target.id, rank)
                .await
                .map(|()| {
                    let admin = if asked >= 80 {
                        " and now has admin status"
                    } else {
                        ""
                    };
                    format!("{} given legacy rank {asked}{admin}.", target.name)
                })
        }

        Moderation::SetFame
        | Moderation::SetGold
        | Moderation::SetPrestige
        | Moderation::SetStar => {
            let Ok(amount) = rest.trim().parse::<i32>() else {
                return deny(link, crate::commands::THREW).await;
            };

            // Stars are earned fame, so setting them is setting that: there is no second number.
            // The two the original refuses outright are the ones outside nought to seventy.
            if what == Moderation::SetStar && !(0..=70).contains(&amount) {
                return;
            }

            let currency = match what {
                Moderation::SetGold => hendra_store::Currency::Gold,
                Moderation::SetPrestige => hendra_store::Currency::Prestige,
                _ => hendra_store::Currency::Fame,
            };

            context
                .store
                .set_currency(target.id, currency, amount)
                .await
                .map(|()| "Success!".to_string())
        }

        Moderation::Gift => {
            let Some(item) = context
                .catalog
                .type_of(rest.trim())
                .and_then(|kind| context.catalog.object(kind))
            else {
                return deny(link, "Unknown item type!").await;
            };

            context.store.add_gift(target.id, item.uuid).await.map(|_| {
                // Their chest changed and they did not do it, so their session is told to
                // re-read rather than finding out on their next visit.
                context.trades.refresh(&target.name);
                format!("You gifted {} one {}.", target.name, item.id)
            })
        }

        // Between three and fifteen letters and nothing else, which is the name the original
        // accepts and the reason it refuses one.
        Moderation::Rename => {
            let wanted = rest.trim();
            if wanted.len() < 3 || wanted.len() > 15 || !wanted.chars().all(char::is_alphabetic) {
                return say(
                    link,
                    "New name is invalid. Must be between 3-15 char long and contain only letters.",
                )
                .await;
            }

            context
                .store
                .rename_account(target.id, wanted)
                .await
                .map(|()| "Rename successful.".to_string())
        }

        Moderation::Unname => context
            .store
            .rename_account(target.id, &format!("Player{}", target.id))
            .await
            .map(|()| "Account succesfully unnamed.".to_string()),

        Moderation::BanAddress => unreachable!("answered above"),
        Moderation::Announce => unreachable!("answered above"),
    };

    match outcome {
        Ok(said) => say(link, &said).await,
        Err(hendra_store::StoreError::Refused(why)) => deny(link, why).await,
        Err(hendra_store::StoreError::NameTaken) => deny(link, "Name already taken").await,
        Err(err) => {
            tracing::warn!(%err, "a moderation command failed");
            deny(link, crate::commands::THREW).await;
        }
    }
}

/// How long a mute with no end lasts.
///
/// The original mutes an address until somebody lifts it; a mute here has to end, so it ends in a
/// month, which is longer than any argument `/mute` accepts.
const FOREVER_IN_MINUTES: i64 = 60 * 24 * 30;

/// Moves a player into a world that has already been chosen.
///
/// The other half of `go_to`, for the worlds whose instance is decided by something other than a
/// name: a guild hall belongs to a guild rather than to whoever asked for it.
async fn enter(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    from: &Placement,
    name: &str,
    world: WorldHandle,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<Placement> {
    // Through the same handover a portal uses. A hall is a door like any other, and one that built
    // its body from the login row would take a levelled character back to whatever it was then.
    let arrival = handover(context, player, from).await;
    let handle = join(link, &world, name, arrival, orders, died).await?;

    from.world
        .send(ToWorld::Leave {
            handle: from.handle,
        })
        .await;

    Some(Placement { world, handle })
}

/// Buys a larger hall for the caller's guild.
///
/// Paid from the guild's fame rather than the buyer's, because the hall belongs to the guild. Only
/// an officer or above may, for the same reason: a hall is something the guild owns and a member
/// spending its fame is a member spending everyone's.
async fn buy_hall_upgrade(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    upgrade: crate::world_task::HallUpgrade,
) {
    use hendra_store::Rank;

    let Ok(Some((guild, rank))) = context.store.guild_of(player.account.id).await else {
        return deny(link, "you are not in a guild").await;
    };

    if rank < Rank::Officer {
        return deny(link, "insufficient privileges").await;
    }

    // Paid first, and only then raised. A purchase that cannot be applied leaves the fame alone,
    // and the reverse order would need a refund that can itself fail.
    if let Err(err) = context.store.spend_guild_fame(guild, upgrade.price).await {
        return match err {
            hendra_store::StoreError::Refused(why) => deny(link, why).await,
            _ => deny(link, "try again shortly").await,
        };
    }

    match context.store.raise_guild_level(guild, upgrade.level).await {
        Ok(()) => {
            say(
                link,
                "Your hall has been upgraded. It will be larger next time you enter.",
            )
            .await;
        }
        Err(err) => {
            // Paid for and not applied is worth shouting about: the guild is now owed a hall.
            tracing::error!(%err, guild, "guild fame was spent and the hall was not raised");
            deny(link, "try again shortly").await;
        }
    }
}

/// Carries out an administrator's tool.
///
/// Split by where the answer lives: some are the world's, some are the store's, and one is both.
async fn wield(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    what: crate::commands::Wielded,
    rest: &str,
) {
    use crate::commands::Wielded;
    use crate::world_task::Wielding;

    let rank = match context.store.account(player.account.id).await {
        Ok(account) => hendra_store::Admin::from_number(account.admin_rank),
        Err(_) => hendra_store::Admin::NONE,
    };

    // The world's, and each answers with a line to repeat, or with nothing where the original says
    // nothing.
    let in_world = match what {
        Wielded::Spawn | Wielded::LootSpawn => {
            // "5 Slime", or "Slime" for one, from SpawnCommand.cs. The count leads rather than
            // trails, and that is not arbitrary: half the game's bosses end in a numeral, so a
            // trailing count turns "Oryx the Mad God 2" into two of Oryx the Mad God.
            let (count, name) = match rest.split_once(' ') {
                Some((head, tail)) if !head.is_empty() => match head.parse::<i64>() {
                    Ok(count) => (count, tail),
                    Err(_) => (1, rest),
                },
                _ => (1, rest),
            };

            if count <= 0 {
                return say(
                    link,
                    &format!("Really? {count} {name}? I'll get right on that..."),
                )
                .await;
            }

            Some(Wielding::Spawn {
                name: name.to_string(),
                count: count as usize,
                // What separates the two commands: what `/spawn` puts down is flagged as summoned,
                // which is what makes it give no experience and drop no loot, and its enemies are
                // additionally hidden for good (`RankedCommands.cs:561-583`).
                summoned: what == Wielded::Spawn,
            })
        }
        Wielded::KillAll => Some(Wielding::KillAll {
            name: rest.to_string(),
        }),
        Wielded::Size => {
            // The range is the caller's rank: everybody else is held between three quarters and a
            // quarter over, and an owner may do anything from nothing to five times.
            let (least, most) = if rank.meets(hendra_store::Admin::OWNER) {
                (0, 500)
            } else {
                (75, 125)
            };

            let size = number_typed(rest);
            if (size < least && size != 0) || size > most {
                return say(
                    link,
                    &format!(
                        "Invalid size. Size needs to be within the range: {least}-{most}. Use 0 \
                         to reset size to default."
                    ),
                )
                .await;
            }

            // Zero restores the sprite's own size, which for a player is a hundred percent.
            let percent = if size == 0 { 100 } else { size as u16 };
            Some(Wielding::Size { percent })
        }
        Wielded::Hide => Some(Wielding::Hide),
        Wielded::Godlands => Some(Wielding::Godlands),
        Wielded::MaxLevel => Some(Wielding::MaxLevel),
        Wielded::MaxStats => Some(Wielding::MaxStats),
        Wielded::CloseRealm => Some(Wielding::CloseRealm),
        Wielded::Pause => Some(Wielding::Pause),
        Wielded::Effect => Some(Wielding::Effect {
            name: rest.to_string(),
        }),
        // `Utils.FromString`, which reads a decimal or an 0x colour and answers zero for anything
        // else, so a colour nobody recognises is black rather than a refusal.
        Wielded::Glow => Some(Wielding::Glow {
            colour: colour_named(rest.trim()).unwrap_or_else(|| number_typed(rest)),
        }),
        Wielded::KillPlayer => Some(Wielding::KillPlayer {
            name: rest.to_string(),
        }),
        Wielded::Summon => Some(Wielding::Summon {
            name: rest.to_string(),
        }),
        Wielded::SummonAll => Some(Wielding::SummonAll),
        Wielded::Spectate => Some(Wielding::Spectate {
            name: rest.to_string(),
        }),
        Wielded::Warg => Some(Wielding::Warg {
            name: rest.to_string(),
        }),
        Wielded::ToQuest => Some(Wielding::ToQuest),
        Wielded::Setpiece => Some(Wielding::Setpiece {
            name: rest.to_string(),
        }),
        Wielded::ClearSpawn => Some(Wielding::ClearSpawn),
        Wielded::ClearGraves => Some(Wielding::ClearGraves),
        Wielded::Debug => Some(Wielding::Debug),
        _ => None,
    };

    if let Some(asked) = in_world {
        let (reply, answer) = tokio::sync::oneshot::channel();
        if placement
            .world
            .send(ToWorld::Wield {
                handle: placement.handle,
                what: asked,
                reply,
            })
            .await
            && let Ok(said) = answer.await
            && !said.is_empty()
        {
            // What is being put down is announced over the caller's own head rather than said to
            // anybody: `NotifySpawn` (`RankedCommands.cs:250`, `:510`) broadcasts a red
            // `Notification` and adds a chat line only for a spectated body, which nothing here is.
            // The world has already queued the float, so there is nothing left to say.
            if matches!(what, Wielded::Spawn | Wielded::LootSpawn) && said.starts_with("Spawning") {
                return;
            }

            say(link, &said).await;
        }
        return;
    }

    // The rest are the store's, or the registry's.
    match what {
        Wielded::Give => {
            let Some(item) = context
                .catalog
                .type_of(rest.trim())
                .and_then(|kind| context.catalog.object(kind))
            else {
                return deny(link, "Unknown item type!").await;
            };

            if item.item.is_none() {
                return deny(link, "Not an item!").await;
            }

            // The dozen names an account under ninety may not ask for, from `GimmeCommand`.
            if !rank.meets(hendra_store::Admin::CONTENT)
                && FORBIDDEN_GIFTS.contains(&item.id.as_str())
            {
                return deny(link, "Insufficient rank for that item.").await;
            }

            match context
                .store
                .give_item(
                    player.character.id,
                    item.uuid,
                    EQUIPPED_SLOTS as i16,
                    last_carried_slot(&context.store, player.character.id).await,
                )
                .await
            {
                // Silent, as the original is: the item appearing in the pack is the answer.
                Ok(_) => send_containers(link, &context.catalog, &context.store, player).await,
                Err(_) => deny(link, "Not enough space in inventory!").await,
            }
        }

        Wielded::ClearPack => {
            let Ok(character) = context.store.character(player.character.id).await else {
                return say(link, "Inventory Cleared.").await;
            };

            // The eight carried slots, which is what `ClearInvCommand` empties: what is worn stays
            // worn.
            for (slot, item) in character.inventory {
                if slot < EQUIPPED_SLOTS as i16 {
                    continue;
                }
                let _ = context
                    .store
                    .take_item(player.character.id, slot, item)
                    .await;
            }

            send_containers(link, &context.catalog, &context.store, player).await;
            say(link, "Inventory Cleared.").await;
        }

        Wielded::Quake => {
            if rest.trim().is_empty() {
                let running = context.worlds.running();
                return say(link, &format!("Valid World Names: {}.", running.join(", "))).await;
            }

            if placement.world.name.as_ref() == crate::commands::NEXUS {
                return deny(link, "Cannot use /quake in Nexus.").await;
            }

            let destination = rest.trim();
            if !context
                .worlds
                .running()
                .iter()
                .any(|name| name.eq_ignore_ascii_case(destination))
            {
                return deny(link, "Invalid world.").await;
            }

            placement
                .world
                .send(ToWorld::SendEveryoneTo {
                    world: destination.to_string(),
                })
                .await;
        }

        Wielded::Visit => {
            let Some(world) = context.trades.world_of(rest.trim()) else {
                return deny(link, "Player not found!").await;
            };
            say(link, &format!("They are in {world}.")).await;
        }

        // Four items into the four empty carried slots, from `GearCommand`, and only when all four
        // are empty.
        Wielded::Gear => {
            gear(link, context, player, rest.trim()).await;
        }

        Wielded::Reskin => {
            // A skin is named by its object type, and only the skins that belong to the class being
            // played are offered or accepted (`RankedCommands.cs:1198-1217`). Without that check any
            // object number at all was accepted and worn, which put a priest in a wizard's robes and
            // a player in a slime.
            let avatar = context
                .catalog
                .type_of_uuid(player.character.class)
                .unwrap_or(hendra_content::ObjectType::NONE);

            let mine: Vec<&hendra_content::SkinDesc> = context
                .catalog
                .skins()
                .iter()
                .filter(|skin| skin.class == avatar)
                .collect();

            if rest.is_empty() {
                deny(link, "Usage: /reskin <positive integer>").await;
                let choices: Vec<String> = mine
                    .iter()
                    .map(|skin| skin.object_type.0.to_string())
                    .collect();
                return say(link, &format!("Choices: {}", choices.join(", "))).await;
            }

            let asked = number_typed(rest) as u16;
            let chosen = mine
                .iter()
                .find(|skin| skin.object_type == hendra_content::ObjectType(asked));

            if asked != 0 && chosen.is_none() {
                return deny(
                    link,
                    "Error setting skin. Either the skin type doesn't exist or the skin is for \
                     another class.",
                )
                .await;
            }

            // A skin the content reserves for one character is not refused to anybody else: they
            // are put back into no skin at all, which is what the original does.
            let worn = match chosen {
                Some(skin) => match skin.player_exclusive.as_deref() {
                    Some(only) if only != player.character.name => 0,
                    _ => asked,
                },
                None => 0,
            };

            // Granted and then worn, rather than worn without owning it: the ownership check is
            // what stops a skin being worn by somebody who has not bought it, and an administrator
            // reaching past it would be a second door into the wardrobe.
            let identity = context
                .catalog
                .object(hendra_content::ObjectType(worn))
                .map(|desc| desc.uuid)
                .unwrap_or_default();

            if worn != 0
                && context
                    .store
                    .grant_skin(player.account.id, identity)
                    .await
                    .is_err()
            {
                return;
            }

            if let Err(why) = context
                .store
                .wear_skin(player.account.id, player.character.id, worn as i32, identity)
                .await
            {
                return deny(link, &why.to_string()).await;
            }

            // And onto the body standing here, not only into the row. `ReskinHandler` puts the skin
            // and the size it brings straight onto the player it is holding
            // (`ReskinHandler.cs:63-64`), which is what makes the wardrobe show its work: a change
            // that waited for the next world would be one nobody could see themselves make.
            let size = chosen
                .map(|skin| skin.size.clamp(0, u16::MAX as i32) as u16)
                .filter(|_| worn != 0)
                .unwrap_or(100);

            let (reply, answer) = tokio::sync::oneshot::channel();
            placement
                .world
                .send(ToWorld::Wield {
                    handle: placement.handle,
                    what: Wielding::Skin { skin: worn, size },
                    reply,
                })
                .await;
            let _ = answer.await;
        }

        Wielded::Music => {
            // Nothing named lists what there is, as `MusicCommand` does with `MusicNames`
            // (`RankedCommands.cs:1778-1784`).
            if rest.trim().is_empty() {
                return say(link, &format!("Music Choices: {}.", context.music.join(", "))).await;
            }

            // Matched without regard to case and answered with the name as the file spells it,
            // which is what `properName` is for: the client asks the app server for
            // `<name>.mp3`, so a name in the wrong case is a name that fetches nothing.
            let asked = rest.trim();
            let chosen = match context
                .music
                .iter()
                .find(|name| name.eq_ignore_ascii_case(asked))
            {
                Some(found) => found.clone(),

                // A deployment with no music directory has nothing to check against, so anything
                // is taken rather than everything refused.
                None if context.music.is_empty() => asked.to_string(),
                None => return deny(link, &format!("Music \"{asked}\" not found!")).await,
            };

            // The world's own name for what is playing, so that somebody who arrives afterwards is
            // welcomed with this rather than with what the definition named.
            if let Ok(mut playing) = placement.world.music.write() {
                *playing = chosen.clone();
            }

            let line = format!("World music changed to {chosen}.");
            let (reply, answer) = tokio::sync::oneshot::channel();
            placement.world.send(ToWorld::Who { reply }).await;

            // Both, to everybody standing here: the line they read and the track they hear.
            // `MusicCommand` sends the first to every player and then queues the second for each
            // of them (`RankedCommands.cs:1798-1815`).
            for who in answer.await.unwrap_or_default() {
                context.trades.send(
                    &who,
                    &ServerMessage::Chat {
                        speaker: hendra_net::EntityId(0),
                        from: "",
                        text: &line,
                    },
                );
                context
                    .trades
                    .send(&who, &ServerMessage::SwitchMusic { music: &chosen });
            }
        }

        Wielded::Override => {
            // Acting as another account means holding two identities on one connection, and every
            // durable write here names the account it belongs to. There is no safe way to do it
            // that is not just logging in as them, so the account it names is never found.
            deny(link, "Account not found!").await;
        }

        Wielded::RemoveOverride => deny(link, "Account isn't overridden.").await,

        Wielded::Link => {
            // A world here is reachable by the name its definition gives it, decided at load rather
            // than at runtime, so a link to this one already exists.
            deny(link, "Link already exists.").await;
        }

        Wielded::Unlink => {
            // The anomaly is kept: the command asks for rank eight and then asks again for eighty,
            // so an account between the two may type it and may not use it.
            if !rank.meets(hendra_store::Admin::MODERATOR) {
                return deny(link, "Forbidden.").await;
            }
            deny(link, "Link not found.").await;
        }

        Wielded::OryxSay => {
            let line = rest.to_string();
            let (reply, answer) = tokio::sync::oneshot::channel();
            placement.world.send(ToWorld::Who { reply }).await;

            for who in answer.await.unwrap_or_default() {
                context.trades.send(
                    &who,
                    &ServerMessage::Chat {
                        speaker: hendra_net::EntityId(0),
                        from: "#Oryx the Mad God",
                        text: &line,
                    },
                );
            }
        }

        Wielded::Reboot => {
            tracing::warn!(
                account = player.account.id,
                "an administrator asked for a stop"
            );

            if rest.is_empty() {
                say(
                    link,
                    "Usage: /reboot < server name | $all | $wserver | $account >",
                )
                .await;
                return say(link, "Current servers available for rebooting:\n").await;
            }
            deny(link, "Server not found.").await;
        }

        // The original's runtime has a large object heap and this one does not, so the command
        // exists, is silent, and does nothing — which is what it looks like there too.
        Wielded::CompactLoh => {}

        _ => {}
    }
}

/// Whether this account may not speak at the moment.
///
/// Read fresh rather than from the session's copy, so a mute applied while somebody is playing takes
/// effect without waiting for them to reconnect.
async fn muted(context: &Context, player: &crate::accounts::Session) -> bool {
    let Ok(account) = context.store.account(player.account.id).await else {
        return false;
    };

    mute_applies(account.admin_rank, account.muted_until, chrono::Utc::now())
}

/// The items `/gimme` refuses to anybody under rank ninety.
///
/// The list `GimmeCommand` carries, by the name each item is displayed under.
const FORBIDDEN_GIFTS: [&str; 14] = [
    "Boshy Gun",
    "Boshy Shotgun",
    "Admin Sword",
    "Admin Wand",
    "Admin Bow",
    "Admin Katana",
    "Admin Dagger",
    "Admin Staff",
    "Crown",
    "Lost Halls Key",
    "Gold Medal",
    "Ent World Key",
    "Strike Amulet",
    "Oryx's Arena Key",
];

/// Reads a number the way `Utils.FromString` reads one.
///
/// A decimal, or `0x` and hexadecimal, and zero for anything else: the original never refuses a
/// number it cannot read, it treats it as nothing. `/size nonsense` is `/size 0` there and here.
fn number_typed(text: &str) -> i32 {
    let text = text.trim();

    if let Some(hex) = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .or_else(|| text.strip_prefix('#'))
    {
        return i32::from_str_radix(hex, 16).unwrap_or(0);
    }

    text.parse::<i32>().unwrap_or(0)
}

/// Gives a class its starting set, from `/Set`.
///
/// Four items into the four carried slots, and only when all four are empty, exactly as
/// `GearCommand` writes them: the weapon, the ability, the armour and a ring every class shares.
async fn gear(link: &mut Link, context: &Context, player: &crate::accounts::Session, class: &str) {
    let Some((proper, kinds)) = CLASS_GEAR
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(class))
    else {
        deny(link, "Usage: /set <ClassName>").await;
        return say(
            link,
            "Classes: Rogue, Archer, Wizard, Priest, Warrior, Knight, Paladin, Assassin, \
             Necromancer, Huntress, Mystic, Trickster, Sorcerer, Ninja",
        )
        .await;
    };

    // Every carried slot has to be empty, and the original checks the four it is about to write
    // rather than the whole pack.
    let Ok(character) = context.store.character(player.character.id).await else {
        return deny(link, "Not enough space").await;
    };

    let wanted: Vec<i16> = (0..4).map(|slot| EQUIPPED_SLOTS as i16 + slot).collect();
    if character
        .inventory
        .iter()
        .any(|(slot, _)| wanted.contains(slot))
    {
        return deny(link, "Not enough space").await;
    }

    for kind in *kinds {
        let Some(item) = context.catalog.object(hendra_content::ObjectType(*kind)) else {
            continue;
        };

        let _ = context
            .store
            .give_item(
                player.character.id,
                item.uuid,
                EQUIPPED_SLOTS as i16,
                last_carried_slot(&context.store, player.character.id).await,
            )
            .await;
    }

    send_containers(link, &context.catalog, &context.store, player).await;
    say(link, &format!("Successfully obtained {proper} class' gear")).await;
}

/// What `/Set` hands each class, by object type, in the order the original writes the four slots.
///
/// The numbers are `GearCommand`'s, and the last of each four is the ring every class is given.
const CLASS_GEAR: [(&str, &[u16; 4]); 14] = [
    ("Rogue", &[0xa19, 0xa59, 0xad3, 0xabd]),
    ("Archer", &[0xa1e, 0xa65, 0xad3, 0xabd]),
    ("Wizard", &[0xa9f, 0xad6, 0xa60, 0xabd]),
    ("Priest", &[0xa07, 0xa33, 0xa60, 0xabd]),
    ("Warrior", &[0xa82, 0xa6a, 0xa13, 0xabd]),
    ("Knight", &[0xa82, 0xa0b, 0xa13, 0xabd]),
    ("Paladin", &[0xa82, 0xa54, 0xa13, 0xabd]),
    ("Assassin", &[0xa19, 0xaa7, 0xad3, 0xabd]),
    ("Necromancer", &[0xa9f, 0xaae, 0xa60, 0xabd]),
    ("Huntress", &[0xa1e, 0xab5, 0xad3, 0xabd]),
    ("Mystic", &[0xa9f, 0xa45, 0xa60, 0xabd]),
    ("Trickster", &[0xa19, 0xb1f, 0xad3, 0xabd]),
    ("Sorcerer", &[0xa07, 0xb31, 0xa60, 0xabd]),
    ("Ninja", &[0x0c4, 0xc57, 0xad3, 0xabd]),
];

/// Lists every copy of a named item in the pack, from `/marketall`.
async fn market_all(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    item: &str,
    price: i32,
) {
    // The command's own check, made before the item name is read at all.
    if placement.world.name.as_ref() != "Marketplace" {
        return deny(link, "Can only market items in Marketplace.").await;
    }

    let Some(wanted) = context
        .catalog
        .type_of(item)
        .and_then(|kind| context.catalog.object(kind))
    else {
        return deny(link, "Unknown item type!").await;
    };

    // Refused by type here, unlike `/market`, where the same check sits commented out in
    // `AddToMarket`. Both oddities are the original's, and they disagree with each other: a
    // soulbound item can be listed one slot at a time and not all at once.
    if wanted.item.as_ref().is_some_and(|item| item.soulbound) {
        return deny(link, "Can't market soulbound items!").await;
    }

    let name = wanted.display_id.as_deref().unwrap_or(&wanted.id);

    let Ok(character) = context.store.character(player.character.id).await else {
        return deny(link, "Errors occurred, couldn't market items.").await;
    };

    let mut holding: Vec<i16> = character
        .inventory
        .iter()
        .filter(|(slot, held)| *slot >= EQUIPPED_SLOTS as i16 && *held == wanted.uuid)
        .map(|(slot, _)| *slot)
        .collect();
    holding.sort_unstable();

    // Each slot listed on its own, counting what went through. Listing eight items and announcing
    // eight when the fifth failed is worse than saying nothing: the seller believes the other three
    // are for sale.
    let mut sold = 0;
    let mut failed = false;
    for slot in holding {
        match list_one(context, player, slot, price).await {
            Ok(()) => sold += 1,
            Err(why) => {
                failed = true;
                deny(link, why).await;
            }
        }
    }

    let plural = if sold > 1 { "s" } else { "" };
    if failed {
        if sold > 0 {
            // `SendErrorFormat`, not `SendInfoFormat`: a partial success is reported as a failure.
            deny(
                link,
                &format!("Errors occurred, only {sold} item{plural} sold."),
            )
            .await;
        } else {
            deny(link, "Errors occurred, couldn't market items.").await;
        }
    } else if sold > 0 {
        let have = if sold > 1 { "ve" } else { "s" };
        say(
            link,
            &format!("Success! Your {sold} item{plural} ha{have} been placed on the market."),
        )
        .await;
    } else {
        deny(link, &format!("No {name} found in your inventory.")).await;
    }

    send_containers(link, &context.catalog, &context.store, player).await;
}

/// Reads a colour the way somebody types one.
fn colour_named(text: &str) -> Option<i32> {
    // A name for the handful worth typing, and a hex number for anything else, which is what an
    // administrator who wants a particular shade will reach for.
    Some(match text.to_ascii_lowercase().as_str() {
        "red" => 0xff_0000,
        "green" => 0x00_ff00,
        "blue" => 0x00_00ff,
        "white" => 0xff_ffff,
        "black" => 0x00_0000,
        "yellow" => 0xff_ff00,
        "purple" => 0xff_00ff,
        "none" | "off" => 0,
        other => i32::from_str_radix(other.trim_start_matches('#'), 16).ok()?,
    })
}

/// Takes a player to their own guild's hall.
///
/// One path, used by the command and by the portal alike: a hall is one room per guild and the
/// level chooses its map, and two doors that decided those differently would be two different rooms
/// with one name.
async fn enter_guild_hall(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    from: &Placement,
    name: &str,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<Placement> {
    let Ok(Some((guild, _))) = context.store.guild_of(player.account.id).await else {
        deny(link, "You need to be in a guild.").await;
        return None;
    };

    let level = context
        .store
        .guild(guild)
        .await
        .map(|guild| guild.level)
        .unwrap_or(0);

    // An empty hall is rebuilt rather than reopened, and only a rebuild reads the level the guild
    // has just paid for (`GuildHall.GetInstance`, `GuildHall.cs:53-66`). Whether anybody is in it is
    // the roster's to answer: it is what already knows where everybody is, and it knows it per room
    // rather than per name.
    let occupied = context
        .trades
        .in_world(&crate::worlds::hall_key(crate::commands::GUILD_HALL, guild))
        > 0;

    let Some(hall) = context.worlds.get_or_start_hall(
        crate::commands::GUILD_HALL,
        guild,
        level,
        occupied,
    ) else {
        deny(link, "your hall is not available").await;
        return None;
    };

    enter(link, context, player, from, name, hall, orders, died).await
}

/// Asks people into the dungeon the caller opened.
///
/// `DungeonInvite` (`UnrankedCommands.cs:204-315`). Two refusals come first and both are about the
/// room rather than about whoever is being asked: it has to be a dungeon the caller opened, and it
/// has to be younger than ninety seconds. After that every name is answered in one of three ways —
/// asked, asked already, or not on the server — and the three are reported as three lines.
async fn dungeon_invite(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    name: &str,
    args: &str,
) {
    use crate::dungeons::Invitation;

    let dungeons = context.worlds.dungeons();
    let room = placement.world.key.to_string();

    if !dungeons.opened_by(&room, name) {
        return deny(link, "This is not your dungeon!").await;
    }
    if dungeons.expired(&room) {
        return deny(link, "It's too late to invite players!").await;
    }

    // Who to ask. `-g` anywhere in the arguments means the caller's guild rather than a list of
    // names, and the original tests it exactly that loosely (`UnrankedCommands.cs:225`).
    let guild_wide = args.contains("-g");
    let candidates: Vec<String> = if guild_wide {
        let Ok(Some((guild, _))) = context.store.guild_of(player.account.id).await else {
            return;
        };
        let Ok(members) = context.store.guild_members(guild).await else {
            return deny(link, "try again shortly").await;
        };

        // Only the ones actually playing: the original walks the connected clients rather than the
        // guild roll, so a guildmate who is offline is not considered at all.
        let mut playing = Vec::new();
        for member in members {
            if context.trades.world_of(&member.name).is_none() {
                continue;
            }
            playing.push(member.name);
        }
        playing
    } else {
        args.split(' ')
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect()
    };

    if candidates.is_empty() {
        return deny(link, "Specify some players to invite!").await;
    }

    let display = crate::worlds::world_of_key(&room).to_string();
    let id = placement.world.id;

    let mut invited: Vec<String> = Vec::new();
    let mut already: Vec<String> = Vec::new();
    let mut missed: Vec<String> = Vec::new();

    for who in candidates {
        if who.eq_ignore_ascii_case(name) {
            continue;
        }

        // Somebody who has put the caller on their ignore list is skipped before anything else in
        // the guild-wide form (`UnrankedCommands.cs:229`), so they are not even reported.
        if guild_wide && ignores(context, &who, player.account.id).await {
            continue;
        }

        // Already in the room. The original reports them as unable to be invited and writes them
        // into `Invited` anyway, so a second `/dinvite -g` does not ask them either.
        if guild_wide && context.trades.room_of(&who).as_deref() == Some(room.as_str()) {
            dungeons.note_present(&room, &who);
            already.push(who);
            continue;
        }

        // A name nothing on the server answers to is "not found": `ChatManager.Invite`
        // (`ChatManager.cs:239-246`) refuses a name it cannot resolve and an account with no play
        // lock, which is an account that is not currently playing.
        if context.trades.world_of(&who).is_none() {
            missed.push(who);
            continue;
        }

        match dungeons.invite(&room, &who) {
            Invitation::Already => already.push(who),
            Invitation::Sent => {
                // Delivered unless they are ignoring the caller. The invitation still counts as
                // sent: the original publishes it and filters on the way out
                // (`ChatManager.cs:309-316`), so the sender is told either way.
                if !ignores(context, &who, player.account.id).await {
                    context.trades.send(
                        &who,
                        &ServerMessage::Chat {
                            speaker: hendra_net::EntityId(0),
                            from: "",
                            text: &format!(
                                "You've been invited by {name} to join them in {display}. \
                                 Send /daccept {id} to join."
                            ),
                        },
                    );
                }
                invited.push(who);
            }
        }
    }

    if !invited.is_empty() {
        say(link, &format!("Invited: {}", invited.join(", "))).await;
    }
    if !already.is_empty() {
        say(link, &format!("Already invited: {}", already.join(", "))).await;
    }
    if !missed.is_empty() {
        say(link, &format!("Not found: {}", missed.join(", "))).await;
    }
}

/// Whether a named player has put an account on their ignore list.
async fn ignores(context: &Context, who: &str, account_id: i64) -> bool {
    let Ok(target) = context.store.account_by_name(who).await else {
        return false;
    };

    context
        .store
        .is_listed(target.id, account_id, hendra_store::ListKind::Ignored)
        .await
        .unwrap_or(false)
}

/// Takes an invitation into a dungeon somebody opened.
///
/// `DungeonAccept` (`UnrankedCommands.cs:149-203`). The number is a world's, not an invitation's,
/// which is why an unknown one is answered with "The world was not found." rather than with
/// anything about invitations.
#[allow(clippy::too_many_arguments)]
async fn dungeon_accept(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    from: &Placement,
    name: &str,
    id: i32,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
) -> Option<Placement> {
    use crate::dungeons::Admission;

    let Some(world) = context.worlds.by_id(id) else {
        deny(link, "The world was not found.").await;
        return None;
    };

    let room = world.key.to_string();
    let display = crate::worlds::world_of_key(&room).to_string();

    match context.worlds.dungeons().accept(&room, name) {
        Admission::Welcome => {}
        Admission::Expired => {
            deny(link, "The invite has expired.").await;
            return None;
        }
        Admission::Already => {
            deny(link, &format!("You have already entered {display}.")).await;
            return None;
        }
        Admission::Uninvited => {
            deny(link, &format!("You were not invited to join {display}.")).await;
            return None;
        }
    }

    enter(link, context, player, from, name, world, orders, died).await
}

/// What a death left behind.
enum AfterDeath {
    /// The character is gone and so is the session.
    Over,

    /// They lived, and are standing somewhere else now.
    SentHome(Placement),

    /// They lived and there was nowhere to put them, which leaves them in no world at all.
    Nowhere,
}

/// Answers a death.
///
/// Follows `Player.Death` (`Player.cs:995`), whose checks run in the order they are written and
/// each of which can stop the rest. Three of them end in `ReconnectToNexus` rather than in a
/// character's death, and which one answers first is the whole of what makes a death permanent.
///
/// The character is written down as dead *before* anything is sent, because that is the half that
/// must not be lost: a death message the player sees and a character the database still calls alive
/// is a character they can log back into.
#[allow(clippy::too_many_arguments)]
async fn die(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    name: &str,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
    departed: crate::world_task::Departed,
) -> AfterDeath {
    // `Rekted` (`Player.cs:880`) and `NonPermaKillEnemy` (`Player.cs:862`), which the world has
    // already folded into one flag: a death nobody claimed, or one dealt by something summoned.
    // Both leave a stone that says they got rekt and put them back in the nexus, alive.
    //
    // The first of these is the original's oddity worth keeping: `Player.Tick`'s sweep passes
    // `rekt: true` (`Player.cs:586`), so any death that no blow claimed for itself is a trip home
    // rather than the end of a character.
    if departed.rekt {
        gravestone(context, player, placement, &departed, true).await;
        return send_home(
            link,
            context,
            player,
            placement,
            name,
            orders,
            died,
            &departed.vitals,
        )
        .await;
    }

    // `Resurrection` (`Player.cs:910`) — before the nexus check, which is the order the original
    // uses: an amulet worn into a safe room is spent by a death there even though the death was
    // never going to take anything.
    if let Some((slot, item)) = resurrection(context, player).await
        && context
            .store
            .take_item(player.character.id, slot, item)
            .await
            .is_ok()
    {
        send_containers(link, &context.catalog, &context.store, player).await;

        // Said to the whole room, as the original says it (`Player.cs:920-921`), because an amulet
        // breaking is the room's business too.
        let shown = context
            .catalog
            .type_of_uuid(item)
            .and_then(|kind| context.catalog.object(kind))
            .map(|desc| desc.name().to_string())
            .unwrap_or_else(|| "amulet".to_string());
        placement
            .world
            .send(ToWorld::Announce {
                text: format!("{name}'s {shown} breaks and he disappears"),
            })
            .await;

        return send_home(
            link,
            context,
            player,
            placement,
            name,
            orders,
            died,
            &departed.vitals,
        )
        .await;
    }

    // `Nexus` (`Player.cs:899`). Nowhere safe kills anybody: a stone goes down under their own
    // name and they are put back where they already were, alive. Our personal rooms are counted in
    // with it, which the original has no equivalent of because its vault is a world like any other.
    if context.worlds.is_personal(&placement.world.name)
        || placement.world.name.as_ref() == crate::commands::NEXUS
    {
        gravestone(context, player, placement, &departed, false).await;
        return send_home(
            link,
            context,
            player,
            placement,
            name,
            orders,
            died,
            &departed.vitals,
        )
        .await;
    }

    // What the body was at the moment it fell, written before anything reads the row back. The
    // fame and the tally a death is worth are read from the store below, and the last seconds of a
    // life are exactly when a character earns them. `Player.cs:1018` saves in the same place, on
    // the line before it calls `Database.Death`. The figures are carried on the death itself
    // because the body is reaped on the tick it dies and there is nothing left to ask by now.
    if let Err(err) = save_vitals(context, player, &departed.vitals, Written::WithClass).await {
        tracing::warn!(%err, character = player.character.id, "could not save a character that died");
    }

    // The durable half first. Everything after this is telling people about it.
    //
    // The graveyard row and the character being marked dead go together in one transaction: a
    // character marked dead with no death recorded loses the only account of what happened to it,
    // and a death recorded against a living character is a graveyard entry for somebody still
    // playing.
    let fame = departed.vitals.fame;

    // `character.CharId < 2` (`FameStats.cs:122`). The original's char ids count from zero per
    // account, so this is the first two characters an account ever made, whether or not either has
    // died -- which is not what the bonus is called and is what the original does.
    let ancestor = context
        .store
        .characters_made_before(player.account.id, player.character.id)
        .await
        .unwrap_or(2)
        < 2;

    // What the character did, which is what its death is worth beyond the fame it had. Read from
    // the database rather than from the world, because the world holds only this session's counting
    // and a character's life is longer than one session.
    let tally = context
        .store
        .tally(player.character.id)
        .await
        .unwrap_or_default();

    // What every character the account has finished with came to, which first born is measured
    // against. `None` where there is nothing to beat, which the original treats as beaten
    // (`FameStats.cs:263`). Read from the graveyard rather than from the per-class records the
    // original keeps, which is the nearest thing we have to them.
    let best_before = match context.store.has_died_before(player.account.id).await {
        Ok(true) => context.store.best_final_fame(player.account.id).await.ok(),
        _ => None,
    };

    let (final_fame, earned) = hendra_sim::fame::bonuses(
        &remembered(&tally),
        hendra_sim::fame::Finished {
            level: departed.vitals.level,
            fame,
            ancestor,
            equipment_bonus: worn_fame_bonus(context, player).await,
            best_before,
        },
    );

    for bonus in &earned {
        say(
            link,
            &format!("{}: {} fame, {}", bonus.name, bonus.fame, bonus.why),
        )
        .await;
    }

    // Whether this beat every previous best, which is what the graveyard row records: the original
    // stores `CalculateTotal`'s `firstBorn` out-parameter (`Database.cs:1099`), which is the
    // best-fame comparison and not the ancestor bonus.
    let first_born = earned.iter().any(|bonus| bonus.name == "First Born");

    let recorded = context
        .store
        .record_death(
            hendra_store::Death {
                account_id: player.account.id,
                character_id: player.character.id,
                killed_by: departed.killer.clone(),
                final_fame,
                first_born,
                bonuses: earned
                    .iter()
                    .map(|bonus| hendra_store::Awarded {
                        name: bonus.name.to_string(),
                        fame: bonus.fame,
                    })
                    .collect(),
            },
            player.lock.as_ref(),
        )
        .await;

    if let Err(err) = recorded {
        // Said loudly. A death that was not written down is a character the player can log back
        // into, which is the one outcome worth shouting about.
        tracing::error!(
            %err,
            character = player.character.id,
            account = player.account.id,
            "a death could not be recorded"
        );
    } else if let Err(err) = context
        .store
        .record_class_progress(
            player.account.id,
            player.character.class,
            departed.vitals.level,
            final_fame,
        )
        .await
    {
        // How far the account has taken this class, which is what opens the next one. A death
        // counts for what it finished with rather than for what it had banked: the original updates
        // the class records from inside `Database.Death`, and takes the greater of the two
        // (`DbModels.cs:586`). The high-water mark only ever rises, so recording it again with the
        // smaller number would have cost the account the bonuses.
        tracing::warn!(
            %err,
            character = player.character.id,
            "could not record what a death took a class to"
        );
    }

    let maxed = gravestone(context, player, placement, &departed, false).await;

    // Word for word from `AnnounceDeath` (`Player.cs:944-948`).
    let line = format!(
        "{} died to {}! ({maxed}/8, {final_fame} Fame)",
        player.character.name, departed.killer
    );

    // Said everywhere for a death worth hearing about, and to the room otherwise. The original
    // draws the line at six of eight or a thousand fame, reads the fame the character had rather
    // than what its death came to, and never shouts about an administrator (`Player.cs:953`).
    let notable = (maxed >= 6 || fame >= NOTABLE_FAME)
        && !hendra_store::Admin::from_number(player.account.admin_rank)
            .meets(hendra_store::Admin::MODERATOR);

    if notable {
        let said = ServerMessage::Chat {
            speaker: hendra_net::EntityId(0),
            from: "Server",
            text: &line,
        };
        for who in context.trades.present() {
            context.trades.send(&who, &said);
        }
    } else {
        // Told to the room, and to the one person who is no longer in it. `AnnounceDeath` walks
        // `Owner.Players.Values` (`Player.cs:978`), which still holds the player who just died --
        // they leave on the disconnect a second later, not on the death -- so the last thing a
        // character hears is what it died to.
        say(link, &line).await;
        placement.world.send(ToWorld::Announce { text: line }).await;
    }

    let mut buffer = Vec::new();
    ServerMessage::Died {
        character: player.character.id.max(0) as u32,
        killed_by: departed.killer,
        fame: final_fame,
    }
    .encode(&mut Writer::new(&mut buffer));
    let _ = link.send(Delivery::Stream, &buffer).await;

    // A second before the connection goes, as `Player.Death` waits on a thousand-millisecond world
    // timer before disconnecting (`Player.cs:1033`). The client needs the moment to draw what it
    // was just told; a link closed on the same breath as the message can lose it.
    tokio::time::sleep(DEATH_LINGER).await;

    AfterDeath::Over
}

/// How long a dead player's connection is left open, from `Player.cs:1033`.
const DEATH_LINGER: std::time::Duration = std::time::Duration::from_millis(1000);

/// Puts a stone where somebody fell, and says how much of the character was finished.
///
/// `GenerateGravestone` (`Player.cs:833`): the count of stats at their class maximum picks the
/// stone and how long it stands, and a phantom death is the same stone under a different name.
async fn gravestone(
    context: &Context,
    player: &crate::accounts::Session,
    placement: &Placement,
    departed: &crate::world_task::Departed,
    rekt: bool,
) -> usize {
    let maxed = maxed_stats(context, player, &departed.vitals);

    placement
        .world
        .send(ToWorld::Gravestone {
            at: (departed.x, departed.y),
            name: player.character.name.clone(),
            maxed,
            level: departed.vitals.level,
            rekt,
        })
        .await;

    maxed
}

/// Puts a player back in the nexus, alive.
///
/// `ReconnectToNexus` (`Player.cs:929`): health set to one, and a reconnect to the nexus. The
/// original's reconnect saves the character and loads it again on the far side, which is why the
/// one point of health survives the journey; here the new body is built with it directly.
#[allow(clippy::too_many_arguments)]
async fn send_home(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
    from: &Placement,
    name: &str,
    orders: &mpsc::Sender<crate::world_task::Order>,
    died: &mpsc::Sender<crate::world_task::Departed>,
    vitals: &crate::world_task::Vitals,
) -> AfterDeath {
    let Some(world) = context
        .worlds
        .get_or_start_for(crate::commands::NEXUS, player.account.id)
    else {
        tracing::error!(%name, "nowhere to send somebody who should not have died");
        return AfterDeath::Nowhere;
    };

    // Written down before the new body is built, as the original's reconnect writes the character
    // and reads it back on the far side. Without it a rekting costs the player everything since the
    // last checkpoint: the body that earned it has already been reaped, and the figures it was
    // carrying exist nowhere but here.
    if let Err(err) = save_vitals(context, player, vitals, Written::WithClass).await {
        tracing::warn!(%err, character = player.character.id, "could not save somebody sent home");
    }

    let mut arrival = arrival_of(player, context).await;
    arrival.hp = 1;
    arrival.mp = vitals.mp;
    arrival.max_hp = vitals.max_hp;
    arrival.stats = hendra_sim::stats::Stats::from_base(&vitals.stats);
    arrival.progress = hendra_sim::leveling::Progress {
        level: vitals.level.max(1),
        experience: vitals.experience,
        fame: vitals.fame,
    };

    let Some(handle) = join(link, &world, name, arrival, orders, died).await else {
        return AfterDeath::Nowhere;
    };

    from.world
        .send(ToWorld::Leave {
            handle: from.handle,
        })
        .await;

    context.trades.moved(name, &world.key);

    AfterDeath::SentHome(Placement { world, handle })
}

/// A death worth telling the whole server about.
const NOTABLE_FAME: i32 = 1000;

/// The worn item that will spend itself to prevent a death, if there is one.
async fn resurrection(
    context: &Context,
    player: &crate::accounts::Session,
) -> Option<(i16, uuid::Uuid)> {
    let character = context.store.character(player.character.id).await.ok()?;

    character
        .inventory
        .iter()
        .filter(|(slot, _)| *slot < EQUIPPED_SLOTS as i16)
        .find(|(_, item)| {
            context
                .catalog
                .type_of_uuid(*item)
                .and_then(|kind| context.catalog.object(kind))
                .and_then(|desc| desc.item.as_ref())
                .is_some_and(|item| item.resurrects)
        })
        .copied()
}

/// How many of the eight stats are at their class's maximum.
///
/// `playerDesc.Stats.Where((t, i) => Stats.Base[i] >= t.MaxValue).Count()` (`Player.cs:835`), which
/// is the number in "six of eight" and the one that picks a gravestone. Read from the base stats
/// the body was carrying rather than from the stored row, because the row is written from the same
/// snapshot and reading it back would be a query for figures already in hand.
fn maxed_stats(
    context: &Context,
    player: &crate::accounts::Session,
    vitals: &crate::world_task::Vitals,
) -> usize {
    let Some(class) = context
        .catalog
        .type_of_uuid(player.character.class)
        .and_then(|kind| {
            context
                .catalog
                .classes()
                .iter()
                .find(|class| class.object_type == kind)
        })
    else {
        return 0;
    };

    vitals
        .stats
        .iter()
        .zip(class.stats.iter())
        .filter(|(held, stat)| **held >= stat.maximum)
        .count()
}

/// What a character has done, as the bonuses need it.
fn remembered(tally: &hendra_store::TallyRow) -> hendra_sim::fame::Tally {
    hendra_sim::fame::Tally {
        shots: tally.shots,
        shots_that_hit: tally.shots_that_hit,
        abilities_used: tally.abilities_used,
        tiles_seen: tally.tiles_seen,
        teleports: tally.teleports,
        potions_drunk: tally.potions_drunk,
        monster_kills: tally.monster_kills,
        god_kills: tally.god_kills,
        cube_kills: tally.cube_kills,
        oryx_kills: tally.oryx_kills,
        quests_completed: tally.quests_completed,
        level_up_assists: tally.level_up_assists,
        dungeons_completed: tally.dungeons_completed.max(0) as u32,
    }
}

/// What the four worn items add to a death, as a percentage.
///
/// `FameBonus` in the content, which a handful of items carry. Only the worn slots count: what is
/// in the pack was not being used.
async fn worn_fame_bonus(context: &Context, player: &crate::accounts::Session) -> i32 {
    let Ok(character) = context.store.character(player.character.id).await else {
        return 0;
    };

    character
        .inventory
        .iter()
        .filter(|(slot, _)| *slot < EQUIPPED_SLOTS as i16)
        .filter_map(|(_, item)| {
            context
                .catalog
                .type_of_uuid(*item)
                .and_then(|kind| context.catalog.object(kind))
        })
        .filter_map(|desc| desc.item.as_ref())
        .map(|item| item.fame_bonus)
        .sum()
}

/// Whether an item belongs to whoever found it.
///
/// Soulbound in the content. It cannot be traded, and dropping it makes a bag only the dropper can
/// open, so there is no path by which it reaches somebody else.
fn is_soulbound(catalog: &hendra_content::Catalog, item: uuid::Uuid) -> bool {
    catalog
        .type_of_uuid(item)
        .and_then(|kind| catalog.object(kind))
        .and_then(|desc| desc.item.as_ref())
        .is_some_and(|item| item.soulbound)
}

/// Waits for a place, if the server is full.
///
/// Returns whether they got in. A refusal makes everybody retry, and everybody retrying makes a busy
/// server hardest to get into exactly when it is busiest; a line is one connection at a time in an
/// order the server chooses, and the people in it can be told where they stand.
///
/// Somebody who gives up leaves the line on the way out, so a queue does not fill with connections
/// that are no longer there.
async fn wait_for_room(
    link: &mut Link,
    context: &Context,
    player: &crate::accounts::Session,
) -> bool {
    // Not full: the common case, and it costs one comparison.
    if context.trades.present().len() < context.capacity {
        return true;
    }

    let mut place = {
        let Ok(mut queue) = context.queue.lock() else {
            return true;
        };

        // Reconnecting is decided by whether this account was playing a moment ago. Somebody the
        // server dropped is not a new arrival competing for a place.
        queue.join(player.account.id, player.account.admin_rank, false)
    };

    tracing::info!(
        account = player.account.id,
        place,
        "the server is full; waiting"
    );

    tell_place(link, context, player).await;

    loop {
        tokio::time::sleep(QUEUE_LOOK).await;

        // A connection that has gone should not hold a place, and there is nothing else that would
        // notice: nobody in a queue sends anything.
        if link.sender().is_closed() {
            if let Ok(mut queue) = context.queue.lock() {
                queue.leave(player.account.id);
            }
            return false;
        }

        let room = context.trades.present().len() < context.capacity;

        let mine = {
            let Ok(mut queue) = context.queue.lock() else {
                return true;
            };

            // First in the line and room to spare is the only combination that gets in, so a place
            // can never be taken by somebody further back.
            if room && queue.place_of(player.account.id) == Some(1) {
                queue.next_in();
                return true;
            }

            queue.place_of(player.account.id)
        };

        let Some(now) = mine else {
            // Taken out of the line by something else, which today means the account was claimed
            // by another connection.
            return false;
        };

        // Only when it changes: a place that is re-sent every second is a number nobody reads.
        if now != place {
            place = now;
            tell_place(link, context, player).await;
        }
    }
}

/// Tells somebody where they stand.
async fn tell_place(link: &mut Link, context: &Context, player: &crate::accounts::Session) {
    let (place, waiting) = {
        let Ok(queue) = context.queue.lock() else {
            return;
        };
        (queue.place_of(player.account.id).unwrap_or(0), queue.len())
    };

    let mut buffer = Vec::new();
    ServerMessage::Queued {
        place: place as u32,
        waiting: waiting as u32,
    }
    .encode(&mut Writer::new(&mut buffer));
    let _ = link.send(Delivery::Stream, &buffer).await;
}

/// How often somebody waiting looks to see whether there is room.
///
/// A second: fast enough that a place opening is taken promptly, slow enough that a full server is
/// not spending its time answering people who are not in it yet.
const QUEUE_LOOK: std::time::Duration = std::time::Duration::from_secs(1);

/// What a character arrives with when the row remembers all of it, some of it, or none.
///
/// The none case is the one that matters: `0026` gave every character that already existed an empty
/// `stats` column, and a fallback that read that as "a fresh character" would have written the
/// class's starting line over a level-twenty character three seconds after it logged in.
#[cfg(test)]
mod arriving {
    use hendra_content::{Node, ObjectType, Stat};

    const WIZARD: &str = r#"
<Objects>
   <Object type="0x030e" id="Wizard">
      <Player/>
      <MaxHitPoints max="670">100</MaxHitPoints>
      <MaxMagicPoints max="385">100</MaxMagicPoints>
      <Attack max="75">12</Attack>
      <Defense max="25">0</Defense>
      <Speed max="50">12</Speed>
      <Dexterity max="75">15</Dexterity>
      <HpRegen max="40">10</HpRegen>
      <MpRegen max="60">10</MpRegen>
      <LevelIncrease min="20" max="30">MaxHitPoints</LevelIncrease>
      <LevelIncrease min="2" max="8">MaxMagicPoints</LevelIncrease>
      <LevelIncrease min="0" max="2">Attack</LevelIncrease>
      <LevelIncrease min="1" max="2">Speed</LevelIncrease>
   </Object>
</Objects>"#;

    fn wizard() -> hendra_content::PlayerDesc {
        let document = Node::parse(WIZARD).unwrap();
        let node = document.children_named("Object").next().unwrap();
        hendra_content::PlayerDesc::parse(node, ObjectType(0x030e)).unwrap()
    }

    fn character(
        level: i16,
        max_hp: i32,
        max_mp: i32,
        stats: Vec<Option<i32>>,
    ) -> hendra_store::Character {
        hendra_store::Character {
            id: 1,
            account_id: 1,
            class: uuid::Uuid::nil(),
            name: "Wizard".into(),
            hp: max_hp,
            max_hp,
            mp: max_mp,
            max_mp,
            level,
            experience: 0,
            fame: 0,
            alive: true,
            inventory: Vec::new(),
            health_potions: 0,
            magic_potions: 0,
            has_backpack: false,
            stats,
            skin: 0,
            dye_cloth: 0,
            dye_accessory: 0,
        }
    }

    #[test]
    fn a_row_that_records_all_eight_is_restored_exactly() {
        let recorded = [670, 385, 75, 25, 50, 75, 60, 80];
        let stats = super::levelled_stats(
            &wizard(),
            &character(20, 670, 385, recorded.map(Some).to_vec()),
            [0; hendra_content::STAT_COUNT],
        );

        assert_eq!(stats.to_base(), recorded);
    }

    #[test]
    fn a_levelled_character_with_no_recorded_stats_keeps_its_own_maxima() {
        let stats = super::levelled_stats(
            &wizard(),
            &character(20, 670, 385, Vec::new()),
            [0; hendra_content::STAT_COUNT],
        );

        assert_eq!(
            stats.max_hp(),
            670,
            "the row's own health, not the class's first line"
        );
        assert_eq!(stats.max_mp(), 385);
    }

    #[test]
    fn a_levelled_character_with_no_recorded_stats_is_not_read_as_a_fresh_one() {
        let class = wizard();
        let fresh = hendra_sim::stats::Stats::starting(&class);
        let stats = super::levelled_stats(
            &class,
            &character(20, 670, 385, Vec::new()),
            [0; hendra_content::STAT_COUNT],
        );

        assert_ne!(stats.to_base(), fresh.to_base());

        // Nineteen levels of the average gain, which is what a shortcut to the top level grants.
        assert_eq!(stats.base(Stat::Attack), 12 + (0 + 2) * 19 / 2);
        assert_eq!(stats.base(Stat::Speed), 12 + (1 + 2) * 19 / 2);

        // A stat the class never grows is still its starting value, which is the honest answer.
        assert_eq!(stats.base(Stat::Dexterity), 15);
    }

    #[test]
    fn a_character_that_has_only_just_been_made_is_untouched() {
        let class = wizard();
        let stats = super::levelled_stats(
            &class,
            &character(1, 100, 100, Vec::new()),
            [0; hendra_content::STAT_COUNT],
        );

        assert_eq!(
            stats.to_base(),
            hendra_sim::stats::Stats::starting(&class).to_base()
        );
    }

    #[test]
    fn what_equipment_adds_is_not_read_back_as_something_levelling_produced() {
        // The row's maxima are what the body was playing with, so a robe worth eighty health is in
        // the eight hundred it recorded and must not also be in the base.
        let mut worn = [0i32; hendra_content::STAT_COUNT];
        worn[Stat::MaxHitPoints.index()] = 80;

        let stats = super::levelled_stats(&wizard(), &character(20, 670, 385, Vec::new()), worn);

        assert_eq!(stats.base(Stat::MaxHitPoints), 590);
    }

    #[test]
    fn a_row_that_records_only_some_of_the_eight_fills_the_rest_in() {
        let mut partial = vec![None; 8];
        partial[Stat::Attack.index()] = Some(63);

        let stats = super::levelled_stats(
            &wizard(),
            &character(20, 670, 385, partial),
            [0; hendra_content::STAT_COUNT],
        );

        assert_eq!(stats.base(Stat::Attack), 63, "what the row says wins");
        assert_eq!(stats.max_hp(), 670, "and the rest is still reconstructed");
    }
}

#[cfg(test)]
mod keepalive {
    use super::{Clocks, KEEPALIVE_DEADLINE};

    /// `Player.DcThresold` is 12000 and `PingPeriod` is 3000
    /// (`wServer/realm/entities/player/Player.KeepAlive.cs:11-12`), so four pings in a row have to
    /// go unanswered before a connection is dropped.
    #[test]
    fn the_deadline_is_four_pings_wide() {
        assert_eq!(KEEPALIVE_DEADLINE.as_millis(), 12_000);
        assert_eq!(
            KEEPALIVE_DEADLINE.as_millis() / super::CHECKPOINT.as_millis(),
            4
        );
    }

    /// `Player.Pong` halves the round trip and averages it over the session
    /// (`Player.KeepAlive.cs:107-108`).
    #[test]
    fn latency_is_half_the_round_trip_averaged() {
        let mut clocks = Clocks::default();

        // A ping stamped at 1000 answered at 1100 is a hundred milliseconds there and back.
        clocks.note(1100, 1000, 0);
        assert_eq!(clocks.latency_ms(), 50);

        // A second, slower one: (50 + 100) / 2.
        clocks.note(4200, 4000, 0);
        assert_eq!(clocks.latency_ms(), 75);
    }

    /// `Player.TimeMap` is the average of `server_now - client_time`, and `C2STime` adds it to a
    /// client timestamp to read it in server time (`Player.KeepAlive.cs:104-105, 136-139`).
    #[test]
    fn the_offset_carries_a_client_timestamp_into_server_time() {
        let mut clocks = Clocks::default();

        // A client that booted five seconds after the server reports five seconds less.
        clocks.note(20_000, 19_900, 15_000);
        clocks.note(23_000, 22_900, 18_000);

        assert_eq!(clocks.offset_ms(), 5_000);
        assert_eq!(18_000 + clocks.offset_ms(), 23_000);
    }

    /// Both clocks are milliseconds in a 32-bit field, and a server up for seven weeks wraps. The
    /// original's are `int`s and wrap the same way, so the difference has to be taken as a wrap
    /// rather than as a jump of four billion.
    #[test]
    fn a_wrapped_clock_still_measures_a_short_round_trip() {
        let mut clocks = Clocks::default();

        clocks.note(40, u32::MAX - 59, 0);

        assert_eq!(clocks.latency_ms(), 50);
    }

    /// Nothing has answered yet, and neither average may divide by zero.
    #[test]
    fn an_unanswered_connection_reports_nothing_rather_than_dividing_by_zero() {
        let clocks = Clocks::default();

        assert_eq!(clocks.latency_ms(), 0);
        assert_eq!(clocks.offset_ms(), 0);
    }
}

//! Dungeons a player opened with a key, and who may be pulled into one.
//!
//! Follows `Portal.CreateWorld` (`Portal.cs:82-88`), which marks the world a player-opened portal
//! built with `PlayerDungeon`, the opener's name, and two empty name sets: `Invites`, everybody who
//! has been asked and has not yet come, and `Invited`, everybody who has ever been asked or has
//! ever walked in. `DungeonInvite` and `DungeonAccept` (`UnrankedCommands.cs:149-315`) read and
//! write those three, and `UsePortalHandler` (`:86-92`) moves a name from the first set to the
//! second when its owner steps through the door.
//!
//! # Why this is here and not on the world
//!
//! The original hangs the four fields off the `World` object, which every command can reach because
//! every world lives in one process-wide dictionary. Here a world is a task behind a channel, and
//! `/dinvite` is typed by one session while `/daccept` is typed by another that is not in that
//! world at all — asking the world would be two round trips for a set membership. So the sets live
//! beside the registry that owns the world's lifetime, keyed by the same instance key that decides
//! which room a portal leads to.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a dungeon accepts invitations for.
///
/// `world.GetAge() > 90000` refuses both ends of it: `DungeonInvite` (`UnrankedCommands.cs:219`)
/// will not send one and `DungeonAccept` (`:171`) will not honour one. The age is the world's, not
/// the invitation's, so every invitation into one dungeon expires at the same moment.
pub const INVITE_WINDOW: Duration = Duration::from_millis(90_000);

/// One player-opened dungeon.
struct Opened {
    /// Whose it is, spelt as they spell it.
    ///
    /// Compared exactly rather than case-insensitively, because `DungeonInvite`
    /// (`UnrankedCommands.cs:215`) compares with `Opener.Equals(player.Name)`, and a name only ever
    /// arrives from the character that owns it.
    opener: String,

    /// When the door was stood, which is what says how long a room nobody ever entered is kept.
    claimed: Instant,

    /// When the world behind this key started ticking, which is what its age is measured from.
    ///
    /// Absent until somebody walks through the door. The original has no world at all before that:
    /// `Portal.CreateWorld` runs on the first use of the portal, so a dungeon nobody has entered
    /// has no age to be past.
    since: Option<Instant>,

    /// Asked, and not yet arrived. `/daccept` spends an entry from here.
    invites: HashSet<String>,

    /// Asked or arrived, ever. Nobody in here is asked twice.
    invited: HashSet<String>,
}

/// Every player-opened dungeon that is still worth remembering, by instance key.
#[derive(Default)]
pub struct Dungeons {
    inner: Mutex<HashMap<String, Opened>>,
}

/// What asking somebody came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invitation {
    /// They were asked, and can now `/daccept`.
    Sent,

    /// They have already been asked, or are already inside.
    Already,
}

/// What answering an invitation came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Come in.
    Welcome,

    /// The dungeon is more than [`INVITE_WINDOW`] old.
    Expired,

    /// They have been in already, so the invitation is spent.
    Already,

    /// Nobody asked them.
    Uninvited,
}

impl Dungeons {
    pub fn new() -> Dungeons {
        Dungeons {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Records that a key opened a door, and whose door it is.
    ///
    /// Called when the portal is stood rather than when the world behind it is built, which is
    /// where the original sets `Portal.PlayerOpened` and `Portal.Opener`
    /// (`Player.UseItem.cs:611-612`); the world copies them off the portal later.
    pub fn claim(&self, key: &str, opener: &str) {
        let Ok(mut open) = self.inner.lock() else {
            return;
        };

        open.entry(key.to_string()).or_insert_with(|| Opened {
            opener: opener.to_string(),
            claimed: Instant::now(),
            since: None,
            invites: HashSet::new(),
            invited: HashSet::new(),
        });
    }

    /// Starts the clock on a dungeon, the first time anybody is in it.
    ///
    /// Idempotent, so the second player through the door does not reset the ninety seconds.
    pub fn began(&self, key: &str) {
        let Ok(mut open) = self.inner.lock() else {
            return;
        };

        if let Some(dungeon) = open.get_mut(key) {
            dungeon.since.get_or_insert_with(Instant::now);
        }
    }

    /// Whether a world was opened by a player at all, which is `World.PlayerDungeon`.
    pub fn is_player_dungeon(&self, key: &str) -> bool {
        self.inner
            .lock()
            .is_ok_and(|open| open.contains_key(key))
    }

    /// Whether a name is the one that opened this dungeon.
    pub fn opened_by(&self, key: &str, name: &str) -> bool {
        self.inner
            .lock()
            .is_ok_and(|open| open.get(key).is_some_and(|dungeon| dungeon.opener == name))
    }

    /// Whether the dungeon is too old to invite into or to be invited into.
    ///
    /// A dungeon nobody has entered has no age, and so is not past it.
    pub fn expired(&self, key: &str) -> bool {
        let Ok(open) = self.inner.lock() else {
            return false;
        };

        open.get(key).is_some_and(|dungeon| {
            dungeon
                .since
                .is_some_and(|since| since.elapsed() > INVITE_WINDOW)
        })
    }

    /// Asks somebody in, unless they have been asked already.
    ///
    /// Names are held folded, as the original holds them: every read and write of both sets goes
    /// through `ToLower()`.
    pub fn invite(&self, key: &str, name: &str) -> Invitation {
        let Ok(mut open) = self.inner.lock() else {
            return Invitation::Already;
        };

        let Some(dungeon) = open.get_mut(key) else {
            return Invitation::Already;
        };

        let folded = name.to_lowercase();
        if dungeon.invited.contains(&folded) {
            return Invitation::Already;
        }

        dungeon.invited.insert(folded.clone());
        dungeon.invites.insert(folded);
        Invitation::Sent
    }

    /// Marks somebody as already inside without asking them.
    ///
    /// The `-g` form of `/dinvite` does this to a guild member who is standing in the dungeon
    /// already (`UnrankedCommands.cs:243-247`): they are reported as unable to be invited, and
    /// their name goes into `Invited` so that a second `/dinvite -g` does not ask them either.
    pub fn note_present(&self, key: &str, name: &str) {
        let Ok(mut open) = self.inner.lock() else {
            return;
        };

        if let Some(dungeon) = open.get_mut(key) {
            dungeon.invited.insert(name.to_lowercase());
        }
    }

    /// Answers an invitation, spending it if it is good.
    ///
    /// The three answers and their order are `DungeonAccept`'s (`UnrankedCommands.cs:169-197`): an
    /// unspent invitation is checked against the world's age first, a spent one says they have been
    /// already, and anything else says they were never asked.
    pub fn accept(&self, key: &str, name: &str) -> Admission {
        let Ok(mut open) = self.inner.lock() else {
            return Admission::Uninvited;
        };

        let Some(dungeon) = open.get_mut(key) else {
            return Admission::Uninvited;
        };

        let folded = name.to_lowercase();
        if dungeon.invites.contains(&folded) {
            if dungeon
                .since
                .is_some_and(|since| since.elapsed() > INVITE_WINDOW)
            {
                return Admission::Expired;
            }

            dungeon.invites.remove(&folded);
            return Admission::Welcome;
        }

        if dungeon.invited.contains(&folded) {
            return Admission::Already;
        }

        Admission::Uninvited
    }

    /// Notes that somebody walked through the door.
    ///
    /// `UsePortalHandler` (`:86-92`) does exactly this on every use of a portal that has a world
    /// hanging off it: the name comes out of `Invites` and goes into `Invited`. It is what makes an
    /// invitation single-use and what makes somebody who walked in unaided count as invited.
    pub fn entered(&self, key: &str, name: &str) {
        let Ok(mut open) = self.inner.lock() else {
            return;
        };

        if let Some(dungeon) = open.get_mut(key) {
            let folded = name.to_lowercase();
            dungeon.invites.remove(&folded);
            dungeon.invited.insert(folded);
        }
    }

    /// Forgets every dungeon whose world has stopped.
    ///
    /// The original has nothing to forget: the sets are fields on the world object and go when it
    /// does. Here they outlive it unless something says otherwise, and a key is reused the moment a
    /// portal entity number is.
    ///
    /// A dungeon nobody has walked into yet has no world for `live` to find — it is claimed when
    /// the door is stood, and the room behind it is not built until somebody uses it. Those are kept
    /// until they are older than a door can be: a portal stands for thirty seconds and the
    /// invitation window is ninety, so nothing past that can ever be entered.
    pub fn retain(&self, live: impl Fn(&str) -> bool) {
        if let Ok(mut open) = self.inner.lock() {
            open.retain(|key, dungeon| match dungeon.since {
                Some(_) => live(key),
                None => dungeon.claimed.elapsed() <= INVITE_WINDOW,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_world_nobody_opened_is_not_a_player_dungeon() {
        let dungeons = Dungeons::new();
        assert!(!dungeons.is_player_dungeon("UndeadLair#Realm:7"));
        assert_eq!(
            dungeons.accept("UndeadLair#Realm:7", "Bo"),
            Admission::Uninvited
        );
    }

    #[test]
    fn only_the_opener_owns_it() {
        let dungeons = Dungeons::new();
        dungeons.claim("UndeadLair#Realm:7", "Ana");

        assert!(dungeons.opened_by("UndeadLair#Realm:7", "Ana"));
        assert!(!dungeons.opened_by("UndeadLair#Realm:7", "Bo"));
    }

    #[test]
    fn an_invitation_is_spent_once() {
        let dungeons = Dungeons::new();
        dungeons.claim("UndeadLair#Realm:7", "Ana");

        assert_eq!(dungeons.invite("UndeadLair#Realm:7", "Bo"), Invitation::Sent);
        assert_eq!(
            dungeons.invite("UndeadLair#Realm:7", "Bo"),
            Invitation::Already
        );

        assert_eq!(
            dungeons.accept("UndeadLair#Realm:7", "bo"),
            Admission::Welcome
        );
        assert_eq!(
            dungeons.accept("UndeadLair#Realm:7", "Bo"),
            Admission::Already
        );
    }

    #[test]
    fn walking_in_spends_the_invitation_too() {
        let dungeons = Dungeons::new();
        dungeons.claim("UndeadLair#Realm:7", "Ana");
        dungeons.invite("UndeadLair#Realm:7", "Bo");
        dungeons.entered("UndeadLair#Realm:7", "Bo");

        assert_eq!(
            dungeons.accept("UndeadLair#Realm:7", "Bo"),
            Admission::Already
        );
    }

    #[test]
    fn a_dungeon_nobody_has_entered_has_no_age() {
        let dungeons = Dungeons::new();
        dungeons.claim("UndeadLair#Realm:7", "Ana");
        assert!(!dungeons.expired("UndeadLair#Realm:7"));
    }

    #[test]
    fn the_clock_starts_once_and_expires_the_invitation() {
        let dungeons = Dungeons::new();
        dungeons.claim("UndeadLair#Realm:7", "Ana");
        dungeons.invite("UndeadLair#Realm:7", "Bo");
        dungeons.began("UndeadLair#Realm:7");

        // Wound back past the window, which is the only way to reach the answer without waiting
        // ninety seconds for it.
        {
            let mut open = dungeons.inner.lock().unwrap();
            let dungeon = open.get_mut("UndeadLair#Realm:7").unwrap();
            dungeon.since = Some(Instant::now() - INVITE_WINDOW - Duration::from_secs(1));
        }

        assert!(dungeons.expired("UndeadLair#Realm:7"));
        assert_eq!(
            dungeons.accept("UndeadLair#Realm:7", "Bo"),
            Admission::Expired
        );

        // A second arrival does not restart it.
        dungeons.began("UndeadLair#Realm:7");
        assert!(dungeons.expired("UndeadLair#Realm:7"));
    }

    #[test]
    fn a_world_that_stopped_is_forgotten() {
        let dungeons = Dungeons::new();
        dungeons.claim("UndeadLair#Realm:7", "Ana");
        dungeons.claim("UndeadLair#Realm:9", "Bo");
        dungeons.began("UndeadLair#Realm:7");
        dungeons.began("UndeadLair#Realm:9");

        dungeons.retain(|key| key == "UndeadLair#Realm:9");

        assert!(!dungeons.is_player_dungeon("UndeadLair#Realm:7"));
        assert!(dungeons.is_player_dungeon("UndeadLair#Realm:9"));
    }

    /// A door is claimed the moment it is stood and the room behind it is not built until somebody
    /// walks through, so a sweep that only kept running worlds would throw away every dungeon
    /// between the key being used and the portal being entered.
    #[test]
    fn a_door_nobody_has_walked_through_survives_a_sweep() {
        let dungeons = Dungeons::new();
        dungeons.claim("UndeadLair#Realm:7", "Ana");

        dungeons.retain(|_| false);
        assert!(dungeons.is_player_dungeon("UndeadLair#Realm:7"));

        // Past the point where a thirty-second door could still be walked through, it goes.
        {
            let mut open = dungeons.inner.lock().unwrap();
            let dungeon = open.get_mut("UndeadLair#Realm:7").unwrap();
            dungeon.claimed = Instant::now() - INVITE_WINDOW - Duration::from_secs(1);
        }

        dungeons.retain(|_| false);
        assert!(!dungeons.is_player_dungeon("UndeadLair#Realm:7"));
    }
}

//! Chat lines and join/leave messages (Шаг 25a).
//!
//! The layouts come from the vanilla 26.3 traces of step 25.0:
//!
//! * a typed line becomes `player_chat` for **every** client, the sender
//!   included (S19, `2026-11-18_s19s20-vanilla-step25-r1`);
//! * a join becomes `system_chat` with the `multiplayer.player.joined`
//!   translation and the new player's name component, sent to the other
//!   clients only; a leave is the same line with
//!   `multiplayer.player.left` (S20, `2026-11-18_s20-vanilla-step25-r4`).
//!
//! Wire order against the entity stream: vanilla sends the line **before**
//! the tab-list entry and the entity spawn of the joiner (S20: `system_chat`
//! seq 49174, `player_info_update` 49175, `add_entity` 49177) and before
//! `remove_entities`/`player_info_remove` of a leaver (57493, 57494, 57496).
//! Emitting the line from the join/quit handler keeps that order: the spawn
//! and removal packets follow in the same or the next tick.

use std::time::{SystemTime, UNIX_EPOCH};

use protocol::{PlayerChat, SystemChat};
use tracing::warn;

use super::messages::{PlayerId, ServerMessage};
use super::scheduler::Scheduler;

/// The translation key of the join line.
///
/// SOURCE: the S20 vanilla trace (`multiplayer.player.joined`).
const JOINED_KEY: &str = "multiplayer.player.joined";

/// The translation key of the leave line.
///
/// SOURCE: the S20 vanilla trace (`multiplayer.player.left`).
const LEFT_KEY: &str = "multiplayer.player.left";

/// Milliseconds since the Unix epoch, the `timestamp` of `player_chat`.
///
/// A clock before 1970 cannot happen on a machine that boots the game; it
/// still maps to `0` instead of panicking.
fn now_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

/// Broadcasts one typed line to every player of the world, the sender
/// included (Шаг 25a).
///
/// The server-wide `global_index` is the scheduler's counter: the S19 trace
/// numbers the first message of the run `0`, the second `1`, regardless of
/// the sender.
pub fn broadcast(scheduler: &mut Scheduler, sender: PlayerId, message: &str) {
    let Some(player) = scheduler.player(sender) else {
        warn!(
            player = sender,
            "chat from a player that is not in the world"
        );
        return;
    };
    let name = player.name().to_owned();
    let uuid = player.uuid();
    let line = PlayerChat {
        global_index: scheduler.next_chat_index(),
        sender: uuid,
        content: message.to_owned(),
        timestamp: now_millis(),
        sender_name: PlayerChat::name_component(&name, uuid),
    };
    let recipients: Vec<PlayerId> = scheduler.players().map(|player| player.id()).collect();
    for id in recipients {
        scheduler.deliver(id, ServerMessage::PlayerChat(line.clone()));
    }
}

/// Tells the other players that `joined` entered the world (Шаг 25a).
///
/// The joining client receives no own join line: the S20 trace has none.
pub fn announce_join(scheduler: &mut Scheduler, joined: PlayerId) {
    announce(scheduler, joined, JOINED_KEY);
}

/// Tells the remaining players that `leaving` is about to leave the world
/// (Шаг 25a). Called before the player is removed, so its name and UUID are
/// still readable.
pub fn announce_leave(scheduler: &mut Scheduler, leaving: PlayerId) {
    announce(scheduler, leaving, LEFT_KEY);
}

/// Sends one join/leave line to every player except `subject`.
fn announce(scheduler: &mut Scheduler, subject: PlayerId, key: &str) {
    let Some(player) = scheduler.player(subject) else {
        // A duplicate login removes the older session; if it is already gone
        // there is no name to announce and nothing to send.
        warn!(
            player = subject,
            key, "join/leave line for a player that is not in the world"
        );
        return;
    };
    let name = player.name().to_owned();
    let uuid = player.uuid();
    let line = SystemChat::join_leave(key, &name, uuid);
    let recipients: Vec<PlayerId> = scheduler
        .players()
        .map(|player| player.id())
        .filter(|id| *id != subject)
        .collect();
    for id in recipients {
        scheduler.deliver(id, ServerMessage::SystemChat(line.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::tests::join_fixture;
    use tokio::sync::mpsc::Receiver;

    /// The messages one connection received since the last call.
    fn drain(receiver: &mut Receiver<ServerMessage>) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        while let Ok(message) = receiver.try_recv() {
            out.push(message);
        }
        out
    }

    /// Two players in one region, with their outbound queues.
    fn world() -> (Scheduler, Receiver<ServerMessage>, Receiver<ServerMessage>) {
        let mut region = super::super::region::Region::new(super::super::SINGLE_REGION);
        let (join_a, queue_a) = join_fixture(1);
        let (join_b, queue_b) = join_fixture(2);
        assert!(region.insert(join_a).is_none());
        assert!(region.insert(join_b).is_none());
        let scheduler = Scheduler::new(vec![region]);
        (scheduler, queue_a, queue_b)
    }

    #[test]
    fn a_chat_line_reaches_both_players_with_a_server_wide_index() {
        let (mut scheduler, mut queue_a, mut queue_b) = world();
        broadcast(&mut scheduler, 1, "hello");
        broadcast(&mut scheduler, 2, "again");

        for queue in [&mut queue_a, &mut queue_b] {
            let lines: Vec<protocol::PlayerChat> = drain(queue)
                .into_iter()
                .map(|message| match message {
                    ServerMessage::PlayerChat(line) => line,
                    other => panic!("unexpected message: {other:?}"),
                })
                .collect();
            assert_eq!(lines.len(), 2, "one line per message");
            assert_eq!(lines[0].global_index, 0, "the first message of the run");
            assert_eq!(lines[1].global_index, 1, "one counter for both senders");
            assert_eq!(lines[0].content, "hello");
            assert_eq!(lines[1].content, "again");
            assert_eq!(lines[0].sender, scheduler.player(1).expect("player").uuid());
            assert_eq!(lines[1].sender, scheduler.player(2).expect("player").uuid());
        }
    }

    #[test]
    fn a_join_line_goes_to_the_others_only() {
        let (mut scheduler, mut queue_a, mut queue_b) = world();
        announce_join(&mut scheduler, 2);
        let lines = drain(&mut queue_a);
        assert_eq!(lines.len(), 1);
        match &lines[0] {
            ServerMessage::SystemChat(line) => {
                assert!(!line.overlay);
                let text = format!("{:?}", line.component);
                assert!(text.contains(JOINED_KEY), "key in {text}");
                assert!(text.contains("player-2"), "name in {text}");
            }
            other => panic!("unexpected message: {other:?}"),
        }
        assert!(drain(&mut queue_b).is_empty(), "no own join line");
    }

    #[test]
    fn a_leave_line_goes_to_the_others_only() {
        let (mut scheduler, mut queue_a, mut queue_b) = world();
        announce_leave(&mut scheduler, 1);
        let lines = drain(&mut queue_b);
        assert_eq!(lines.len(), 1);
        match &lines[0] {
            ServerMessage::SystemChat(line) => {
                let text = format!("{:?}", line.component);
                assert!(text.contains(LEFT_KEY), "key in {text}");
                assert!(text.contains("player-1"), "name in {text}");
            }
            other => panic!("unexpected message: {other:?}"),
        }
        assert!(drain(&mut queue_a).is_empty(), "no own leave line");
    }
}

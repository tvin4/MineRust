//! Commands of step 25b/25c: the join command tree and the small set of
//! vanilla commands this server implements.
//!
//! The trace facts come from `2026-11-18_s21-*-vanilla-step25-r*`:
//!
//! * a non-operator's `/gamemode`, `/tp`, `/time`, `/say` are unknown
//!   commands (two `system_chat` lines: `command.unknown.command` and the
//!   context line); the tree only declares `/help`;
//! * an operator level 2 gets `/gamemode`, `/tp`, `/time set` and `/say`;
//!   `/op` is level 3 and therefore still unknown at level 2;
//! * a successful `/gamemode` sends, to the actor,
//!   `player_abilities`, `player_info_update` (action `0x04`),
//!   `game_event` 3 (value = new mode), `player_abilities` again, then the
//!   success line `commands.gamemode.success.self`; the other clients get the
//!   `player_info_update` only, and the spectator visibility change is the
//!   entity stream's business (remove/add, see `entities::visibility_pass`);
//! * `/tp` moves the actor (`player_position` to it, the entity stream syncs
//!   the observers) and replies `commands.teleport.success.*`;
//! * `/time set` sends `set_time` to **both** clients and the actor gets
//!   `commands.time.set.time_marker`;
//! * `/say` is an ordinary `player_chat` from the actor (the S21 l2 trace
//!   carries a serverbound `chat_command_signed` and a clientbound
//!   `player_chat`);
//! * `/help` of vanilla prints ~73 individual `system_chat` lines; this
//!   server prints the usage of its own commands only (short help, the
//!   accepted subset noted in `docs/MECHANICS.md` 25.0c).
//!
//! Unknown or unimplemented commands are not imitated beyond the two-line
//! denial; the full vanilla set stays a debt.

use std::collections::BTreeMap;

use protocol::{
    ArgumentParser, CommandNode, CommandNodeKind, CommandTree, GameEvent, NbtTag, PlayerAbilities,
    PlayerChat, PlayerInfoGameMode, SetTime, StringKind, SystemChat,
};
use tracing::warn;

use super::messages::{PlayerId, ServerMessage};
use super::scheduler::Scheduler;
use super::OP_LEVEL;

/// Game-mode change event id of `game_event`.
///
/// SOURCE: 2026-11-18_s21-l2-vanilla-step25-r2, conn-1 seq 78585: event `03`,
/// value = the new mode as `f32` (`1.0` creative, `2.0` adventure, `3.0`
/// spectator, `0.0` survival).
const GAME_MODE_EVENT: u8 = 3;

/// Total ticks `/time set day` carries in the S21 l2 trace.
///
/// SOURCE: 2026-11-18_s21-l2-vanilla-step25-r2, conn-1 seq 106749:
/// `01 00 a8 c3 01` = one clock, id 0, 25000 ticks.
const DAY_TICKS: i32 = 25_000;

/// The command tree this server declares for an operator level.
///
/// The tree is ours, not the vanilla one: it contains exactly the commands
/// [`apply`] answers, so the client does not offer what the server would
/// deny. The vanilla non-operator tree (26 nodes, 265 bytes) and operator
/// tree (34 KB) are **not** reproduced — see the module documentation.
pub(crate) fn tree(op_level: i32) -> CommandTree {
    let mut nodes = vec![CommandNode {
        kind: CommandNodeKind::Root,
        executable: false,
        children: Vec::new(),
        redirect: None,
    }];
    let mut root_children = Vec::new();

    // Children always get a higher index than their parent, like the vanilla
    // tree: the literals are pushed first, then their arguments.
    let help = push(&mut nodes, CommandNode::literal("help").executable());
    root_children.push(help);
    let help_arg = push(
        &mut nodes,
        CommandNode::argument("command", ArgumentParser::String(StringKind::GreedyPhrase))
            .executable(),
    );
    nodes[help as usize].children.push(help_arg);

    if op_level >= OP_LEVEL {
        // /gamemode <mode>
        let gamemode = push(&mut nodes, CommandNode::literal("gamemode"));
        root_children.push(gamemode);
        let mode = push(
            &mut nodes,
            CommandNode::argument("gamemode", ArgumentParser::GameMode).executable(),
        );
        nodes[gamemode as usize].children.push(mode);

        // /tp <x> <y> <z> | /tp <player>
        let tp = push(&mut nodes, CommandNode::literal("tp"));
        root_children.push(tp);
        let location = push(
            &mut nodes,
            CommandNode::argument("location", ArgumentParser::Vec3).executable(),
        );
        let target = push(
            &mut nodes,
            CommandNode::argument("target", ArgumentParser::Entity { flags: 0x03 }).executable(),
        );
        nodes[tp as usize].children.extend([location, target]);

        // /time set <day|noon|night|midnight>
        let time = push(&mut nodes, CommandNode::literal("time"));
        root_children.push(time);
        let set = push(&mut nodes, CommandNode::literal("set"));
        nodes[time as usize].children.push(set);
        let time_value = push(
            &mut nodes,
            CommandNode::argument("time", ArgumentParser::Time { min: 0 }).executable(),
        );
        nodes[set as usize].children.push(time_value);

        // /say <message>
        let say = push(&mut nodes, CommandNode::literal("say"));
        root_children.push(say);
        let message = push(
            &mut nodes,
            CommandNode::argument("message", ArgumentParser::Message).executable(),
        );
        nodes[say as usize].children.push(message);
    }

    nodes[0].children = root_children;
    CommandTree { nodes }
}

/// Appends a node and returns its index.
fn push(nodes: &mut Vec<CommandNode>, node: CommandNode) -> u32 {
    let index = nodes.len() as u32;
    nodes.push(node);
    index
}

/// What one command does to the world.
pub fn apply(scheduler: &mut Scheduler, tick: u64, id: PlayerId, text: &str) {
    let Some(player) = scheduler.player(id) else {
        warn!(
            player = id,
            "command from a player that is not in the world"
        );
        return;
    };
    let level = player.op_level();
    let actor = Actor {
        name: player.name().to_owned(),
        uuid: player.uuid(),
    };
    let text = text.trim();
    let text = text.strip_prefix('/').unwrap_or(text).trim();
    if text.is_empty() {
        return;
    }
    let mut parts = text.split_whitespace();
    let Some(name) = parts.next() else {
        return;
    };
    let args: Vec<&str> = parts.collect();

    match name {
        "help" => help(scheduler, id, &args),
        "gamemode" if level >= OP_LEVEL => gamemode(scheduler, id, &actor, text, &args),
        "tp" if level >= OP_LEVEL => teleport(scheduler, id, &actor, text, &args, tick),
        "time" if level >= OP_LEVEL => time(scheduler, id, text, &args, tick),
        "say" if level >= OP_LEVEL => say(scheduler, id, &args),
        // Vanilla hides commands above the player's level from the tree and
        // answers them exactly like a misspelt command (S21 non-op, `/op` at
        // level 2).
        _ => unknown(scheduler, id, text),
    }
}

/// The identity of the command's actor, read before the player is mutated.
struct Actor {
    name: String,
    uuid: protocol::Uuid,
}

/// The two `system_chat` lines of an unknown command.
///
/// SOURCE: S21 non-op seq 68464/68465 (and every other unknown command).
fn unknown(scheduler: &mut Scheduler, id: PlayerId, text: &str) {
    reply(scheduler, id, tagged("command.unknown.command", None));
    reply(scheduler, id, context(text));
}

/// `/help [command]`: one plain component per line.
fn help(scheduler: &mut Scheduler, id: PlayerId, args: &[&str]) {
    let Some(level) = scheduler.player(id).map(|player| player.op_level()) else {
        return;
    };
    let mut usages: Vec<(&str, &str)> = vec![("/help [command]", "/help")];
    if level >= OP_LEVEL {
        usages.extend([
            (
                "/gamemode <survival|creative|adventure|spectator>",
                "/gamemode",
            ),
            ("/tp <x> <y> <z> or /tp <player>", "/tp"),
            ("/time set <day|noon|night|midnight>", "/time"),
            ("/say <message>", "/say"),
        ]);
    }
    let filter = args.first().map(|arg| arg.trim_start_matches('/'));
    let mut printed = 0;
    for (usage, command) in usages {
        if let Some(filter) = filter {
            if !command.trim_start_matches('/').eq_ignore_ascii_case(filter) {
                continue;
            }
            printed += 1;
        }
        reply(scheduler, id, NbtTag::String(usage.to_owned()));
    }
    if filter.is_some() && printed == 0 {
        // SOURCE: S21 non-op seq 75509 (`/help gamemode` without the
        // permission): the server answers `commands.help.failed`.
        reply(scheduler, id, tagged("commands.help.failed", None));
    }
}

/// `/gamemode <mode>`: the S21 l2 packet order and the success line.
fn gamemode(scheduler: &mut Scheduler, id: PlayerId, actor: &Actor, text: &str, args: &[&str]) {
    let Some(arg) = args.first() else {
        // SOURCE: wiki, unverified — the usage key was not traced.
        reply(scheduler, id, tagged("commands.gamemode.usage", None));
        reply(scheduler, id, context(text));
        return;
    };
    let Some(mode) = parse_mode(arg) else {
        // SOURCE: S21 l2 seq 113873/113874.
        reply(
            scheduler,
            id,
            tagged(
                "argument.gamemode.invalid",
                Some(vec![NbtTag::String((*arg).to_owned())]),
            ),
        );
        reply(scheduler, id, context(text));
        return;
    };
    if scheduler
        .player_mut(id)
        .is_none_or(|player| !player.set_game_mode(mode))
    {
        // A repeated mode still gets the vanilla sequence: the trace changes
        // between different modes only, so this branch is a hypothesis.
        // SOURCE: wiki, unverified.
    }
    let abilities = PlayerAbilities::for_mode(mode);
    scheduler.deliver(id, ServerMessage::PlayerAbilities(abilities));
    let recipients: Vec<PlayerId> = scheduler.players().map(|player| player.id()).collect();
    for recipient in recipients {
        scheduler.deliver(
            recipient,
            ServerMessage::PlayerInfoGameMode(PlayerInfoGameMode {
                uuid: actor.uuid,
                game_mode: mode,
            }),
        );
    }
    scheduler.deliver(
        id,
        ServerMessage::GameEvent(GameEvent {
            event: GAME_MODE_EVENT,
            value: mode as f32,
        }),
    );
    scheduler.deliver(id, ServerMessage::PlayerAbilities(abilities));
    // SOURCE: S21 l2 seq 78587 (`commands.gamemode.success.self` with the
    // `gameMode.<name>` translation as its only argument).
    reply(
        scheduler,
        id,
        tagged(
            "commands.gamemode.success.self",
            Some(vec![tagged(&format!("gameMode.{}", mode_name(mode)), None)]),
        ),
    );
}

/// `/tp <x> <y> <z>` or `/tp <player>`.
fn teleport(
    scheduler: &mut Scheduler,
    id: PlayerId,
    actor: &Actor,
    text: &str,
    args: &[&str],
    tick: u64,
) {
    let (x, y, z, target_name) = match args.len() {
        3 => match (
            parse_coord(args[0]),
            parse_coord(args[1]),
            parse_coord(args[2]),
        ) {
            (Some(x), Some(y), Some(z)) => (
                // A whole-number x/z is a block coordinate and vanilla
                // centres it (S21 l2: `/tp 8 64 8` lands at 8.5, 64, 8.5).
                centre(args[0], x),
                y,
                centre(args[2], z),
                None,
            ),
            _ => {
                // SOURCE: wiki, unverified — the invalid-position key was not
                // traced.
                reply(scheduler, id, tagged("argument.pos.invalid", None));
                reply(scheduler, id, context(text));
                return;
            }
        },
        1 => {
            let name = args[0];
            let Some(target) = scheduler
                .players()
                .find(|player| player.name().eq_ignore_ascii_case(name))
            else {
                // SOURCE: wiki, unverified.
                reply(
                    scheduler,
                    id,
                    tagged("argument.entity.notfound.player", None),
                );
                reply(scheduler, id, context(text));
                return;
            };
            let position = target.position();
            (
                position.x,
                position.y,
                position.z,
                Some(target.name().to_owned()),
            )
        }
        _ => {
            // SOURCE: wiki, unverified.
            reply(scheduler, id, tagged("commands.teleport.usage", None));
            reply(scheduler, id, context(text));
            return;
        }
    };

    let orientation = match scheduler.player_mut(id) {
        Some(player) => {
            player.teleport(x, y, z);
            (player.position().yaw, player.position().pitch)
        }
        None => return,
    };
    let teleport_id = scheduler.next_teleport_id();
    scheduler.deliver(
        id,
        ServerMessage::PlayerPosition {
            teleport_id,
            x,
            y,
            z,
            yaw: orientation.0,
            pitch: orientation.1,
        },
    );
    // SOURCE: S21 l2 seq 95828: the location variant carries the actor's name
    // component and three `{ "": "%.6f" }` components; seq 104598: the entity
    // variant carries the actor and the target components. The actor's copy
    // is the only one (the observer's trace has no such line).
    let component = match target_name {
        Some(target) => tagged(
            "commands.teleport.success.entity.single",
            Some(vec![name_component(actor), name_component_by_name(&target)]),
        ),
        None => tagged(
            "commands.teleport.success.location.single",
            Some(vec![
                name_component(actor),
                coordinate(x),
                coordinate(y),
                coordinate(z),
            ]),
        ),
    };
    reply(scheduler, id, component);
    // The tick counter is not part of the reply; it only documents which
    // tick the command was applied in for the replay journal.
    let _ = tick;
}

/// `/time set <value>`.
fn time(scheduler: &mut Scheduler, id: PlayerId, text: &str, args: &[&str], tick: u64) {
    let Some(marker) = args.get(1) else {
        // SOURCE: wiki, unverified.
        reply(scheduler, id, tagged("commands.time.set.usage", None));
        reply(scheduler, id, context(text));
        return;
    };
    if !args
        .first()
        .is_some_and(|set| set.eq_ignore_ascii_case("set"))
    {
        // SOURCE: wiki, unverified.
        reply(scheduler, id, tagged("commands.time.set.usage", None));
        reply(scheduler, id, context(text));
        return;
    }
    let Some(ticks) = parse_time(marker) else {
        // SOURCE: wiki, unverified.
        reply(scheduler, id, tagged("argument.time.invalid_unit", None));
        reply(scheduler, id, context(text));
        return;
    };
    // A world clock is not simulated yet (join sends no `set_time`): the
    // packet carries the server tick as game time and one overworld clock,
    // with the traced day length for `day`. Debt: `docs/PROGRESS.md`.
    let packet = SetTime {
        game_time: i64::try_from(tick).unwrap_or(i64::MAX),
        clocks: vec![protocol::ClockUpdate {
            id: 0,
            total_ticks: ticks,
            partial_tick: 0.0,
            rate: 1.0,
        }],
    };
    let recipients: Vec<PlayerId> = scheduler.players().map(|player| player.id()).collect();
    for recipient in recipients {
        scheduler.deliver(recipient, ServerMessage::SetTime(packet.clone()));
    }
    // SOURCE: S21 l2 seq 106750.
    reply(
        scheduler,
        id,
        tagged(
            "commands.time.set.time_marker",
            Some(vec![
                NbtTag::String("minecraft:overworld".to_owned()),
                NbtTag::String(format!("minecraft:{marker}")),
            ]),
        ),
    );
}

/// `/say <message>`: an ordinary chat line from the actor.
fn say(scheduler: &mut Scheduler, id: PlayerId, args: &[&str]) {
    if args.is_empty() {
        // SOURCE: wiki, unverified.
        reply(scheduler, id, tagged("commands.say.usage", None));
        return;
    }
    let message = args.join(" ");
    super::chat::broadcast(scheduler, id, &message);
}

/// Parses the four game-mode names.
fn parse_mode(text: &str) -> Option<i32> {
    match text.to_ascii_lowercase().as_str() {
        "survival" => Some(0),
        "creative" => Some(1),
        "adventure" => Some(2),
        "spectator" => Some(3),
        _ => None,
    }
}

/// The `gameMode.<name>` translation suffix of a mode.
fn mode_name(mode: i32) -> &'static str {
    match mode {
        0 => "survival",
        1 => "creative",
        2 => "adventure",
        3 => "spectator",
        _ => "survival",
    }
}

/// `/time set` markers this server knows, in ticks.
///
/// SOURCE: `day` is the S21 l2 trace (25000). The other three markers use the
/// vanilla-memory tick values and are unverified: `docs/MECHANICS.md` 25.0c.
fn parse_time(text: &str) -> Option<i32> {
    match text.to_ascii_lowercase().as_str() {
        "day" => Some(DAY_TICKS),
        "noon" => Some(6_000),
        "night" => Some(13_000),
        "midnight" => Some(18_000),
        _ => None,
    }
}

/// Parses one coordinate of `/tp`; vanilla also accepts `~` deltas, which are
/// not implemented (debt).
fn parse_coord(text: &str) -> Option<f64> {
    text.parse::<f64>().ok()
}

/// Centres a whole-number block coordinate of `/tp`.
fn centre(text: &str, value: f64) -> f64 {
    if text.contains(['.', 'e', 'E']) {
        value
    } else {
        value + 0.5
    }
}

/// Sends one `system_chat` line to a player.
fn reply(scheduler: &mut Scheduler, id: PlayerId, component: NbtTag) {
    scheduler.deliver(
        id,
        ServerMessage::SystemChat(SystemChat {
            component,
            overlay: false,
        }),
    );
}

/// Builds `{ translate: key }` or `{ translate: key, with: [...] }`.
///
/// The compound keys are written sorted by our NBT map; NBT does not define
/// an order (the trace's insertion order differs, lengths and decoding do
/// not).
fn tagged(key: &str, with: Option<Vec<NbtTag>>) -> NbtTag {
    let mut root = BTreeMap::new();
    root.insert("translate".to_owned(), NbtTag::String(key.to_owned()));
    if let Some(with) = with {
        root.insert("with".to_owned(), NbtTag::List(with));
    }
    NbtTag::Compound(root)
}

/// The red two-line denial of an unknown or unparsable command.
///
/// SOURCE: S21 non-op seq 68464/68465 and the l2 variants: line one is
/// `{color:"red", extra:[{translate:"command.unknown.command"}], text:""}`,
/// line two carries the typed command as a clickable suggestion with the
/// underlined text and the italic `command.context.here` marker.
fn context(text: &str) -> NbtTag {
    let mut click = BTreeMap::new();
    click.insert(
        "action".to_owned(),
        NbtTag::String("suggest_command".to_owned()),
    );
    click.insert("command".to_owned(), NbtTag::String(format!("/{text}")));

    let mut typed = BTreeMap::new();
    typed.insert("color".to_owned(), NbtTag::String("red".to_owned()));
    typed.insert("text".to_owned(), NbtTag::String(text.to_owned()));
    typed.insert("underlined".to_owned(), NbtTag::Byte(1));

    let mut here = BTreeMap::new();
    here.insert("color".to_owned(), NbtTag::String("red".to_owned()));
    here.insert("italic".to_owned(), NbtTag::Byte(1));
    here.insert(
        "translate".to_owned(),
        NbtTag::String("command.context.here".to_owned()),
    );

    let mut inner = BTreeMap::new();
    inner.insert("click_event".to_owned(), NbtTag::Compound(click));
    inner.insert("color".to_owned(), NbtTag::String("gray".to_owned()));
    inner.insert(
        "extra".to_owned(),
        NbtTag::List(vec![NbtTag::Compound(typed), NbtTag::Compound(here)]),
    );
    inner.insert("text".to_owned(), NbtTag::String(String::new()));

    let mut root = BTreeMap::new();
    root.insert("color".to_owned(), NbtTag::String("red".to_owned()));
    root.insert(
        "extra".to_owned(),
        NbtTag::List(vec![NbtTag::Compound(inner)]),
    );
    root.insert("text".to_owned(), NbtTag::String(String::new()));
    NbtTag::Compound(root)
}

/// The name component of the actor, as the S21 success lines carry it.
fn name_component(actor: &Actor) -> NbtTag {
    PlayerChat::name_component(&actor.name, actor.uuid)
}

/// The name component of a player known only by name (the `/tp` target).
fn name_component_by_name(name: &str) -> NbtTag {
    PlayerChat::name_component(name, protocol::offline_uuid(name))
}

/// A `{ "": "%.6f" }` coordinate component of the teleport success line.
fn coordinate(value: f64) -> NbtTag {
    let mut root = BTreeMap::new();
    root.insert(String::new(), NbtTag::String(format!("{value:.6}")));
    NbtTag::Compound(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::messages::{KickFlag, MoveSlot, PlayerConnection};
    use crate::game::region::PlayerPos;
    use crate::game::{PlayerJoin, Region, SINGLE_REGION};
    use protocol::Position;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    /// A scheduler with one player at the spawn, returning its receiver.
    fn one_player(name: &str, op_level: i32) -> (Scheduler, mpsc::Receiver<ServerMessage>) {
        let (outbound, receiver) = mpsc::channel(64);
        let join = PlayerJoin {
            id: 1,
            name: name.to_owned(),
            view_distance: 10,
            spawn: Position { x: 8, y: 64, z: 8 },
            game_mode: 0,
            op_level,
            pose: Some(PlayerPos::entry(Position { x: 8, y: 64, z: 8 })),
            health: 20.0,
            food: 20,
            saturation: 5.0,
            saved_inventory: Vec::new(),
            connection: PlayerConnection {
                outbound,
                kick: Arc::new(KickFlag::default()),
                last_move: Arc::new(MoveSlot::default()),
            },
        };
        let mut scheduler = Scheduler::new(vec![Region::new(SINGLE_REGION)]);
        scheduler.insert_player(join);
        (scheduler, receiver)
    }

    /// Drains the queue.
    fn drain(receiver: &mut mpsc::Receiver<ServerMessage>) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        while let Ok(message) = receiver.try_recv() {
            out.push(message);
        }
        out
    }

    /// Every system-chat component of a drained batch.
    fn lines(messages: &[ServerMessage]) -> Vec<NbtTag> {
        messages
            .iter()
            .filter_map(|message| match message {
                ServerMessage::SystemChat(line) => Some(line.component.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_literal_and_an_argument_close_the_help_tree() {
        let tree = tree(0);
        assert_eq!(tree.nodes.len(), 3, "root, help literal, command argument");
        assert_eq!(tree.nodes[0].children, vec![1]);
        assert!(tree.nodes[1].executable);
        assert_eq!(tree.nodes[1].children, vec![2]);
        assert!(tree.encode_payload().expect("encodable").len() > 10);
    }

    #[test]
    fn an_operator_tree_declares_the_four_commands() {
        let tree = tree(2);
        assert_eq!(
            tree.nodes[0].children.len(),
            5,
            "help, gamemode, tp, time, say"
        );
        assert!(tree.nodes.len() > 10);
        tree.encode_payload().expect("encodable");
    }

    #[test]
    fn a_non_operator_gets_the_two_unknown_lines() {
        let (mut scheduler, mut receiver) = one_player("CmdA", 0);
        apply(&mut scheduler, 1, 1, "gamemode creative");
        let messages = drain(&mut receiver);
        let lines = lines(&messages);
        assert_eq!(lines.len(), 2);
        assert!(format!("{lines:?}").contains("command.unknown.command"));
        assert!(format!("{lines:?}").contains("command.context.here"));
    }

    #[test]
    fn an_operator_gamemode_sends_the_s21_sequence() {
        let (mut scheduler, mut receiver) = one_player("CmdA", 2);
        apply(&mut scheduler, 1, 1, "gamemode creative");
        let messages = drain(&mut receiver);
        let kinds: Vec<&str> = messages
            .iter()
            .map(|message| match message {
                ServerMessage::PlayerAbilities(_) => "abilities",
                ServerMessage::PlayerInfoGameMode(_) => "info",
                ServerMessage::GameEvent(_) => "event",
                ServerMessage::SystemChat(_) => "chat",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["abilities", "info", "event", "abilities", "chat"]);
        assert_eq!(scheduler.player(1).expect("player").game_mode(), 1);
    }

    #[test]
    fn an_invalid_gamemode_is_rejected_with_two_lines_and_keeps_the_mode() {
        let (mut scheduler, mut receiver) = one_player("CmdA", 2);
        apply(&mut scheduler, 1, 1, "gamemode banana");
        let messages = drain(&mut receiver);
        let lines = lines(&messages);
        assert_eq!(lines.len(), 2);
        assert!(format!("{lines:?}").contains("argument.gamemode.invalid"));
        assert_eq!(scheduler.player(1).expect("player").game_mode(), 0);
    }

    #[test]
    fn say_broadcasts_a_player_chat() {
        let (mut scheduler, mut receiver) = one_player("CmdA", 2);
        apply(&mut scheduler, 1, 1, "say hello world");
        let messages = drain(&mut receiver);
        match &messages[..] {
            [ServerMessage::PlayerChat(line)] => assert_eq!(line.content, "hello world"),
            other => panic!("expected one player_chat, got {other:?}"),
        }
    }

    #[test]
    fn teleport_moves_the_player_and_replies_with_the_location_line() {
        let (mut scheduler, mut receiver) = one_player("CmdA", 2);
        apply(&mut scheduler, 1, 1, "tp 8 64 8");
        let messages = drain(&mut receiver);
        match &messages[..] {
            [ServerMessage::PlayerPosition { x, y, z, .. }, ServerMessage::SystemChat(line)] => {
                assert_eq!((*x, *y, *z), (8.5, 64.0, 8.5), "whole blocks centre");
                assert!(format!("{:?}", line.component)
                    .contains("commands.teleport.success.location.single"));
            }
            other => panic!("expected the teleport and its line, got {other:?}"),
        }
        let player = scheduler.player(1).expect("player");
        assert_eq!((player.position().x, player.position().z), (8.5, 8.5));
    }

    #[test]
    fn time_set_sends_set_time_and_the_marker_line() {
        let (mut scheduler, mut receiver) = one_player("CmdA", 2);
        apply(&mut scheduler, 42, 1, "time set day");
        let messages = drain(&mut receiver);
        let mut saw_time = false;
        let mut saw_line = false;
        for message in &messages {
            match message {
                ServerMessage::SetTime(packet) => {
                    saw_time = true;
                    assert_eq!(packet.clocks.len(), 1);
                    assert_eq!(packet.clocks[0].total_ticks, DAY_TICKS);
                }
                ServerMessage::SystemChat(line) => {
                    saw_line = true;
                    assert!(
                        format!("{:?}", line.component).contains("commands.time.set.time_marker")
                    );
                }
                _ => {}
            }
        }
        assert!(saw_time && saw_line);
    }

    #[test]
    fn help_without_a_command_prints_the_short_list() {
        let (mut scheduler, mut receiver) = one_player("CmdA", 2);
        apply(&mut scheduler, 1, 1, "help");
        let messages = drain(&mut receiver);
        let lines = lines(&messages);
        assert_eq!(lines.len(), 5, "help plus the four operator usages");
    }

    #[test]
    fn help_of_a_hidden_command_fails_like_the_non_operator_trace() {
        let (mut scheduler, mut receiver) = one_player("CmdA", 0);
        apply(&mut scheduler, 1, 1, "help gamemode");
        let lines = lines(&drain(&mut receiver));
        assert_eq!(lines.len(), 1);
        assert!(format!("{lines:?}").contains("commands.help.failed"));
    }
}

//! Identity of a packet id: protocol state, direction and resource location.
//!
//! Every packet id the server sends or receives carries a [`PacketKey`]. The
//! key gives the server a stable way to resolve a numeric id by name: at
//! startup the id can be taken from the data generator's packets report
//! (`reports/packets.json` of the user's own copy of the game) instead of
//! the compiled default that comes from the public protocol documentation.

/// Protocol state a packet id belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolState {
    /// The initial handshake that picks the next state.
    Handshake,
    /// Server list ping.
    Status,
    /// Login sequence (profile exchange, compression, disconnect).
    Login,
    /// Configuration exchange (registries, tags, finish).
    Configuration,
    /// The play state: the actual game.
    Play,
}

impl ProtocolState {
    /// Number of states; the length of any array indexed by a state.
    pub const COUNT: usize = 5;

    /// Stable array index of the state, in protocol order.
    pub const fn index(self) -> usize {
        match self {
            Self::Handshake => 0,
            Self::Status => 1,
            Self::Login => 2,
            Self::Configuration => 3,
            Self::Play => 4,
        }
    }

    /// Name of the state as it appears in the packets report.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Handshake => "handshake",
            Self::Status => "status",
            Self::Login => "login",
            Self::Configuration => "configuration",
            Self::Play => "play",
        }
    }

    /// All states in protocol (index) order.
    pub fn all() -> [ProtocolState; Self::COUNT] {
        [
            Self::Handshake,
            Self::Status,
            Self::Login,
            Self::Configuration,
            Self::Play,
        ]
    }
}

/// Direction a packet travels, relative to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Sent by the server to the client.
    Clientbound,
    /// Sent by the client to the server.
    Serverbound,
}

impl Direction {
    /// Number of directions; the length of any array indexed by a direction.
    pub const COUNT: usize = 2;

    /// Stable array index of the direction (0 = clientbound, 1 = serverbound).
    pub const fn index(self) -> usize {
        match self {
            Self::Clientbound => 0,
            Self::Serverbound => 1,
        }
    }

    /// Name of the direction as it appears in the packets report.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clientbound => "clientbound",
            Self::Serverbound => "serverbound",
        }
    }
}

/// Identity of one packet id: the state it belongs to, its direction, its
/// resource location and the compiled default id.
///
/// Two keys denote the same packet when state, direction and name are equal;
/// two packets with the same name in different states or directions (for
/// example Keep Alive, which exists as clientbound and serverbound) are
/// distinct keys with their own ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketKey {
    /// Protocol state of the packet.
    pub state: ProtocolState,
    /// Direction the id is used for.
    pub direction: Direction,
    /// Resource location of the packet, e.g. `minecraft:keep_alive`.
    pub name: &'static str,
    /// Id used when no game data report is loaded (SOURCE: wiki, unverified).
    pub default_id: u32,
}

impl PacketKey {
    /// Creates a key with the compiled default id.
    pub const fn new(
        state: ProtocolState,
        direction: Direction,
        name: &'static str,
        default_id: u32,
    ) -> Self {
        Self {
            state,
            direction,
            name,
            default_id,
        }
    }

    /// Creates a clientbound key (server → client).
    pub const fn clientbound(state: ProtocolState, name: &'static str, default_id: u32) -> Self {
        Self::new(state, Direction::Clientbound, name, default_id)
    }

    /// Creates a serverbound key (client → server).
    pub const fn serverbound(state: ProtocolState, name: &'static str, default_id: u32) -> Self {
        Self::new(state, Direction::Serverbound, name, default_id)
    }

    /// Describes the key as `state/direction/name`, for error messages.
    pub fn describe(&self) -> String {
        format!(
            "{}/{}/{}",
            self.state.as_str(),
            self.direction.as_str(),
            self.name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_and_direction_has_a_distinct_index() {
        let states: [ProtocolState; ProtocolState::COUNT] = [
            ProtocolState::Handshake,
            ProtocolState::Status,
            ProtocolState::Login,
            ProtocolState::Configuration,
            ProtocolState::Play,
        ];
        let mut seen = [false; ProtocolState::COUNT];
        for state in states {
            seen[state.index()] = true;
        }
        assert!(seen.iter().all(|used| *used));

        assert_ne!(
            Direction::Clientbound.index(),
            Direction::Serverbound.index()
        );
    }

    #[test]
    fn keys_of_the_same_packet_in_two_directions_differ() {
        let cb = PacketKey::clientbound(ProtocolState::Play, "minecraft:keep_alive", 1);
        let sb = PacketKey::serverbound(ProtocolState::Play, "minecraft:keep_alive", 2);
        assert_ne!(cb, sb);
        assert_eq!(cb.name, sb.name);
    }

    #[test]
    fn describe_shows_state_direction_and_name() {
        let key = PacketKey::clientbound(ProtocolState::Play, "minecraft:login", 0x32);
        assert_eq!(key.describe(), "play/clientbound/minecraft:login");
    }
}

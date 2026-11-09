//! The scripted login → configuration → play sequence shared by every
//! slice that has to first join a server.
//!
//! The steps and the known-packs reply were walked against vanilla 26.3 in
//! the join slice recording; slices append their own steps after this list
//! (a drain, a wait, a reply of their own).

use std::time::Duration;

use data::PacketIds;
use protocol::{
    ClientInformation, FinishAck, FinishConfiguration, Handshake, LoginAcknowledged, LoginStart,
    LoginSuccess, NextState, ProtocolState,
};

use crate::{ParityEnv, Step};

/// How long to wait for a single scripted packet.
const WAIT: Duration = Duration::from_secs(15);
/// Vanilla answers its known-packs probe within milliseconds; our server
/// sends no probe at all, so this step must not burn the full WAIT.
const PROBE_WAIT: Duration = Duration::from_secs(3);
/// Player name of the scripted client; offline UUIDs derive from it.
const PLAYER: &str = "ParityBot";

/// Reply to vanilla's `select_known_packs` probe: the same single pack the
/// server announced (`minecraft:core` version `26.3`), echoed back in the
/// observed layout (VarInt count, then three strings).
///
/// The layout was read off vanilla's own probe bytes in the parity trace;
/// a successful exchange (finish arrives afterwards) is what proves it.
fn known_packs_reply() -> Vec<u8> {
    let mut out = Vec::new();
    protocol::encode_varint(1, &mut out);
    for part in ["minecraft", "core", "26.3"] {
        protocol::encode_varint(part.len() as i32, &mut out);
        out.extend_from_slice(part.as_bytes());
    }
    out
}

/// The scripted login → configuration → play sequence, up to the moment
/// the connection has entered play (Finish Acknowledged sent), with the
/// historical view distance of 8.
///
/// Slices append their own steps after this list (a drain, a wait, a reply
/// of their own); the walk bot asks for its own distance through
/// [`join_steps_with_view`].
pub fn join_steps(env: &ParityEnv, ids: &PacketIds, server_port: u16) -> Vec<Step> {
    join_steps_with_view(env, ids, server_port, 8)
}

/// The same scripted login → configuration → play sequence with an explicit
/// client view distance: the walk bot sends 10 to match the copy's
/// `server.properties` (`view-distance=10`).
pub fn join_steps_with_view(
    env: &ParityEnv,
    ids: &PacketIds,
    server_port: u16,
    view_distance: i8,
) -> Vec<Step> {
    let handshake = Handshake {
        protocol_version: env.config.protocol_version,
        server_address: "localhost".to_owned(),
        server_port,
        next_state: NextState::Login,
    };
    let login = LoginStart {
        name: PLAYER.to_owned(),
        uuid: protocol::offline_uuid(PLAYER),
    };
    let info = ClientInformation {
        locale: "en_us".to_owned(),
        view_distance,
        chat_mode: 0,
        chat_colors: true,
        skin_parts: 0x7F,
        main_hand: 1,
        text_filtering: false,
        allow_server_listings: true,
        particle_status: 0,
    };
    vec![
        Step::send(
            ids.id(Handshake::KEY),
            handshake.encode_payload().expect("handshake encodable"),
            Some(ProtocolState::Login),
        ),
        Step::send(
            ids.id(LoginStart::KEY),
            login.encode_payload().expect("login encodable"),
            None,
        ),
        Step::wait(ids.id(LoginSuccess::KEY), WAIT),
        Step::send(
            ids.id(LoginAcknowledged::KEY),
            LoginAcknowledged.encode_payload(),
            Some(ProtocolState::Configuration),
        ),
        Step::send(
            ids.id(ClientInformation::KEY),
            info.encode_payload().expect("client info encodable"),
            None,
        ),
        // Vanilla announces its data packs and expects an answer; our
        // server does not perform this exchange, so the step ends quietly.
        Step::respond(
            ids.id(protocol::SELECT_KNOWN_PACKS),
            PROBE_WAIT,
            ids.id(protocol::SELECT_KNOWN_PACKS_SERVERBOUND),
            known_packs_reply(),
        ),
        Step::wait(ids.id(FinishConfiguration::KEY), WAIT),
        Step::send(
            ids.id(FinishAck::KEY),
            FinishAck.encode_payload(),
            Some(ProtocolState::Play),
        ),
    ]
}

//! The login state and the configuration state that ends it.

use data::{GameData, PacketIds};
use protocol::{
    decode_frame, offline_uuid, ClientInformation, FinishAck, FinishConfiguration, GameProfile,
    LoginAcknowledged, LoginStart, LoginSuccess, RegistryData, RegistryEntry, UpdateTags,
    CONFIGURATION_PLUGIN_MESSAGE, MAX_STRING_CHARS,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, info};

use crate::error::ConnectionError;
use crate::frame::{read_frame, write_frame};

/// Serves the login state: Login Start is answered with Login Success and
/// the client confirms with Login Acknowledged to reach configuration,
/// which must be completed before the player name is returned.
///
/// Runs under the pre-play timeout; see
/// [`super::handle_connection`].
pub(super) async fn login_state<R, W>(
    reader: &mut R,
    writer: &mut W,
    game_data: &GameData,
    ids: &PacketIds,
) -> Result<String, ConnectionError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let body = read_frame(reader).await?;
    let (packet_id, mut payload) = decode_frame(&body)?;
    if packet_id != ids.id(LoginStart::KEY) {
        return Err(ConnectionError::UnexpectedPacket {
            id: packet_id,
            state: "login",
        });
    }
    let start = LoginStart::decode_payload(&mut payload)?;
    payload.ensure_empty()?;
    debug!(name = %start.name, "login start received");

    // Offline login: no encryption, no compression, and the profile UUID is
    // the documented offline derivation of the name.
    let uuid = offline_uuid(&start.name);
    let success = LoginSuccess {
        profile: GameProfile {
            uuid,
            name: start.name.clone(),
            properties: Vec::new(),
        },
        // A fresh random session UUID: the vanilla server draws a new one
        // per connection (two recorded join slices end login_finished with
        // two different version-4 UUIDs, while the first 41 bytes are the
        // same on both sides).
        // SOURCE: parity recording of vanilla 26.3 (behaviour-level).
        session_id: random_session_uuid()?,
    };
    write_frame(
        writer,
        ids.id(LoginSuccess::KEY),
        &success.encode_payload()?,
    )
    .await?;

    let body = read_frame(reader).await?;
    let (packet_id, mut payload) = decode_frame(&body)?;
    if packet_id != ids.id(LoginAcknowledged::KEY) {
        return Err(ConnectionError::UnexpectedPacket {
            id: packet_id,
            state: "login",
        });
    }
    LoginAcknowledged::decode_payload(&mut payload)?;
    payload.ensure_empty()?;
    debug!("login acknowledged, entering configuration");

    configuration_state(reader, writer, game_data, ids).await?;
    Ok(start.name)
}

/// Serves the configuration state: sends every synchronized registry, the
/// tags and Finish Configuration, then waits for the client's Finish
/// Acknowledged; afterwards the connection moves on to play.
async fn configuration_state<R, W>(
    reader: &mut R,
    writer: &mut W,
    game_data: &GameData,
    ids: &PacketIds,
) -> Result<(), ConnectionError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let registries = game_data.registries().count();
    for (registry_id, registry) in game_data.registries() {
        let packet = RegistryData {
            registry_id: registry_id.to_owned(),
            entries: registry
                .entries()
                .iter()
                .map(|entry| RegistryEntry {
                    name: entry.name.clone(),
                    data: Some(entry.data.clone()),
                })
                .collect(),
        };
        write_frame(writer, ids.id(RegistryData::KEY), &packet.encode_payload()?).await?;
    }

    let tags = game_data.tags_packet();
    write_frame(writer, ids.id(UpdateTags::KEY), &tags.encode_payload()?).await?;
    write_frame(
        writer,
        ids.id(FinishConfiguration::KEY),
        &FinishConfiguration.encode_payload(),
    )
    .await?;
    info!(
        registries,
        tag_registries = tags.registries.len(),
        "configuration data sent"
    );

    loop {
        let body = read_frame(reader).await?;
        let (packet_id, mut payload) = decode_frame(&body)?;
        match packet_id {
            id if id == ids.id(ClientInformation::KEY) => {
                let info = ClientInformation::decode_payload(&mut payload)?;
                payload.ensure_empty()?;
                debug!(
                    locale = %info.locale,
                    view_distance = info.view_distance,
                    "client information received"
                );
            }
            id if id == ids.id(CONFIGURATION_PLUGIN_MESSAGE) => {
                // The channel name is worth logging; M2 ignores the payload.
                let channel = payload.read_string(MAX_STRING_CHARS)?;
                debug!(%channel, "plugin message received, ignored");
            }
            id if id == ids.id(FinishAck::KEY) => {
                FinishAck::decode_payload(&mut payload)?;
                payload.ensure_empty()?;
                debug!("configuration acknowledged, entering play");
                return Ok(());
            }
            id => {
                return Err(ConnectionError::UnexpectedPacket {
                    id,
                    state: "configuration",
                });
            }
        }
    }
}

/// A random version-4 UUID for one connection's session id.
///
/// The vanilla 26.3 server sends a fresh random UUID in the tail of every
/// `minecraft:login_finished` recording (version nibble 4, RFC 4122
/// variant, changing between runs), so we draw our own per connection.
// SOURCE: parity recording of vanilla 26.3 (behaviour-level, two runs).
fn random_session_uuid() -> Result<[u8; 16], ConnectionError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|err| ConnectionError::Entropy(err.to_string()))?;
    bytes[6] = (bytes[6] & 0x0F) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3F) | 0x80; // RFC 4122 variant
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The session UUID must look like the vanilla one: version 4, RFC 4122
    /// variant, and never the same between two connections.
    #[test]
    fn random_session_uuid_is_a_fresh_version_four_id() {
        let first = random_session_uuid().expect("entropy available");
        let second = random_session_uuid().expect("entropy available");
        assert_ne!(first, second, "must not repeat within a run");
        assert_eq!(first[6] >> 4, 4, "version nibble must be 4");
        assert_eq!(first[8] & 0xC0, 0x80, "RFC 4122 variant bits");
        assert_eq!(second[6] >> 4, 4, "version nibble must be 4");
        assert_eq!(second[8] & 0xC0, 0x80, "RFC 4122 variant bits");
    }
}

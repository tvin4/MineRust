//! The login state and the configuration state that ends it.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use data::{GameData, PacketIds};
use protocol::{
    decode_frame, offline_uuid, ClientInformation, DisconnectConfiguration, FinishAck,
    FinishConfiguration, GameProfile, LoginAcknowledged, LoginStart, LoginSuccess, ProtocolState,
    RegistryData, RegistryEntry, UpdateTags, Uuid, CONFIGURATION_PLUGIN_MESSAGE, MAX_STRING_CHARS,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, info, warn};

use crate::error::ConnectionError;
use crate::frame::{read_tracked, write_tracked};

/// Offline profiles whose first connection has entered the pre-play flow but
/// has not finished configuration yet (Шаг 23a).
///
/// Claiming starts at Login Start so two back-to-back connections have a
/// deterministic first/late order. The claim is held through configuration
/// and removed by [`ConfigurationClaim::drop`] on success, disconnect, timeout
/// or decode error. This lock is never touched by the game tick.
#[derive(Debug, Default)]
pub(super) struct ConfigurationClaims {
    profiles: Mutex<HashSet<Uuid>>,
}

impl ConfigurationClaims {
    /// Claims `uuid` for the first pre-play connection; `None` means a prior
    /// connection still owns its configuration slot.
    fn claim(self: &Arc<Self>, uuid: Uuid) -> Option<ConfigurationClaim> {
        let mut profiles = self
            .profiles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !profiles.insert(uuid) {
            return None;
        }
        Some(ConfigurationClaim {
            claims: Arc::clone(self),
            uuid,
        })
    }
}

/// Removes one first connection's pre-play claim on every exit path.
struct ConfigurationClaim {
    claims: Arc<ConfigurationClaims>,
    uuid: Uuid,
}

impl Drop for ConfigurationClaim {
    fn drop(&mut self) {
        let mut profiles = self
            .claims
            .profiles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        profiles.remove(&self.uuid);
    }
}

/// Serves the login state: Login Start is answered with Login Success and
/// the client confirms with Login Acknowledged to reach configuration,
/// which must be completed before the player name is returned.
///
/// Returns what play needs of both states (a [`super::Client`]): the name
/// and the view distance the client asked for in configuration — the play
/// loop turns them into the radius it sends chunks in,
/// `min(client request, server maximum)`.
///
/// Runs under the pre-play timeout; see
/// [`super::handle_connection`].
pub(super) async fn login_state<R, W>(
    reader: &mut R,
    writer: &mut W,
    game_data: &GameData,
    ids: &PacketIds,
    claims: &Arc<ConfigurationClaims>,
) -> Result<Option<super::Client>, ConnectionError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let body = read_tracked(reader, ProtocolState::Login).await?;
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
    let configuration_claim = claims.claim(uuid);
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
    write_tracked(
        writer,
        ProtocolState::Login,
        ids.id(LoginSuccess::KEY),
        &success.encode_payload()?,
    )
    .await?;

    let body = read_tracked(reader, ProtocolState::Login).await?;
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

    if configuration_claim.is_none() {
        let disconnect = DisconnectConfiguration {
            reason: DisconnectConfiguration::DUPLICATE_LOGIN.to_owned(),
        };
        write_tracked(
            writer,
            ProtocolState::Configuration,
            ids.id(DisconnectConfiguration::KEY),
            &disconnect.encode_payload()?,
        )
        .await?;
        warn!(
            name = %start.name,
            "duplicate login: rejecting the later connection during configuration"
        );
        return Ok(None);
    }

    let view_distance = configuration_state(reader, writer, game_data, ids).await?;
    Ok(Some(super::Client {
        player_name: start.name,
        view_distance,
    }))
}

/// Serves the configuration state: sends every synchronized registry, the
/// tags and Finish Configuration, then waits for the client's Finish
/// Acknowledged; afterwards the connection moves on to play.
///
/// Returns the view distance of the last `client_information` the client
/// sent (`None` when it sent none at all) — the value the play loop caps
/// its chunk sending with.
async fn configuration_state<R, W>(
    reader: &mut R,
    writer: &mut W,
    game_data: &GameData,
    ids: &PacketIds,
) -> Result<Option<i32>, ConnectionError>
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
        write_tracked(
            writer,
            ProtocolState::Configuration,
            ids.id(RegistryData::KEY),
            &packet.encode_payload()?,
        )
        .await?;
    }

    let tags = game_data.tags_packet();
    write_tracked(
        writer,
        ProtocolState::Configuration,
        ids.id(UpdateTags::KEY),
        &tags.encode_payload()?,
    )
    .await?;
    write_tracked(
        writer,
        ProtocolState::Configuration,
        ids.id(FinishConfiguration::KEY),
        &FinishConfiguration.encode_payload(),
    )
    .await?;
    info!(
        registries,
        tag_registries = tags.registries.len(),
        "configuration data sent"
    );

    // The last requested view distance: configuration normally carries one
    // `client_information` right at its start, then nothing.
    let mut view_distance = None;
    loop {
        let body = read_tracked(reader, ProtocolState::Configuration).await?;
        let (packet_id, mut payload) = decode_frame(&body)?;
        match packet_id {
            id if id == ids.id(ClientInformation::KEY) => {
                let info = ClientInformation::decode_payload(&mut payload)?;
                payload.ensure_empty()?;
                view_distance = Some(i32::from(info.view_distance));
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
                debug!(?view_distance, "configuration acknowledged, entering play");
                return Ok(view_distance);
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
    use crate::frame::read_frame;
    use protocol::Uuid;
    use tokio::io::AsyncWriteExt;

    fn v4_duplicate_payload() -> Vec<u8> {
        let text = "0a0800097472616e736c61746500266d756c7469706c617965722e646973636f6e6e6563742e6475706c69636174655f6c6f67696e00";
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex pair"))
            .collect()
    }

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

    /// V4: the first pre-play connection owns the profile claim; a back-to-
    /// back duplicate is late. Once the first leaves configuration the same
    /// UUID may proceed to play (V1), and case-distinct names remain distinct
    /// profiles (V2).
    #[test]
    fn configuration_claims_preserve_v4_v1_and_v2_ordering() {
        let claims = Arc::new(ConfigurationClaims::default());
        let alice = offline_uuid("Alice");
        let lower = offline_uuid("alice");
        let first = claims.claim(alice).expect("first Alice wins");
        assert!(claims.claim(alice).is_none(), "late duplicate is rejected");
        let lower_claim = claims.claim(lower).expect("case-distinct profile");
        drop(lower_claim);
        drop(first);
        assert!(
            claims.claim(alice).is_some(),
            "after configuration V1 may replace the player in play"
        );
    }

    /// A late V4 connection receives configuration/clientbound id 2 and the
    /// exact 54-byte NBT component recorded by vanilla, then never enters
    /// play. The first claim remains alive throughout the exchange.
    #[tokio::test]
    async fn a_late_duplicate_is_disconnected_during_configuration_byte_for_byte() {
        let claims = Arc::new(ConfigurationClaims::default());
        let name = "Race";
        let first = claims.claim(offline_uuid(name)).expect("first Race claim");
        let task_claims = Arc::clone(&claims);
        let (server, mut client) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move {
            let (mut reader, mut writer) = tokio::io::split(server);
            login_state(
                &mut reader,
                &mut writer,
                &GameData::default(),
                &PacketIds::compiled(),
                &task_claims,
            )
            .await
        });

        let start = LoginStart {
            name: name.to_owned(),
            uuid: Uuid::default(),
        };
        client
            .write_all(&start.to_frame().expect("login start frame"))
            .await
            .expect("send start");
        client
            .write_all(&LoginAcknowledged.to_frame().expect("login ack frame"))
            .await
            .expect("send ack");

        let login_body = read_frame(&mut client).await.expect("login success frame");
        let (login_id, _) = decode_frame(&login_body).expect("login success body");
        assert_eq!(login_id, LoginSuccess::ID);

        let disconnect_body = read_frame(&mut client)
            .await
            .expect("configuration disconnect frame");
        let (disconnect_id, mut payload) = decode_frame(&disconnect_body).expect("disconnect body");
        assert_eq!(disconnect_id, DisconnectConfiguration::ID);
        let remaining = payload.remaining();
        assert_eq!(
            payload.read_bytes(remaining).expect("payload"),
            v4_duplicate_payload().as_slice()
        );
        assert!(
            task.await
                .expect("login task")
                .expect("clean rejection")
                .is_none(),
            "late connection never enters play"
        );
        drop(first);
    }
}

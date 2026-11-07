//! Configuration state packets: registry data, tags, finish handshake and
//! client settings.
//!
//! After Login Acknowledged the server sends the synchronized registries
//! (one Registry Data packet each) and their tags, then Finish
//! Configuration; the client answers with Finish Acknowledged and the
//! connection moves on to play. Field layouts follow the public protocol
//! documentation (<https://minecraft.wiki/wiki/Java_Edition_protocol>).

use crate::codec::{CodecError, PacketReader, PacketWriter, MAX_STRING_CHARS};
use crate::frame::encode_frame;
use crate::nbt::NbtTag;
use crate::packet_id::{PacketKey, ProtocolState};

/// Maximum locale length in UTF-16 code units (protocol String (16)).
const MAX_LOCALE_CHARS: usize = 16;

/// Key of the serverbound Plugin Message packet in the configuration state.
///
/// The server does not define a struct for it: the channel name is logged
/// and the payload is ignored.
pub const CONFIGURATION_PLUGIN_MESSAGE: PacketKey = PacketKey::serverbound(
    ProtocolState::Configuration,
    "minecraft:custom_payload",
    0x02,
);

/// One entry of the Registry Data packet.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryEntry {
    /// Full identifier of the entry, e.g. `minecraft:overworld`.
    pub name: String,
    /// Entry data as network NBT. `None` tells the client to source the
    /// entry from a known pack it already has.
    pub data: Option<NbtTag>,
}

/// Clientbound Registry Data: the contents of one synchronized registry.
///
/// A packet carries exactly one registry; the position of an entry in
/// [`entries`](Self::entries) defines its numeric id, starting at 0.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryData {
    /// Identifier of the registry, e.g. `minecraft:dimension_type`.
    pub registry_id: String,
    /// Entries in numeric id order (the index is the id).
    pub entries: Vec<RegistryEntry>,
}

impl RegistryData {
    /// Packet id of Registry Data (configuration state, clientbound).
    pub const ID: u32 = 0x07;

    /// Key that resolves the Registry Data id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Configuration,
        "minecraft:registry_data",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_string(&self.registry_id, MAX_STRING_CHARS)?;
        writer.write_varint(self.entries.len() as i32);
        for entry in &self.entries {
            writer.write_string(&entry.name, MAX_STRING_CHARS)?;
            match &entry.data {
                Some(data) => {
                    writer.write_bool(true);
                    writer.write_nbt(data)?;
                }
                None => writer.write_bool(false),
            }
        }
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let registry_id = reader.read_string(MAX_STRING_CHARS)?;
        let count = reader.read_varint()?;
        let count = usize::try_from(count).map_err(|_| CodecError::NegativeLength(count))?;
        // Registries hold up to a few thousand entries; cap the preallocation
        // so a hostile length cannot request an enormous buffer up front.
        let mut entries = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let name = reader.read_string(MAX_STRING_CHARS)?;
            let data = if reader.read_bool()? {
                Some(reader.read_nbt()?)
            } else {
                None
            };
            entries.push(RegistryEntry { name, data });
        }
        Ok(Self {
            registry_id,
            entries,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

/// A single tag of one registry: its name plus the numeric ids of the
/// entries it contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryTag {
    /// Tag identifier without the `#` prefix, e.g. `minecraft:is_fire`.
    pub name: String,
    /// Numeric ids of the tagged entries (ids of that registry).
    pub entries: Vec<i32>,
}

/// The tags of a single registry as sent by Update Tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRegistry {
    /// Identifier of the registry, e.g. `minecraft:damage_type`.
    pub registry_id: String,
    /// Tags defined for that registry.
    pub tags: Vec<RegistryTag>,
}

/// Clientbound Update Tags: replaces the server's tag lists for the
/// synchronized registries. Tags of registries the client already knows
/// (block, item, ...) are not sent here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateTags {
    /// Tags grouped by registry, in any order.
    pub registries: Vec<TagRegistry>,
}

impl UpdateTags {
    /// Packet id of Update Tags (configuration state, clientbound).
    pub const ID: u32 = 0x0E;

    /// Key that resolves the Update Tags id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Configuration,
        "minecraft:update_tags",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.registries.len() as i32);
        for registry in &self.registries {
            writer.write_string(&registry.registry_id, MAX_STRING_CHARS)?;
            writer.write_varint(registry.tags.len() as i32);
            for tag in &registry.tags {
                writer.write_string(&tag.name, MAX_STRING_CHARS)?;
                writer.write_varint(tag.entries.len() as i32);
                for &id in &tag.entries {
                    writer.write_varint(id);
                }
            }
        }
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let registry_count = read_count(reader)?;
        let mut registries = Vec::with_capacity(registry_count.min(64));
        for _ in 0..registry_count {
            let registry_id = reader.read_string(MAX_STRING_CHARS)?;
            let tag_count = read_count(reader)?;
            let mut tags = Vec::with_capacity(tag_count.min(1024));
            for _ in 0..tag_count {
                let name = reader.read_string(MAX_STRING_CHARS)?;
                let entry_count = read_count(reader)?;
                let mut entries = Vec::with_capacity(entry_count.min(1024));
                for _ in 0..entry_count {
                    entries.push(reader.read_varint()?);
                }
                tags.push(RegistryTag { name, entries });
            }
            registries.push(TagRegistry { registry_id, tags });
        }
        Ok(Self { registries })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

/// Reads a non-negative length prefix used by the array fields above.
fn read_count(reader: &mut PacketReader<'_>) -> Result<usize, CodecError> {
    let count = reader.read_varint()?;
    usize::try_from(count).map_err(|_| CodecError::NegativeLength(count))
}

/// Clientbound Finish Configuration: asks the client to validate the
/// received registry and tag data; it answers with [`FinishAck`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FinishConfiguration;

impl FinishConfiguration {
    /// Packet id of Finish Configuration (configuration state, clientbound).
    pub const ID: u32 = 0x03;

    /// Key that resolves the Finish Configuration id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Configuration,
        "minecraft:finish_configuration",
        Self::ID,
    );

    /// Encodes the payload; there is none, so this always succeeds.
    pub fn encode_payload(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Decodes the payload and fails if any bytes remain.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        reader.ensure_empty()?;
        Ok(Self)
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Serverbound Acknowledge Finish Configuration: the client is ready to
/// enter play. It has no fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FinishAck;

impl FinishAck {
    /// Packet id of Acknowledge Finish Configuration (configuration state,
    /// serverbound).
    pub const ID: u32 = 0x03;

    /// Key that resolves the Finish Acknowledged id at runtime.
    pub const KEY: PacketKey = PacketKey::serverbound(
        ProtocolState::Configuration,
        "minecraft:finish_configuration",
        Self::ID,
    );

    /// Encodes the payload; there is none, so this always succeeds.
    pub fn encode_payload(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Decodes the payload and fails if any bytes remain.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        reader.ensure_empty()?;
        Ok(Self)
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Serverbound Client Information: the client's settings, sent on entering
/// the configuration state and whenever the settings change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInformation {
    /// Language code, e.g. `en_GB`.
    pub locale: String,
    /// Client-side render distance in chunks.
    pub view_distance: i8,
    /// Chat mode: 0 enabled, 1 commands only, 2 hidden.
    pub chat_mode: i32,
    /// Whether multiplayer chat colors are enabled.
    pub chat_colors: bool,
    /// Bit mask of displayed skin parts (see the public protocol docs).
    pub skin_parts: u8,
    /// Main hand: 0 left, 1 right.
    pub main_hand: i32,
    /// Whether text filtering is enabled.
    pub text_filtering: bool,
    /// Whether the player allows appearing in server listings.
    pub allow_server_listings: bool,
    /// Particle status: 0 all, 1 decreased, 2 minimal.
    pub particle_status: i32,
}

impl ClientInformation {
    /// Packet id of Client Information (configuration state, serverbound).
    pub const ID: u32 = 0x00;

    /// Key that resolves the configuration Client Information id at runtime.
    pub const KEY: PacketKey = PacketKey::serverbound(
        ProtocolState::Configuration,
        "minecraft:client_information",
        Self::ID,
    );

    /// Key of the same packet in the play state, where the id differs and
    /// the compiled default is `0x0E` (SOURCE: wiki, unverified).
    pub const PLAY_KEY: PacketKey =
        PacketKey::serverbound(ProtocolState::Play, "minecraft:client_information", 0x0E);

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_string(&self.locale, MAX_LOCALE_CHARS)?;
        writer.write_i8(self.view_distance);
        writer.write_varint(self.chat_mode);
        writer.write_bool(self.chat_colors);
        writer.write_u8(self.skin_parts);
        writer.write_varint(self.main_hand);
        writer.write_bool(self.text_filtering);
        writer.write_bool(self.allow_server_listings);
        writer.write_varint(self.particle_status);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            locale: reader.read_string(MAX_LOCALE_CHARS)?,
            view_distance: reader.read_i8()?,
            chat_mode: reader.read_varint()?,
            chat_colors: reader.read_bool()?,
            skin_parts: reader.read_u8()?,
            main_hand: reader.read_varint()?,
            text_filtering: reader.read_bool()?,
            allow_server_listings: reader.read_bool()?,
            particle_status: reader.read_varint()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::split_frame;
    use std::collections::BTreeMap;

    /// A sample registry entry compound.
    fn sample_data() -> NbtTag {
        NbtTag::Compound(BTreeMap::from([
            ("height".to_owned(), NbtTag::Short(384)),
            ("has_skylight".to_owned(), NbtTag::Byte(1)),
        ]))
    }

    #[test]
    fn registry_data_roundtrips_entries_with_and_without_data() {
        let packet = RegistryData {
            registry_id: "minecraft:dimension_type".to_owned(),
            entries: vec![
                RegistryEntry {
                    name: "minecraft:overworld".to_owned(),
                    data: Some(sample_data()),
                },
                RegistryEntry {
                    name: "minecraft:the_end".to_owned(),
                    data: None,
                },
            ],
        };
        let frame = packet.to_frame().expect("encodable");
        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, RegistryData::ID);

        let decoded = RegistryData::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn registry_data_entry_order_is_the_numeric_id_order() {
        // The wire layout puts the count first and entries back to back, so
        // the first entry decodes as id 0, the second as id 1, and so on.
        let packet = RegistryData {
            registry_id: "minecraft:damage_type".to_owned(),
            entries: vec![
                RegistryEntry {
                    name: "minecraft:fall".to_owned(),
                    data: Some(NbtTag::Compound(BTreeMap::new())),
                },
                RegistryEntry {
                    name: "minecraft:generic".to_owned(),
                    data: None,
                },
                RegistryEntry {
                    name: "minecraft:fire".to_owned(),
                    data: None,
                },
            ],
        };
        let payload = packet.encode_payload().expect("encodable");
        let mut reader = PacketReader::new(&payload);
        assert_eq!(
            reader.read_string(MAX_STRING_CHARS).expect("registry"),
            "minecraft:damage_type"
        );
        assert_eq!(reader.read_varint().expect("count"), 3);
        for expected in ["minecraft:fall", "minecraft:generic", "minecraft:fire"] {
            assert_eq!(
                reader.read_string(MAX_STRING_CHARS).expect("entry"),
                expected,
                "entry order defines ids"
            );
            // Consume the optional data flag (and value when present).
            if reader.read_bool().expect("flag") {
                reader.read_nbt().expect("data");
            }
        }
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn update_tags_roundtrips_registries_tags_and_ids() {
        let packet = UpdateTags {
            registries: vec![
                TagRegistry {
                    registry_id: "minecraft:damage_type".to_owned(),
                    tags: vec![RegistryTag {
                        name: "minecraft:is_fire".to_owned(),
                        entries: vec![3, 17],
                    }],
                },
                TagRegistry {
                    registry_id: "minecraft:banner_pattern".to_owned(),
                    tags: vec![RegistryTag {
                        name: "minecraft:pattern_item/skull".to_owned(),
                        entries: vec![0],
                    }],
                },
            ],
        };
        let frame = packet.to_frame().expect("encodable");
        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, UpdateTags::ID);

        let decoded = UpdateTags::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn update_tags_roundtrips_an_empty_list() {
        let packet = UpdateTags::default();
        let frame = packet.to_frame().expect("encodable");
        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, UpdateTags::ID);

        let decoded = UpdateTags::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn finish_configuration_has_no_fields() {
        let frame = FinishConfiguration.to_frame().expect("encodable");
        assert_eq!(frame, vec![0x01, FinishConfiguration::ID as u8]);

        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, FinishConfiguration::ID);
        FinishConfiguration::decode_payload(&mut reader).expect("decodable");
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn finish_ack_has_no_fields() {
        let frame = FinishAck.to_frame().expect("encodable");
        assert_eq!(frame, vec![0x01, FinishAck::ID as u8]);

        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, FinishAck::ID);
        FinishAck::decode_payload(&mut reader).expect("decodable");
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn client_information_roundtrips_every_field() {
        let packet = ClientInformation {
            locale: "en_GB".to_owned(),
            view_distance: 12,
            chat_mode: 1,
            chat_colors: false,
            skin_parts: 0x7F,
            main_hand: 1,
            text_filtering: false,
            allow_server_listings: true,
            particle_status: 2,
        };
        let frame = packet.to_frame().expect("encodable");
        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, ClientInformation::ID);

        let decoded = ClientInformation::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn client_information_rejects_an_oversized_locale() {
        let packet = ClientInformation {
            locale: "x".repeat(MAX_LOCALE_CHARS + 1),
            view_distance: 8,
            chat_mode: 0,
            chat_colors: true,
            skin_parts: 0,
            main_hand: 1,
            text_filtering: false,
            allow_server_listings: true,
            particle_status: 0,
        };
        let err = packet.encode_payload().expect_err("too long");
        assert!(matches!(err, CodecError::StringTooLong { .. }));
    }
}

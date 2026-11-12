//! Small implementation helper shared by the entity packet codecs.

/// Adds the packet id, the runtime key and `to_frame` to a packet type.
macro_rules! play_packet {
    ($packet:ty, $id:expr, $name:literal) => {
        impl $packet {
            /// Packet id in the play state, clientbound.
            pub const ID: u32 = $id;

            /// Key that resolves the packet id at runtime.
            pub const KEY: $crate::packet_id::PacketKey = $crate::packet_id::PacketKey::clientbound(
                $crate::packet_id::ProtocolState::Play,
                $name,
                Self::ID,
            );

            /// Encodes the packet as a complete frame.
            pub fn to_frame(&self) -> Result<Vec<u8>, $crate::codec::CodecError> {
                $crate::frame::encode_frame(Self::ID, &self.encode_payload()?)
            }
        }
    };
}

pub(crate) use play_packet;

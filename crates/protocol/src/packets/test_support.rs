//! Shared assertions for the entity packet codec tests.

use crate::codec::{CodecError, PacketReader, Uuid};

/// Parses a hex string into bytes.
pub(crate) fn hex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0, "odd hex length");
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex pair"))
        .collect()
}

/// Parses a 16-byte hex string into a UUID.
pub(crate) fn uuid(text: &str) -> Uuid {
    let raw = hex(text);
    let mut uuid = [0; 16];
    uuid.copy_from_slice(&raw);
    uuid
}

/// Encodes `packet`, compares the payload with the recorded bytes and decodes
/// it back; any trailing byte fails the test.
pub(crate) fn check<T: PartialEq + std::fmt::Debug>(
    packet: T,
    expected: &str,
    encode: impl Fn(&T) -> Result<Vec<u8>, CodecError>,
    decode: fn(&mut PacketReader<'_>) -> Result<T, CodecError>,
) {
    let raw = hex(expected);
    assert_eq!(
        encode(&packet).expect("encodable"),
        raw,
        "encoded payload differs from the vanilla trace"
    );
    let mut reader = PacketReader::new(&raw);
    let decoded = decode(&mut reader).expect("decodable");
    assert_eq!(decoded, packet, "decoded packet differs");
    reader.ensure_empty().expect("no trailing bytes");
}

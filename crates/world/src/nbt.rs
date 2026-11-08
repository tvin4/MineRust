//! Decoding the on-disk NBT form.

use protocol::NbtTag;

use crate::error::WorldError;

/// Decodes a disk NBT document: `type + root name + payload`.
///
/// `protocol::NbtTag` decodes the network form, which omits the name of the
/// root tag; on disk the name is still there, so it is skipped here and only
/// the value is handed to the protocol decoder. The TAG_End sentinel of an
/// empty compound is not expected as a root (the root of a world document
/// is always a compound, observed in the owner's template).
pub(crate) fn disk_nbt(path: &str, bytes: &[u8]) -> Result<NbtTag, WorldError> {
    decode_disk(bytes).map_err(|detail| WorldError::Corrupt {
        path: path.to_owned(),
        detail,
    })
}

/// The shared disk decoder, failing with the detail alone so a caller
/// (`level.dat` reader) can wrap it into its own error shape.
pub(crate) fn decode_disk(bytes: &[u8]) -> Result<NbtTag, String> {
    let type_id = *bytes
        .first()
        .ok_or_else(|| "empty NBT document".to_owned())?;
    if type_id != 10 {
        return Err(format!("root tag type {type_id} is not a compound"));
    }
    let name_len = bytes
        .get(1..3)
        .map(|len| u16::from_be_bytes([len[0], len[1]]) as usize)
        .ok_or_else(|| "truncated root name length".to_owned())?;
    let payload = bytes
        .get(3 + name_len..)
        .ok_or_else(|| "truncated root name".to_owned())?;
    let mut network = Vec::with_capacity(payload.len() + 1);
    network.push(type_id);
    network.extend_from_slice(payload);
    let (tag, _) = NbtTag::decode_network(&network).map_err(|error| error.to_string())?;
    Ok(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty compound whose root name is `"""` (the world documents all
    /// carry an empty root name): `0a 00 00 00`.
    #[test]
    fn decodes_an_empty_named_compound() {
        let dsk = [0x0a_u8, 0x00, 0x00, 0x00];
        let tag = disk_nbt("mem", &dsk).expect("valid document");
        assert_eq!(tag, NbtTag::Compound(BTreeMap::new()));
    }

    use std::collections::BTreeMap;

    #[test]
    fn reports_a_truncated_document() {
        let error = disk_nbt("mem", &[]).expect_err("empty input must fail");
        assert!(error.to_string().contains("empty NBT document"));
    }
}

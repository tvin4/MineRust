//! Async packet framing over tokio streams.
//!
//! Two of its helpers are instrumented: [`read_tracked`] and
//! [`write_tracked`] frame a packet exactly like [`read_frame`] and
//! [`write_frame`], but report every packet to [`crate::trace`] — the
//! session recording and the ordered `debug` log of the live diagnostics.

use protocol::{check_frame_len, decode_varint, encode_frame, CodecError, ProtocolState};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::ConnectionError;
use crate::trace;

/// Reads one length-prefixed packet frame and returns its body (packet id +
/// payload).
///
/// Fails with [`CodecError::UnexpectedEof`] when the peer closes the
/// connection in the middle of a frame.
pub async fn read_frame<R>(reader: &mut R) -> Result<Vec<u8>, ConnectionError>
where
    R: AsyncRead + Unpin,
{
    // The length field itself must fit into 3 bytes, but up to the full VarInt
    // size is read so malformed input is reported instead of desynchronising.
    let mut length_bytes = [0u8; protocol::MAX_VARINT_BYTES];
    let mut read = 0usize;
    loop {
        let byte = reader.read_u8().await?;
        length_bytes[read] = byte;
        read += 1;
        if byte & 0x80 == 0 {
            break;
        }
        if read == length_bytes.len() {
            return Err(CodecError::VarIntTooLong.into());
        }
    }

    let (length, consumed) = decode_varint(&length_bytes[..read])?;
    let length = check_frame_len(length, consumed)?;

    let mut body = vec![0u8; length];
    if length > 0 {
        reader.read_exact(&mut body).await?;
    }
    Ok(body)
}

/// Writes one packet frame (length prefix + packet id + payload) and flushes.
pub async fn write_frame<W>(
    writer: &mut W,
    packet_id: u32,
    payload: &[u8],
) -> Result<(), ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    let frame = encode_frame(packet_id, payload)?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads one frame and records it as a packet of `state` received from the
/// client. The recording happens before the caller decodes the packet, so a
/// malformed payload still appears in the trace with its true size.
pub(crate) async fn read_tracked<R>(
    reader: &mut R,
    state: ProtocolState,
) -> Result<Vec<u8>, ConnectionError>
where
    R: AsyncRead + Unpin,
{
    let body = read_frame(reader).await?;
    let (id, consumed) = decode_varint(&body)?;
    let id = u32::try_from(id).map_err(|_| CodecError::InvalidPacketId(id))?;
    trace::received(state, id, &body[consumed..]);
    Ok(body)
}

/// Writes one frame and records it as a packet of `state` sent to the
/// client. Only what actually left the server is recorded, so a failed
/// write never shows up in the trace.
pub(crate) async fn write_tracked<W>(
    writer: &mut W,
    state: ProtocolState,
    packet_id: u32,
    payload: &[u8],
) -> Result<(), ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    write_frame(writer, packet_id, payload).await?;
    trace::sent(state, packet_id, payload);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::decode_frame;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn frame_roundtrips_over_a_stream() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        write_frame(&mut client, 0x00, &[0x2A])
            .await
            .expect("write");

        let body = read_frame(&mut server).await.expect("read");
        let (id, mut payload) = decode_frame(&body).expect("decode");
        assert_eq!(id, 0x00);
        assert_eq!(payload.read_varint().expect("payload"), 42);
        payload.ensure_empty().expect("no trailing bytes");
    }

    #[tokio::test]
    async fn truncated_frame_reports_eof() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        // Claims a 5-byte body but only sends 2 bytes before closing.
        client.write_all(&[0x05, 0x00, 0x01]).await.expect("write");
        drop(client);

        let err = read_frame(&mut server).await.expect_err("eof");
        assert!(err.is_client_disconnect(), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn length_field_over_three_bytes_is_rejected() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        // Non-canonical 4-byte encoding of the value 1.
        client
            .write_all(&[0x81, 0x80, 0x80, 0x00])
            .await
            .expect("write");
        drop(client);

        let err = read_frame(&mut server).await.expect_err("rejected");
        assert!(matches!(
            err,
            ConnectionError::Codec(CodecError::FrameLengthEncodingTooLong)
        ));
    }

    #[tokio::test]
    async fn overlong_length_field_is_rejected() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        client
            .write_all(&[0x80, 0x80, 0x80, 0x80, 0x80])
            .await
            .expect("write");
        drop(client);

        let err = read_frame(&mut server).await.expect_err("rejected");
        assert!(matches!(
            err,
            ConnectionError::Codec(CodecError::VarIntTooLong)
        ));
    }
}

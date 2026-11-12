//! Entity packets of the M4.5 visibility stream.
//!
//! One [`StreamEntities`](crate::game::StreamEntities) event is turned into
//! the packets a viewer needs: the spawn sequence of a newly visible player,
//! the movement updates and the removals. The shapes and the order come from
//! the two-bot vanilla trace (С1–С4, `docs/ARCH-M4.md` §6); where the trace
//! does not establish a value it is carried as recorded and marked «не
//! установлено».
//!
//! The bytes themselves are built by [`crate::entity_packets`] — the single
//! place they are constructed, shared with the Шаг 3.5 benchmark, so the two
//! never drift. This module only resolves the runtime packet ids and writes
//! the built payloads in order.
//!
//! SOURCE: two-bot vanilla trace of 26.3 (byte layout observed), not wiki.

use data::PacketIds;
use protocol::ProtocolState;
use tokio::io::AsyncWrite;

use crate::entity_packets::push_entity_stream;
use crate::error::ConnectionError;
use crate::frame::write_tracked;
use crate::game::{EntityMove, EntityRemove, EntitySpawn};

/// Writes every packet of one entity-stream event, in send order.
pub(super) async fn write_entity_stream<W>(
    writer: &mut W,
    ids: &PacketIds,
    spawn: &[EntitySpawn],
    remove: &[EntityRemove],
    moves: &[EntityMove],
) -> Result<(), ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    // The builder pushes `(key, payload)` in send order; the ids are resolved
    // here because only the connection knows its `PacketIds`.
    let mut packets = Vec::new();
    push_entity_stream(spawn, remove, moves, &mut packets)?;
    for (key, payload) in packets {
        write_tracked(writer, ProtocolState::Play, ids.id(key), &payload).await?;
    }
    Ok(())
}

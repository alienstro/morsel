//! The parts of each meeting that Ask searches: text, times, keyword index and embedding.

use crate::types::{MeetingId, Result};
use rusqlite::{Connection, params};

/// One part to store. Note parts have no times.
#[derive(Debug, Clone, PartialEq)]
pub struct NewPart {
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub text: String,
}

/// Deletes the old parts of the meeting, then stores `parts` in order. Returns their ids.
pub fn replace_parts(conn: &Connection, meeting: MeetingId, parts: &[NewPart]) -> Result<Vec<i64>> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM chunks WHERE meeting_id = ?1", [meeting])?;
    let mut ids = Vec::with_capacity(parts.len());
    {
        let mut insert = tx.prepare(
            "INSERT INTO chunks (meeting_id, part, start_ms, end_ms, text) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (i, p) in parts.iter().enumerate() {
            let start = p.start_ms.map(|v| v as i64);
            let end = p.end_ms.map(|v| v as i64);
            insert.execute(params![meeting, i as i64, start, end, p.text])?;
            ids.push(tx.last_insert_rowid());
        }
    }
    tx.commit()?;
    Ok(ids)
}

/// Parts that have no embedding yet, oldest first: (part id, meeting, text).
pub fn parts_without_embedding(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<(i64, MeetingId, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, meeting_id, text FROM chunks WHERE embedding IS NULL ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Saves the embedding of one part as little-endian `f32` values.
pub fn set_embedding(conn: &Connection, part: i64, vector: &[f32]) -> Result<()> {
    let blob: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
    conn.execute(
        "UPDATE chunks SET embedding = ?1 WHERE id = ?2",
        params![blob, part],
    )?;
    Ok(())
}

/// Reads an embedding that `set_embedding` saved.
pub fn decode_embedding(blob: &[u8]) -> Vec<f32> {
    blob.as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

/// Meetings with a complete transcript and no parts yet, oldest first. A meeting
/// whose notes failed or were cancelled still has its transcript, so it counts.
pub fn meetings_without_parts(conn: &Connection) -> Result<Vec<MeetingId>> {
    let mut stmt = conn.prepare(
        "SELECT m.id FROM meetings m \
         WHERE m.status IN ('done', 'notes_failed', 'notes_cancelled') \
         AND NOT EXISTS (SELECT 1 FROM chunks c WHERE c.meeting_id = m.id) \
         ORDER BY m.started_at, m.id",
    )?;
    let ids = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<MeetingId>>>()?;
    Ok(ids)
}

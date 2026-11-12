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

/// Deletes the old parts of the meeting, then stores `parts` in order.
pub fn replace_parts(conn: &Connection, meeting: MeetingId, parts: &[NewPart]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM chunks WHERE meeting_id = ?1", [meeting])?;
    {
        let mut insert = tx.prepare(
            "INSERT INTO chunks (meeting_id, part, start_ms, end_ms, text) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (i, p) in parts.iter().enumerate() {
            let start = p.start_ms.map(|v| v as i64);
            let end = p.end_ms.map(|v| v as i64);
            insert.execute(params![meeting, i as i64, start, end, p.text])?;
        }
    }
    tx.commit()?;
    Ok(())
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

/// Finished meetings that have no parts yet, oldest first.
pub fn meetings_without_parts(conn: &Connection) -> Result<Vec<MeetingId>> {
    let mut stmt = conn.prepare(
        "SELECT m.id FROM meetings m WHERE m.status = 'done' \
         AND NOT EXISTS (SELECT 1 FROM chunks c WHERE c.meeting_id = m.id) ORDER BY m.started_at",
    )?;
    let ids = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<MeetingId>>>()?;
    Ok(ids)
}

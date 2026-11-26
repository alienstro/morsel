//! Meetings, transcript lines, rough notes, finished notes and templates.

use super::library::reindex_meeting;
use super::now_ms;
use crate::settings::TranscribeTiming;
use crate::types::{MeetingId, MeetingStatus, MorselError, Result, Speaker, TranscriptLine};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub id: MeetingId,
    pub title: String,
    pub template_id: i64,
    pub template_name: String,
    pub status: MeetingStatus,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub people: Vec<String>,
    /// After mode: the audio time that the job has transcribed.
    pub transcribed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetail {
    pub summary: MeetingSummary,
    pub lines: Vec<TranscriptLine>,
    pub user_notes: String,
    pub note: Option<String>,
    /// The reason of the last notes failure, or `None` when the notes never failed.
    pub notes_error: Option<String>,
}

/// The title of a meeting that the user did not name.
pub const UNTITLED: &str = "Untitled meeting";

pub(crate) const SUMMARY_SQL: &str = "SELECT m.id, m.title, m.template_id, t.name, m.status, m.started_at, m.ended_at, m.people, m.transcribed_ms \
     FROM meetings m JOIN templates t ON t.id = m.template_id";

/// The raw columns of `SUMMARY_SQL`, before the status and the people are parsed.
pub(crate) struct SummaryRow {
    id: MeetingId,
    title: String,
    template_id: i64,
    template_name: String,
    status: String,
    started_at: i64,
    ended_at: Option<i64>,
    people: String,
    transcribed_ms: i64,
}

pub(crate) fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<SummaryRow> {
    Ok(SummaryRow {
        id: row.get(0)?,
        title: row.get(1)?,
        template_id: row.get(2)?,
        template_name: row.get(3)?,
        status: row.get(4)?,
        started_at: row.get(5)?,
        ended_at: row.get(6)?,
        people: row.get(7)?,
        transcribed_ms: row.get(8)?,
    })
}

pub(crate) fn to_summary(raw: SummaryRow) -> Result<MeetingSummary> {
    let people = serde_json::from_str(&raw.people)
        .map_err(|e| MorselError::Db(format!("meeting {} people: {e}", raw.id)))?;
    Ok(MeetingSummary {
        id: raw.id,
        title: raw.title,
        template_id: raw.template_id,
        template_name: raw.template_name,
        status: MeetingStatus::parse(&raw.status)?,
        started_at: raw.started_at,
        ended_at: raw.ended_at,
        people,
        transcribed_ms: raw.transcribed_ms.max(0) as u64,
    })
}

pub fn create_meeting(conn: &Connection, title: &str, template_id: i64) -> Result<MeetingId> {
    super::templates::get(conn, template_id)?;
    conn.execute(
        "INSERT INTO meetings (title, template_id, status, started_at) VALUES (?1, ?2, ?3, ?4)",
        params![
            title,
            template_id,
            MeetingStatus::Recording.as_str(),
            now_ms()
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn set_status(conn: &Connection, id: MeetingId, status: MeetingStatus) -> Result<()> {
    let ended = matches!(status, MeetingStatus::Stopped).then(now_ms);
    let changed = conn.execute(
        "UPDATE meetings SET status = ?1, ended_at = COALESCE(?2, ended_at) WHERE id = ?3",
        params![status.as_str(), ended, id],
    )?;
    if changed == 0 {
        return Err(MorselError::NotFound(format!("meeting {id}")));
    }
    reindex_meeting(conn, id)
}

/// Saves the reason of a notes failure. The note page shows it while the status is `notes_failed`.
pub fn set_notes_error(conn: &Connection, id: MeetingId, message: &str) -> Result<()> {
    conn.execute(
        "UPDATE meetings SET notes_error = ?1 WHERE id = ?2",
        params![message, id],
    )?;
    Ok(())
}

/// Saves the people of a calendar meeting. Empty names are dropped.
pub fn set_people(conn: &Connection, id: MeetingId, people: &[String]) -> Result<()> {
    let clean: Vec<&str> = people
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    let json = serde_json::to_string(&clean).map_err(|e| MorselError::Invalid(e.to_string()))?;
    conn.execute(
        "UPDATE meetings SET people = ?1 WHERE id = ?2",
        params![json, id],
    )?;
    Ok(())
}

pub fn set_title(conn: &Connection, id: MeetingId, title: &str) -> Result<()> {
    conn.execute(
        "UPDATE meetings SET title = ?1 WHERE id = ?2",
        params![title, id],
    )?;
    reindex_meeting(conn, id)
}

pub fn add_line(conn: &Connection, line: &TranscriptLine) -> Result<()> {
    conn.execute(
        "INSERT INTO transcript_lines (meeting_id, speaker, start_ms, end_ms, text) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![line.meeting_id, line.speaker.as_str(), line.start_ms as i64, line.end_ms as i64, line.text],
    )?;
    Ok(())
}

/// After mode: saves one line and the progress of the job in one transaction.
pub fn add_line_at(conn: &Connection, line: &TranscriptLine, transcribed_ms: u64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    add_line(&tx, line)?;
    set_transcribed_ms(&tx, line.meeting_id, transcribed_ms)?;
    tx.commit()?;
    Ok(())
}

pub fn set_transcribed_ms(conn: &Connection, id: MeetingId, ms: u64) -> Result<()> {
    conn.execute(
        "UPDATE meetings SET transcribed_ms = ?1 WHERE id = ?2",
        params![ms as i64, id],
    )?;
    Ok(())
}

/// The end of the last line of one speaker, or 0. The after job continues from here.
pub fn last_line_end(conn: &Connection, id: MeetingId, speaker: Speaker) -> Result<u64> {
    let end: Option<i64> = conn.query_row(
        "SELECT MAX(end_ms) FROM transcript_lines WHERE meeting_id = ?1 AND speaker = ?2",
        params![id, speaker.as_str()],
        |r| r.get(0),
    )?;
    Ok(end.unwrap_or(0).max(0) as u64)
}

pub fn set_timing(conn: &Connection, id: MeetingId, timing: TranscribeTiming) -> Result<()> {
    conn.execute(
        "UPDATE meetings SET transcribe_timing = ?1 WHERE id = ?2",
        params![timing.as_str(), id],
    )?;
    Ok(())
}

pub fn timing_of(conn: &Connection, id: MeetingId) -> Result<TranscribeTiming> {
    let value: String = conn.query_row(
        "SELECT transcribe_timing FROM meetings WHERE id = ?1",
        [id],
        |r| r.get(0),
    )?;
    Ok(TranscribeTiming::parse(&value))
}

/// The meetings with one status, oldest first.
pub fn list_by_status(conn: &Connection, status: MeetingStatus) -> Result<Vec<MeetingId>> {
    Ok(conn
        .prepare("SELECT id FROM meetings WHERE status = ?1 ORDER BY started_at, id")?
        .query_map([status.as_str()], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn save_user_notes(conn: &Connection, id: MeetingId, body: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO user_notes (meeting_id, body, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(meeting_id) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at",
        params![id, body, now_ms()],
    )?;
    reindex_meeting(conn, id)
}

pub fn save_note(conn: &Connection, id: MeetingId, body_md: &str, model: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO notes (meeting_id, body_md, model, created_at) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(meeting_id) DO UPDATE SET body_md = excluded.body_md, model = excluded.model, \
         created_at = excluded.created_at",
        params![id, body_md, model, now_ms()],
    )?;
    reindex_meeting(conn, id)
}

pub fn get_meeting(conn: &Connection, id: MeetingId) -> Result<MeetingDetail> {
    let raw = conn
        .query_row(
            &format!("{SUMMARY_SQL} WHERE m.id = ?1"),
            [id],
            summary_from_row,
        )
        .optional()?
        .ok_or_else(|| MorselError::NotFound(format!("meeting {id}")))?;
    let summary = to_summary(raw)?;

    let mut stmt = conn.prepare(
        "SELECT speaker, start_ms, end_ms, text FROM transcript_lines \
         WHERE meeting_id = ?1 ORDER BY start_ms, id",
    )?;
    let raws = stmt
        .query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let lines = raws
        .into_iter()
        .map(|(speaker, start, end, text)| {
            Ok(TranscriptLine {
                meeting_id: id,
                speaker: Speaker::parse(&speaker)?,
                start_ms: start as u64,
                end_ms: end as u64,
                text,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let user_notes = conn
        .query_row(
            "SELECT body FROM user_notes WHERE meeting_id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default();
    let note = conn
        .query_row(
            "SELECT body_md FROM notes WHERE meeting_id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let notes_error = conn.query_row(
        "SELECT notes_error FROM meetings WHERE id = ?1",
        [id],
        |r| r.get::<_, Option<String>>(0),
    )?;

    Ok(MeetingDetail {
        summary,
        lines,
        user_notes,
        note,
        notes_error,
    })
}

pub fn list_meetings(conn: &Connection, limit: u32) -> Result<Vec<MeetingSummary>> {
    let mut stmt = conn.prepare(&format!(
        "{SUMMARY_SQL} ORDER BY m.started_at DESC, m.id DESC LIMIT ?1"
    ))?;
    let raws = stmt
        .query_map([limit], summary_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raws.into_iter().map(to_summary).collect()
}

/// Marks meetings that were recording or paused when the app stopped, and indexes their
/// transcript for search (lines are not indexed while a meeting records). Returns the count.
pub fn mark_open_meetings_interrupted(conn: &Connection) -> Result<u32> {
    let open: Vec<MeetingId> = conn
        .prepare("SELECT id FROM meetings WHERE status IN (?1, ?2)")?
        .query_map(
            params![
                MeetingStatus::Recording.as_str(),
                MeetingStatus::Paused.as_str()
            ],
            |r| r.get(0),
        )?
        .collect::<rusqlite::Result<_>>()?;
    for id in &open {
        set_status(conn, *id, MeetingStatus::Interrupted)?;
    }
    Ok(open.len() as u32)
}

/// Saves the token counts of one model request.
pub fn record_usage(
    conn: &Connection,
    model: &str,
    prompt_tokens: u32,
    completion_tokens: u32,
    cost: Option<f64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO usage (at, model, prompt_tokens, completion_tokens, cost) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![now_ms(), model, prompt_tokens, completion_tokens, cost],
    )?;
    Ok(())
}

//! Meetings, transcript lines, rough notes, finished notes and templates.

use super::now_ms;
use crate::types::{MeetingId, MeetingStatus, MorselError, Result, Speaker, TranscriptLine};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: i64,
    pub name: String,
    pub sections: Vec<String>,
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub id: MeetingId,
    pub title: String,
    pub template_name: String,
    pub status: MeetingStatus,
    pub started_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetail {
    pub summary: MeetingSummary,
    pub lines: Vec<TranscriptLine>,
    pub user_notes: String,
    pub note: Option<String>,
}

/// The title of a meeting that the user did not name.
pub const UNTITLED: &str = "Untitled meeting";

const SUMMARY_SQL: &str = "SELECT m.id, m.title, t.name, m.status, m.started_at \
     FROM meetings m JOIN templates t ON t.id = m.template_id";

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<(MeetingId, String, String, String, i64)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn to_summary(raw: (MeetingId, String, String, String, i64)) -> Result<MeetingSummary> {
    Ok(MeetingSummary {
        id: raw.0,
        title: raw.1,
        template_name: raw.2,
        status: MeetingStatus::parse(&raw.3)?,
        started_at: raw.4,
    })
}

fn template_from_row(row: &Row<'_>) -> rusqlite::Result<(i64, String, String, String)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn to_template(raw: (i64, String, String, String)) -> Result<Template> {
    let sections = serde_json::from_str(&raw.2)
        .map_err(|e| MorselError::Db(format!("template {} sections: {e}", raw.0)))?;
    Ok(Template {
        id: raw.0,
        name: raw.1,
        sections,
        instructions: raw.3,
    })
}

pub fn list_templates(conn: &Connection) -> Result<Vec<Template>> {
    let mut stmt =
        conn.prepare("SELECT id, name, sections_json, instructions FROM templates ORDER BY id")?;
    let raws = stmt
        .query_map([], template_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raws.into_iter().map(to_template).collect()
}

pub fn get_template(conn: &Connection, id: i64) -> Result<Template> {
    let raw = conn
        .query_row(
            "SELECT id, name, sections_json, instructions FROM templates WHERE id = ?1",
            [id],
            template_from_row,
        )
        .optional()?
        .ok_or_else(|| MorselError::NotFound(format!("template {id}")))?;
    to_template(raw)
}

pub fn create_meeting(conn: &Connection, title: &str, template_id: i64) -> Result<MeetingId> {
    get_template(conn, template_id)?;
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
    Ok(())
}

pub fn set_title(conn: &Connection, id: MeetingId, title: &str) -> Result<()> {
    conn.execute(
        "UPDATE meetings SET title = ?1 WHERE id = ?2",
        params![title, id],
    )?;
    Ok(())
}

pub fn add_line(conn: &Connection, line: &TranscriptLine) -> Result<()> {
    conn.execute(
        "INSERT INTO transcript_lines (meeting_id, speaker, start_ms, end_ms, text) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![line.meeting_id, line.speaker.as_str(), line.start_ms as i64, line.end_ms as i64, line.text],
    )?;
    Ok(())
}

pub fn save_user_notes(conn: &Connection, id: MeetingId, body: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO user_notes (meeting_id, body, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(meeting_id) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at",
        params![id, body, now_ms()],
    )?;
    Ok(())
}

pub fn save_note(conn: &Connection, id: MeetingId, body_md: &str, model: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO notes (meeting_id, body_md, model, created_at) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(meeting_id) DO UPDATE SET body_md = excluded.body_md, model = excluded.model, \
         created_at = excluded.created_at",
        params![id, body_md, model, now_ms()],
    )?;
    Ok(())
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

    Ok(MeetingDetail {
        summary,
        lines,
        user_notes,
        note,
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

/// Marks meetings that were recording or paused when the app stopped. Returns the count.
pub fn mark_open_meetings_interrupted(conn: &Connection) -> Result<u32> {
    let changed = conn.execute(
        "UPDATE meetings SET status = ?1 WHERE status IN (?2, ?3)",
        params![
            MeetingStatus::Interrupted.as_str(),
            MeetingStatus::Recording.as_str(),
            MeetingStatus::Paused.as_str()
        ],
    )?;
    Ok(changed as u32)
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

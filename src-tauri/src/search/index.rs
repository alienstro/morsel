//! Keeps the search parts of each meeting, with their embeddings, up to date.

use crate::db::SharedDb;
use crate::db::meetings::get_meeting;
use crate::db::parts::{
    meetings_without_parts, parts_without_embedding, replace_parts, set_embedding,
};
use crate::events::{EventSink, SEARCH_PROGRESS};
use crate::search::embed::part_text;
use crate::search::parts::meeting_parts;
use crate::types::{MeetingId, MorselError, Result};
use serde_json::json;

/// The parts that go to the search model in one call.
const BATCH: usize = 16;

/// Turns texts into normalized vectors. `None` in the functions below means that
/// the search model is not there, so only the keyword index is built.
pub trait Embed: Send + Sync {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// Replaces the parts of one meeting, and embeds them when `embed` is there.
pub fn index_meeting(db: &SharedDb, embed: Option<&dyn Embed>, meeting: MeetingId) -> Result<()> {
    let (title, ids, texts) = {
        let conn = db.lock().expect("db lock");
        let detail = get_meeting(&conn, meeting)?;
        let note = detail
            .note
            .or_else(|| (!detail.user_notes.trim().is_empty()).then_some(detail.user_notes));
        let parts = meeting_parts(&detail.lines, note.as_deref());
        let ids = replace_parts(&conn, meeting, &parts)?;
        let texts: Vec<String> = parts.into_iter().map(|p| p.text).collect();
        (detail.summary.title, ids, texts)
    };
    let Some(embed) = embed else {
        return Ok(());
    };
    for (ids, texts) in ids.chunks(BATCH).zip(texts.chunks(BATCH)) {
        let labelled: Vec<String> = texts.iter().map(|t| part_text(&title, t)).collect();
        save_vectors(db, ids, embed.embed(&labelled)?)?;
    }
    Ok(())
}

fn save_vectors(db: &SharedDb, ids: &[i64], vectors: Vec<Vec<f32>>) -> Result<()> {
    if vectors.len() != ids.len() {
        return Err(MorselError::Invalid(format!(
            "the search model gave {} vectors for {} parts",
            vectors.len(),
            ids.len()
        )));
    }
    let conn = db.lock().expect("db lock");
    for (id, vector) in ids.iter().zip(vectors) {
        set_embedding(&conn, *id, &vector)?;
    }
    Ok(())
}

fn title_of(db: &SharedDb, meeting: MeetingId) -> Result<String> {
    let conn = db.lock().expect("db lock");
    Ok(
        conn.query_row("SELECT title FROM meetings WHERE id = ?1", [meeting], |r| {
            r.get(0)
        })?,
    )
}

/// Indexes the finished meetings that have no parts, then embeds the parts that
/// have no embedding. Sends `search://progress` after each step.
pub fn index_all(db: &SharedDb, embed: Option<&dyn Embed>, sink: &dyn EventSink) -> Result<()> {
    let meetings = meetings_without_parts(&db.lock().expect("db lock"))?;
    let total = meetings.len();
    for (i, meeting) in meetings.into_iter().enumerate() {
        index_meeting(db, embed, meeting)?;
        sink.emit(SEARCH_PROGRESS, json!({ "done": i + 1, "total": total }));
    }
    let Some(embed) = embed else {
        return Ok(());
    };
    let total: i64 = db.lock().expect("db lock").query_row(
        "SELECT count(*) FROM chunks WHERE embedding IS NULL",
        [],
        |r| r.get(0),
    )?;
    let mut done = 0;
    loop {
        let batch = parts_without_embedding(&db.lock().expect("db lock"), BATCH)?;
        if batch.is_empty() {
            return Ok(());
        }
        let mut labelled = Vec::with_capacity(batch.len());
        for (_, meeting, text) in &batch {
            labelled.push(part_text(&title_of(db, *meeting)?, text));
        }
        let ids: Vec<i64> = batch.iter().map(|b| b.0).collect();
        save_vectors(db, &ids, embed.embed(&labelled)?)?;
        done += ids.len();
        sink.emit(SEARCH_PROGRESS, json!({ "done": done, "total": total }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::meetings::{add_line, create_meeting, set_status};
    use crate::events::{SEARCH_PROGRESS, VecSink};
    use crate::types::{MeetingStatus, Speaker, TranscriptLine};
    use std::sync::{Arc, Mutex};

    /// Gives every text the same vector, and keeps the texts it saw.
    #[derive(Default)]
    struct FakeEmbed(Mutex<Vec<String>>);

    impl Embed for FakeEmbed {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.0.lock().unwrap().extend(texts.iter().cloned());
            Ok(texts.iter().map(|_| vec![1.0, 0.0, 0.0]).collect())
        }
    }

    fn db() -> (tempfile::TempDir, SharedDb) {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&dir.path().join("m.db")).unwrap();
        (dir, Arc::new(Mutex::new(conn)))
    }

    fn done_meeting(db: &SharedDb, title: &str) -> MeetingId {
        let conn = db.lock().unwrap();
        let id = create_meeting(&conn, title, 1).unwrap();
        for i in 0..3u64 {
            let line = TranscriptLine {
                meeting_id: id,
                speaker: Speaker::You,
                start_ms: i * 5_000,
                end_ms: i * 5_000 + 4_000,
                text: format!("the budget point {i}"),
            };
            add_line(&conn, &line).unwrap();
        }
        set_status(&conn, id, MeetingStatus::Done).unwrap();
        id
    }

    /// (parts, parts without an embedding) of one meeting.
    fn counts(db: &SharedDb, id: MeetingId) -> (i64, i64) {
        db.lock()
            .unwrap()
            .query_row(
                "SELECT count(*), count(*) - count(embedding) FROM chunks WHERE meeting_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    #[test]
    fn index_meeting_stores_parts_with_embeddings() {
        let (_d, db) = db();
        let id = done_meeting(&db, "Q1 sync");
        let embed = FakeEmbed::default();
        index_meeting(&db, Some(&embed), id).unwrap();
        let (parts, missing) = counts(&db, id);
        assert!(parts > 0);
        assert_eq!(missing, 0);
        let seen = embed.0.lock().unwrap();
        assert!(
            seen[0].starts_with("title: Q1 sync | text: "),
            "{}",
            seen[0]
        );
    }

    #[test]
    fn index_meeting_without_a_model_stores_parts_only() {
        let (_d, db) = db();
        let id = done_meeting(&db, "Q1 sync");
        index_meeting(&db, None, id).unwrap();
        let (parts, missing) = counts(&db, id);
        assert!(parts > 0);
        assert_eq!(missing, parts);
    }

    #[test]
    fn index_all_fills_missing_embeddings_when_the_model_arrives() {
        let (_d, db) = db();
        let id = done_meeting(&db, "Q1 sync");
        index_all(&db, None, &VecSink::default()).unwrap();
        assert!(counts(&db, id).0 > 0);
        assert_eq!(counts(&db, id).1, counts(&db, id).0);
        index_all(&db, Some(&FakeEmbed::default()), &VecSink::default()).unwrap();
        assert_eq!(counts(&db, id).1, 0);
    }

    #[test]
    fn index_all_reports_progress() {
        let (_d, db) = db();
        done_meeting(&db, "One");
        done_meeting(&db, "Two");
        let sink = VecSink::default();
        index_all(&db, None, &sink).unwrap();
        let events = sink.named(SEARCH_PROGRESS);
        assert_eq!(events.first().unwrap()["total"], 2);
        assert_eq!(events.last().unwrap()["done"], 2);
    }
}

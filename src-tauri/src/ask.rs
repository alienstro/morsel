//! Ask Morsel: finds the best meetings for a question, and lets the quick model answer from them only.

use crate::db::library::{fts_query, save_ask};
use crate::db::meetings::{get_meeting, record_usage};
use crate::db::{SharedDb, now_ms};
use crate::llm::{ChatMessage, LlmConfig, chat};
use crate::notes::prompt::transcript_text;
use crate::types::{MeetingId, MorselError, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

/// The most meetings that one answer reads.
pub const MAX_MEETINGS: usize = 8;
/// The token budget of the meetings that go to the model (estimate: characters ÷ 4).
pub const ASK_TOKENS: usize = 100_000;
/// The answer when no meeting matches. No model call happens then.
pub const NO_MATCH: &str = "No meeting matches this question.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AskRange {
    All,
    Week,
    Month,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskSource {
    pub meeting_id: MeetingId,
    pub title: String,
    pub started_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskAnswer {
    /// The saved row. `None` for a question about one meeting, which is not saved.
    pub id: Option<i64>,
    pub question: String,
    pub answer: String,
    pub sources: Vec<AskSource>,
}

const DAY_MS: i64 = 86_400_000;

const SYSTEM: &str = "You answer questions about the user's meetings.\n\
     Use only the meetings below. If they do not hold the answer, say so in one sentence.\n\
     Keep the answer short: two to five sentences, or a short list.\n\
     Name the meeting when a fact comes from one meeting.\n\
     \"You\" in a transcript is the user. \"Others\" is everyone else in the call.";

impl AskRange {
    /// The earliest start time that the range includes.
    fn since(self, now_ms: i64) -> i64 {
        match self {
            AskRange::All => i64::MIN,
            AskRange::Week => now_ms - 7 * DAY_MS,
            AskRange::Month => now_ms - 30 * DAY_MS,
        }
    }
}

/// The best meetings for a question, best first, with SQLite FTS5 over notes and transcripts.
pub fn find_meetings(
    conn: &Connection,
    question: &str,
    range: AskRange,
    now_ms: i64,
) -> Result<Vec<MeetingId>> {
    let Some(query) = fts_query(question) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(
        "SELECT m.id FROM meeting_search s JOIN meetings m ON m.id = s.rowid \
         WHERE meeting_search MATCH ?1 AND m.started_at >= ?2 \
         ORDER BY bm25(meeting_search) LIMIT ?3",
    )?;
    let ids = stmt
        .query_map(
            params![query, range.since(now_ms), MAX_MEETINGS as i64],
            |r| r.get(0),
        )?
        .collect::<rusqlite::Result<Vec<MeetingId>>>()?;
    Ok(ids)
}

/// The first `max_chars` characters of a text.
fn cut(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}

/// One meeting as context text, inside its share of the token budget.
fn meeting_block(conn: &Connection, id: MeetingId, max_chars: usize) -> Result<(AskSource, String)> {
    let detail = get_meeting(conn, id)?;
    let date: String = conn.query_row(
        "SELECT date(?1 / 1000, 'unixepoch')",
        [detail.summary.started_at],
        |r| r.get(0),
    )?;
    let source = AskSource {
        meeting_id: id,
        title: detail.summary.title.clone(),
        started_at: detail.summary.started_at,
    };
    let note = detail.note.unwrap_or(detail.user_notes);
    let head = format!("Meeting: {} ({date})\nNotes:\n", detail.summary.title);
    let note = cut(&note, max_chars.saturating_sub(head.len()));
    let mut block = format!("{head}{note}\n");
    let left = max_chars.saturating_sub(block.chars().count() + 12);
    if left > 0 && !detail.lines.is_empty() {
        block.push_str("Transcript:\n");
        block.push_str(cut(&transcript_text(&detail.lines), left));
        block.push('\n');
    }
    Ok((source, block))
}

/// Answers a question from the best meetings, or from one meeting when `meeting` is set.
/// A question across meetings is saved; a question about one meeting is not.
pub async fn ask(
    db: &SharedDb,
    http: &reqwest::Client,
    quick: &LlmConfig,
    question: &str,
    range: AskRange,
    meeting: Option<MeetingId>,
) -> Result<AskAnswer> {
    let question = question.trim().to_owned();
    if question.is_empty() {
        return Err(MorselError::Invalid("type a question first".into()));
    }
    let (sources, blocks) = {
        let conn = db.lock().expect("db lock");
        let ids = match meeting {
            Some(id) => vec![id],
            None => find_meetings(&conn, &question, range, now_ms())?,
        };
        let share = if ids.is_empty() {
            0
        } else {
            ASK_TOKENS * 4 / ids.len()
        };
        let mut sources = Vec::with_capacity(ids.len());
        let mut blocks = Vec::with_capacity(ids.len());
        for id in ids {
            let (source, block) = meeting_block(&conn, id, share)?;
            sources.push(source);
            blocks.push(block);
        }
        (sources, blocks)
    };

    let answer = if sources.is_empty() {
        NO_MATCH.to_owned()
    } else {
        let messages = [
            ChatMessage::system(SYSTEM),
            ChatMessage::user(format!(
                "Meetings:\n\n{}\nQuestion: {question}",
                blocks.join("\n---\n")
            )),
        ];
        let result = chat(http, quick, &messages).await?;
        record_usage(
            &db.lock().expect("db lock"),
            &quick.model,
            result.prompt_tokens,
            result.completion_tokens,
            result.cost,
        )?;
        result.text.trim().to_owned()
    };

    let mut out = AskAnswer {
        id: None,
        question,
        answer,
        sources,
    };
    if meeting.is_none() {
        out.id = Some(save_ask(&db.lock().expect("db lock"), &out)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::library::{get_ask, list_asks};
    use crate::db::meetings::{add_line, create_meeting, save_note, set_status};
    use crate::types::{MeetingStatus, Speaker, TranscriptLine};
    use rusqlite::params;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const DAY: i64 = 86_400_000;

    fn temp_db() -> (tempfile::TempDir, SharedDb) {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&dir.path().join("m.db")).unwrap();
        (dir, Arc::new(Mutex::new(conn)))
    }

    fn meeting(db: &SharedDb, title: &str, started_at: i64, note: &str) -> MeetingId {
        let conn = db.lock().unwrap();
        let id = create_meeting(&conn, title, 1).unwrap();
        conn.execute(
            "UPDATE meetings SET started_at = ?1 WHERE id = ?2",
            params![started_at, id],
        )
        .unwrap();
        save_note(&conn, id, note, "m").unwrap();
        set_status(&conn, id, MeetingStatus::Done).unwrap();
        id
    }

    fn config(server: &MockServer) -> LlmConfig {
        LlmConfig {
            base_url: server.uri(),
            model: "quick".into(),
            api_key: "k".into(),
        }
    }

    fn reply(text: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "role": "assistant", "content": text } }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
        }))
    }

    #[test]
    fn finds_best_meetings_for_question() {
        let (_d, db) = temp_db();
        let now = now_ms();
        let pricing = meeting(&db, "Pricing", now - DAY, "Acme wants a discount.");
        meeting(&db, "Roadmap", now - DAY, "Ship in March.");
        let conn = db.lock().unwrap();
        let found = find_meetings(&conn, "What about the discount?", AskRange::All, now).unwrap();
        assert_eq!(found, vec![pricing]);
        assert!(find_meetings(&conn, "?? **", AskRange::All, now).unwrap().is_empty());
    }

    #[test]
    fn range_week_skips_old_meetings() {
        let (_d, db) = temp_db();
        let now = now_ms();
        meeting(&db, "Old", now - 10 * DAY, "discount");
        let recent = meeting(&db, "New", now - DAY, "discount");
        let conn = db.lock().unwrap();
        assert_eq!(
            find_meetings(&conn, "discount", AskRange::Week, now).unwrap(),
            vec![recent]
        );
        assert_eq!(
            find_meetings(&conn, "discount", AskRange::Month, now)
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn ask_sends_only_found_meetings_and_returns_sources() {
        let (_d, db) = temp_db();
        let now = now_ms();
        let pricing = meeting(&db, "Pricing call", now - DAY, "Acme wants a discount.");
        meeting(&db, "Roadmap", now - DAY, "Ship in March.");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply("Acme asked for 10% off."))
            .expect(1)
            .mount(&server)
            .await;

        let answer = ask(
            &db,
            &reqwest::Client::new(),
            &config(&server),
            " What did Acme say about the discount? ",
            AskRange::All,
            None,
        )
        .await
        .unwrap();

        assert_eq!(answer.answer, "Acme asked for 10% off.");
        assert_eq!(answer.question, "What did Acme say about the discount?");
        assert_eq!(answer.sources.len(), 1);
        assert_eq!(answer.sources[0].meeting_id, pricing);
        assert_eq!(answer.sources[0].title, "Pricing call");
        let body = String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
        assert!(body.contains("Pricing call"));
        assert!(!body.contains("Roadmap"));

        let id = answer.id.expect("saved");
        assert_eq!(get_ask(&db.lock().unwrap(), id).unwrap(), answer);
        let usage: i64 = db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM usage", [], |r| r.get(0))
            .unwrap();
        assert_eq!(usage, 1);
    }

    #[tokio::test]
    async fn ask_with_no_meetings_makes_no_model_call() {
        let (_d, db) = temp_db();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply("never"))
            .expect(0)
            .mount(&server)
            .await;
        let answer = ask(
            &db,
            &reqwest::Client::new(),
            &config(&server),
            "What about the discount?",
            AskRange::All,
            None,
        )
        .await
        .unwrap();
        assert_eq!(answer.answer, NO_MATCH);
        assert!(answer.sources.is_empty());
    }

    #[tokio::test]
    async fn ask_context_stays_under_the_limit() {
        let (_d, db) = temp_db();
        let id = meeting(&db, "Marathon", now_ms() - DAY, "discount");
        {
            let conn = db.lock().unwrap();
            let text = "discount talk ".repeat(500);
            for i in 0..100u64 {
                add_line(
                    &conn,
                    &TranscriptLine {
                        meeting_id: id,
                        speaker: Speaker::Others,
                        start_ms: i * 1000,
                        end_ms: i * 1000 + 900,
                        text: text.clone(),
                    },
                )
                .unwrap();
            }
        }
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply("ok"))
            .mount(&server)
            .await;
        ask(
            &db,
            &reqwest::Client::new(),
            &config(&server),
            "discount",
            AskRange::All,
            None,
        )
        .await
        .unwrap();
        let body = &server.received_requests().await.unwrap()[0].body;
        assert!(body.len() / 4 < ASK_TOKENS + 2_000, "body is {} bytes", body.len());
    }

    #[tokio::test]
    async fn meeting_ask_is_not_saved() {
        let (_d, db) = temp_db();
        let id = meeting(&db, "Standup", now_ms() - DAY, "Leo is blocked.");
        meeting(&db, "Other", now_ms() - DAY, "Leo is fine.");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply("Leo is blocked."))
            .mount(&server)
            .await;
        let answer = ask(
            &db,
            &reqwest::Client::new(),
            &config(&server),
            "Who is blocked?",
            AskRange::All,
            Some(id),
        )
        .await
        .unwrap();
        assert_eq!(answer.id, None);
        assert_eq!(answer.sources.len(), 1);
        assert_eq!(answer.sources[0].meeting_id, id);
        assert!(list_asks(&db.lock().unwrap(), 10).unwrap().is_empty());
        let body = String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
        assert!(!body.contains("Leo is fine"));
    }
}

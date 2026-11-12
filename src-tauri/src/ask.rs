//! Ask Morsel: finds the best meetings for a question, and lets the quick model answer from them only.

use crate::chat_model::ChatModel;
use crate::db::library::{fts_query, save_ask};
use crate::db::meetings::{get_meeting, record_usage};
use crate::db::{SharedDb, now_ms};
use crate::llm::ChatMessage;
use crate::notes::prompt::transcript_text;
use crate::search::embed::query_text;
use crate::search::hybrid::{PartHit, search_parts};
use crate::search::index::Embed;
use crate::types::{MeetingId, MorselError, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
    pub fn since(self, now_ms: i64) -> i64 {
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
fn meeting_block(
    conn: &Connection,
    id: MeetingId,
    max_chars: usize,
) -> Result<(AskSource, String)> {
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

/// The embedding of the question, or `None` when the search model fails: then Ask
/// uses keywords only.
async fn question_vector(embed: Arc<dyn Embed>, question: &str) -> Option<Vec<f32>> {
    let text = query_text(question);
    let job = tokio::task::spawn_blocking(move || embed.embed(&[text]));
    match job.await {
        Ok(Ok(mut vectors)) if !vectors.is_empty() => Some(vectors.remove(0)),
        Ok(Ok(_)) => None,
        Ok(Err(err)) => {
            eprintln!("the search model failed, so Ask uses keywords only: {err}");
            None
        }
        Err(err) => {
            eprintln!("the search model job stopped, so Ask uses keywords only: {err}");
            None
        }
    }
}

/// Whole meetings, each with an equal share of `budget_chars`.
fn meeting_blocks(
    conn: &Connection,
    ids: &[MeetingId],
    budget_chars: usize,
) -> Result<(Vec<AskSource>, Vec<String>)> {
    let share = if ids.is_empty() {
        0
    } else {
        budget_chars / ids.len()
    };
    let mut sources = Vec::with_capacity(ids.len());
    let mut blocks = Vec::with_capacity(ids.len());
    for &id in ids {
        let (source, block) = meeting_block(conn, id, share)?;
        sources.push(source);
        blocks.push(block);
    }
    Ok((sources, blocks))
}

/// The found parts in rank order, as long as they fit in `budget_chars`, grouped by
/// meeting. Each meeting is one source, in the order of its best part.
fn part_blocks(
    conn: &Connection,
    hits: &[PartHit],
    budget_chars: usize,
) -> Result<(Vec<AskSource>, Vec<String>)> {
    let mut order: Vec<MeetingId> = Vec::new();
    let mut texts: Vec<Vec<&str>> = Vec::new();
    let mut used = 0;
    for hit in hits {
        let len = hit.text.chars().count();
        if used + len > budget_chars {
            continue;
        }
        used += len;
        match order.iter().position(|&m| m == hit.meeting) {
            Some(i) => texts[i].push(&hit.text),
            None => {
                order.push(hit.meeting);
                texts.push(vec![&hit.text]);
            }
        }
    }
    let mut sources = Vec::with_capacity(order.len());
    let mut blocks = Vec::with_capacity(order.len());
    for (id, parts) in order.into_iter().zip(texts) {
        let (title, started_at, date): (String, i64, String) = conn.query_row(
            "SELECT title, started_at, date(started_at / 1000, 'unixepoch') FROM meetings WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        blocks.push(format!("Meeting: {title} ({date})\n{}\n", parts.join("\n")));
        sources.push(AskSource {
            meeting_id: id,
            title,
            started_at,
        });
    }
    Ok((sources, blocks))
}

/// Answers a question from the best meetings, or from one meeting when `meeting` is set.
/// A question across meetings is saved; a question about one meeting is not.
pub async fn ask(
    db: &SharedDb,
    model: &dyn ChatModel,
    embed: Option<Arc<dyn Embed>>,
    question: &str,
    range: AskRange,
    meeting: Option<MeetingId>,
) -> Result<AskAnswer> {
    let question = question.trim().to_owned();
    if question.is_empty() {
        return Err(MorselError::Invalid("type a question first".into()));
    }
    let query = match (meeting, embed) {
        (None, Some(embed)) => question_vector(embed, &question).await,
        _ => None,
    };
    let (sources, blocks) = {
        let conn = db.lock().expect("db lock");
        let budget_chars = model.budget().ask * 4;
        let hits = match meeting {
            Some(_) => Vec::new(),
            None => search_parts(&conn, &question, query.as_deref(), range, now_ms())?,
        };
        if hits.is_empty() {
            let ids = match meeting {
                Some(id) => vec![id],
                None => find_meetings(&conn, &question, range, now_ms())?,
            };
            meeting_blocks(&conn, &ids, budget_chars)?
        } else {
            part_blocks(&conn, &hits, budget_chars)?
        }
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
        let result = model
            .chat(&messages, &tokio_util::sync::CancellationToken::new())
            .await?;
        record_usage(
            &db.lock().expect("db lock"),
            model.name(),
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

    fn config(server: &MockServer) -> crate::chat_model::CloudChat {
        crate::chat_model::CloudChat::new(
            reqwest::Client::new(),
            crate::llm::LlmConfig {
                base_url: server.uri(),
                model: "quick".into(),
                api_key: "k".into(),
            },
        )
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
        assert!(
            find_meetings(&conn, "?? **", AskRange::All, now)
                .unwrap()
                .is_empty()
        );
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

    /// A finished meeting with 120 transcript lines about the roadmap, and one line,
    /// line 60, from `special`. Its parts are in the search index.
    fn long_meeting(db: &SharedDb, title: &str, special: &str) -> MeetingId {
        let id = meeting(db, title, now_ms() - DAY, "");
        {
            let conn = db.lock().unwrap();
            conn.execute("DELETE FROM notes WHERE meeting_id = ?1", [id])
                .unwrap();
            for i in 0..120u64 {
                let text = if i == 60 {
                    special.to_owned()
                } else {
                    format!("line {i:03} about the roadmap")
                };
                let line = TranscriptLine {
                    meeting_id: id,
                    speaker: Speaker::Others,
                    start_ms: i * 5_000,
                    end_ms: i * 5_000 + 4_000,
                    text,
                };
                add_line(&conn, &line).unwrap();
            }
        }
        crate::search::index::index_meeting(db, None, id).unwrap();
        id
    }

    /// The user message of the one model call.
    async fn sent_context(server: &MockServer) -> String {
        let body: serde_json::Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        body["messages"][1]["content"].as_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn ask_sends_only_the_found_parts_grouped_by_meeting() {
        let (_d, db) = temp_db();
        long_meeting(&db, "Pricing call", "Acme wants a discount");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply("10% off."))
            .mount(&server)
            .await;
        ask(&db, &config(&server), None, "discount", AskRange::All, None)
            .await
            .unwrap();
        let context = sent_context(&server).await;
        assert!(context.contains("Meeting: Pricing call"), "{context}");
        assert!(context.contains("Acme wants a discount"));
        assert!(!context.contains("line 000 "), "{context}");
        assert!(!context.contains("line 119 "), "{context}");
    }

    #[tokio::test]
    async fn ask_lists_each_source_meeting_once() {
        let (_d, db) = temp_db();
        let id = long_meeting(&db, "Roadmap", "the roadmap again");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply("Ship it."))
            .mount(&server)
            .await;
        let answer = ask(&db, &config(&server), None, "roadmap", AskRange::All, None)
            .await
            .unwrap();
        assert_eq!(answer.sources.len(), 1);
        assert_eq!(answer.sources[0].meeting_id, id);
    }

    #[tokio::test]
    async fn ask_without_the_search_model_uses_keywords() {
        let (_d, db) = temp_db();
        let id = long_meeting(&db, "Pricing call", "Acme wants a discount");
        long_meeting(&db, "Other", "nothing to see");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply("10% off."))
            .mount(&server)
            .await;
        let answer = ask(&db, &config(&server), None, "discount", AskRange::All, None)
            .await
            .unwrap();
        let ids: Vec<MeetingId> = answer.sources.iter().map(|s| s.meeting_id).collect();
        assert_eq!(ids, vec![id]);
    }

    /// Points texts about the launch one way, and all other texts another way.
    struct LaunchEmbed;

    impl Embed for LaunchEmbed {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| {
                    if t.contains("launch") || t.contains("Marso") {
                        vec![1.0, 0.0]
                    } else {
                        vec![0.0, 1.0]
                    }
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn ask_finds_a_part_by_meaning_with_the_search_model() {
        let (_d, db) = temp_db();
        let id = long_meeting(&db, "Sync", "ilipat natin sa Marso 29");
        crate::search::index::index_meeting(&db, Some(&LaunchEmbed), id).unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply("March 29."))
            .mount(&server)
            .await;
        ask(
            &db,
            &config(&server),
            Some(Arc::new(LaunchEmbed)),
            "when is the launch?",
            AskRange::All,
            None,
        )
        .await
        .unwrap();
        let context = sent_context(&server).await;
        assert!(context.contains("ilipat natin sa Marso 29"), "{context}");
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
            &config(&server),
            None,
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
        let body =
            String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
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
            &config(&server),
            None,
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
    async fn ask_uses_the_model_budget() {
        use crate::chat_model::{Budget, FakeChat};
        let (_d, db) = temp_db();
        let id = meeting(&db, "Marathon", now_ms() - DAY, "discount");
        {
            let conn = db.lock().unwrap();
            let text = "discount talk ".repeat(500);
            for i in 0..20u64 {
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
        let fake = FakeChat::new(Budget::LOCAL, "ok");
        ask(&db, &fake, None, "discount", AskRange::All, None)
            .await
            .unwrap();
        let calls = fake.calls();
        let chars: usize = calls[0].iter().map(|m| m.content.len()).sum();
        assert!(chars / 4 < 6_000 + 500, "the prompt has {chars} characters");
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
        ask(&db, &config(&server), None, "discount", AskRange::All, None)
            .await
            .unwrap();
        let body = &server.received_requests().await.unwrap()[0].body;
        assert!(
            body.len() / 4 < ASK_TOKENS + 2_000,
            "body is {} bytes",
            body.len()
        );
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
            &config(&server),
            None,
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
        let body =
            String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
        assert!(!body.contains("Leo is fine"));
    }
}

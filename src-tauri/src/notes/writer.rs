//! Runs the notes requests, saves the note, and reports the status.

use super::prompt::{
    build_messages, build_messages_from_summaries, clean_title, estimate_tokens, split_lines,
    summary_messages, title_messages, transcript_text,
};
use crate::chat_model::ChatModel;
use crate::db::SharedDb;
use crate::db::meetings::{UNTITLED, get_meeting, record_usage, save_note, set_status, set_title};
use crate::db::templates;
use crate::events::{EventSink, NOTES_DONE, NOTES_FAILED, STATUS};
use crate::llm::ChatMessage;
use crate::types::{MeetingId, MeetingStatus, MorselError, Result};
use serde_json::json;
use std::sync::Arc;
pub use tokio_util::sync::CancellationToken;

pub struct NotesContext {
    pub db: SharedDb,
    pub sink: Arc<dyn EventSink>,
    pub notes: Arc<dyn ChatModel>,
    pub quick: Arc<dyn ChatModel>,
}

impl NotesContext {
    fn set(&self, id: MeetingId, status: MeetingStatus) {
        if let Err(err) = set_status(&self.db.lock().expect("db lock"), id, status) {
            eprintln!("cannot set the meeting status: {err}");
        }
        self.sink.emit(
            STATUS,
            json!({ "meetingId": id, "status": status, "warning": null }),
        );
    }

    async fn ask(
        &self,
        model: &dyn ChatModel,
        messages: Vec<ChatMessage>,
        cancel: &CancellationToken,
    ) -> Result<String> {
        let result = model.chat(&messages, cancel).await?;
        record_usage(
            &self.db.lock().expect("db lock"),
            model.name(),
            result.prompt_tokens,
            result.completion_tokens,
            result.cost,
        )?;
        Ok(result.text.trim().to_owned())
    }
}

/// Writes the note of a meeting. The rough notes and the transcript stay when it fails or is cancelled.
pub async fn write_notes(
    ctx: &NotesContext,
    id: MeetingId,
    template_id: i64,
    cancel: CancellationToken,
) -> Result<()> {
    ctx.set(id, MeetingStatus::WritingNotes);
    let result = run(ctx, id, template_id, &cancel).await;
    match &result {
        Ok(()) => {
            ctx.set(id, MeetingStatus::Done);
            ctx.sink.emit(NOTES_DONE, json!({ "meetingId": id }));
        }
        Err(MorselError::Cancelled) => ctx.set(id, MeetingStatus::NotesCancelled),
        Err(err) => {
            ctx.set(id, MeetingStatus::NotesFailed);
            ctx.sink.emit(
                NOTES_FAILED,
                json!({ "meetingId": id, "message": err.to_string() }),
            );
        }
    }
    result
}

async fn run(
    ctx: &NotesContext,
    id: MeetingId,
    template_id: i64,
    cancel: &CancellationToken,
) -> Result<()> {
    let (detail, template) = {
        let conn = ctx.db.lock().expect("db lock");
        (get_meeting(&conn, id)?, templates::get(&conn, template_id)?)
    };
    let budget = ctx.notes.budget();
    let messages = if estimate_tokens(&transcript_text(&detail.lines)) > budget.long_transcript {
        let parts = split_lines(&detail.lines, budget.part);
        let mut summaries = Vec::with_capacity(parts.len());
        for (i, part) in parts.iter().enumerate() {
            summaries.push(
                ctx.ask(
                    ctx.notes.as_ref(),
                    summary_messages(&transcript_text(part), i + 1, parts.len()),
                    cancel,
                )
                .await?,
            );
        }
        build_messages_from_summaries(&template, &detail.user_notes, &summaries)
    } else {
        build_messages(&template, &detail.user_notes, &detail.lines)
    };
    let note = ctx.ask(ctx.notes.as_ref(), messages, cancel).await?;
    save_note(
        &ctx.db.lock().expect("db lock"),
        id,
        &note,
        ctx.notes.name(),
    )?;
    if detail.summary.title == UNTITLED {
        // A missing title is not an error: the note is already saved.
        if let Ok(raw) = ctx
            .ask(ctx.quick.as_ref(), title_messages(&note), cancel)
            .await
        {
            let title = clean_title(&raw);
            if !title.is_empty() {
                set_title(&ctx.db.lock().expect("db lock"), id, &title)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::meetings::{add_line, create_meeting, get_meeting, save_user_notes};
    use crate::events::VecSink;
    use crate::types::{Speaker, TranscriptLine};
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn reply(text: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "role": "assistant", "content": text } }],
            "usage": { "prompt_tokens": 100, "completion_tokens": 20, "cost": 0.001 }
        }))
    }

    async fn mount_ok(server: &MockServer) {
        Mock::given(method("POST"))
            .and(body_string_contains("Summarize part"))
            .respond_with(reply("part summary"))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("Write a title"))
            .respond_with(reply("Q1 budget review"))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .respond_with(reply("## Summary\nbudget ok"))
            .mount(server)
            .await;
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        ctx: NotesContext,
        sink: Arc<VecSink>,
        id: MeetingId,
    }

    fn fixture(server: &MockServer, title: &str, lines: usize, line_chars: usize) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&dir.path().join("m.db")).unwrap();
        let id = create_meeting(&conn, title, 1).unwrap();
        save_user_notes(&conn, id, "budget ok").unwrap();
        for i in 0..lines {
            let text = "y".repeat(line_chars);
            add_line(
                &conn,
                &TranscriptLine {
                    meeting_id: id,
                    speaker: Speaker::Others,
                    start_ms: i as u64 * 1000,
                    end_ms: i as u64 * 1000 + 900,
                    text,
                },
            )
            .unwrap();
        }
        let sink = Arc::new(VecSink::default());
        let cfg = |model: &str| -> Arc<dyn crate::chat_model::ChatModel> {
            Arc::new(crate::chat_model::CloudChat::new(
                reqwest::Client::new(),
                crate::llm::LlmConfig {
                    base_url: format!("{}/v1", server.uri()),
                    model: model.into(),
                    api_key: "k".into(),
                },
            ))
        };
        let ctx = NotesContext {
            db: Arc::new(std::sync::Mutex::new(conn)),
            sink: sink.clone(),
            notes: cfg("notes-model"),
            quick: cfg("quick-model"),
        };
        Fixture {
            _dir: dir,
            ctx,
            sink,
            id,
        }
    }

    fn status(f: &Fixture) -> MeetingStatus {
        get_meeting(&f.ctx.db.lock().unwrap(), f.id)
            .unwrap()
            .summary
            .status
    }

    #[tokio::test]
    async fn local_budget_splits_a_7000_token_transcript() {
        use crate::chat_model::{Budget, FakeChat};
        let server = MockServer::start().await;
        // 70 lines of 400 characters: about 7,000 tokens, over the local limit of 6,000.
        let mut f = fixture(&server, "Weekly sync", 70, 400);
        let fake = Arc::new(FakeChat::new(Budget::LOCAL, "notes"));
        f.ctx.notes = fake.clone();
        f.ctx.quick = fake.clone();
        write_notes(&f.ctx, f.id, 1, CancellationToken::new())
            .await
            .unwrap();
        let calls = fake.calls();
        // Three part summaries of at most 3,000 tokens, then the final notes.
        assert_eq!(calls.len(), 4);
        for call in &calls {
            let chars: usize = call.iter().map(|m| m.content.len()).sum();
            assert!(chars / 4 < 3_000 + 1_000, "a call has {chars} characters");
        }
        let model: String = f
            .ctx
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT model FROM notes WHERE meeting_id = ?1",
                [f.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(model, "fake");
    }

    #[tokio::test]
    async fn writes_and_saves_the_note() {
        let server = MockServer::start().await;
        mount_ok(&server).await;
        let f = fixture(&server, "Weekly sync", 3, 20);
        write_notes(&f.ctx, f.id, 1, CancellationToken::new())
            .await
            .unwrap();
        let detail = get_meeting(&f.ctx.db.lock().unwrap(), f.id).unwrap();
        assert_eq!(detail.note.as_deref(), Some("## Summary\nbudget ok"));
        assert_eq!(detail.summary.status, MeetingStatus::Done);
        assert_eq!(detail.summary.title, "Weekly sync");
        assert_eq!(f.sink.named(NOTES_DONE)[0]["meetingId"], f.id);
    }

    #[tokio::test]
    async fn long_transcript_is_split_into_parts() {
        let server = MockServer::start().await;
        mount_ok(&server).await;
        // 500 lines of 1,000 characters: about 125,000 tokens, so 2 parts of at most 80,000 tokens.
        let f = fixture(&server, "Long", 500, 1000);
        write_notes(&f.ctx, f.id, 1, CancellationToken::new())
            .await
            .unwrap();
        let bodies: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        assert_eq!(
            bodies
                .iter()
                .filter(|b| b.contains("Summarize part"))
                .count(),
            2
        );
        assert_eq!(bodies.len(), 3);
        assert!(bodies[2].contains("part summary"));
    }

    #[tokio::test]
    async fn cancel_keeps_user_notes_and_transcript() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(reply("late").set_delay(std::time::Duration::from_secs(5)))
            .mount(&server)
            .await;
        let f = fixture(&server, "Weekly sync", 2, 20);
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            trigger.cancel();
        });
        let err = write_notes(&f.ctx, f.id, 1, cancel).await.unwrap_err();
        assert!(matches!(err, MorselError::Cancelled));
        let detail = get_meeting(&f.ctx.db.lock().unwrap(), f.id).unwrap();
        assert_eq!(detail.summary.status, MeetingStatus::NotesCancelled);
        assert_eq!(detail.user_notes, "budget ok");
        assert_eq!(detail.lines.len(), 2);
        assert!(detail.note.is_none());
    }

    #[tokio::test]
    async fn failed_request_keeps_user_notes_and_transcript() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        mount_ok(&server).await;
        let f = fixture(&server, "Weekly sync", 2, 20);
        assert!(
            write_notes(&f.ctx, f.id, 1, CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(status(&f), MeetingStatus::NotesFailed);
        assert_eq!(f.sink.named(NOTES_FAILED).len(), 1);
        let detail = get_meeting(&f.ctx.db.lock().unwrap(), f.id).unwrap();
        assert_eq!(
            (detail.user_notes.as_str(), detail.lines.len()),
            ("budget ok", 2)
        );
        write_notes(&f.ctx, f.id, 1, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(status(&f), MeetingStatus::Done);
    }

    #[tokio::test]
    async fn empty_transcript_uses_user_notes_only() {
        let server = MockServer::start().await;
        mount_ok(&server).await;
        let f = fixture(&server, "Weekly sync", 0, 0);
        write_notes(&f.ctx, f.id, 1, CancellationToken::new())
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&server.received_requests().await.unwrap()[0].body)
            .into_owned();
        assert!(body.contains("(no transcript)") && body.contains("budget ok"));
    }

    #[tokio::test]
    async fn usage_row_is_saved() {
        let server = MockServer::start().await;
        mount_ok(&server).await;
        let f = fixture(&server, "Weekly sync", 1, 20);
        write_notes(&f.ctx, f.id, 1, CancellationToken::new())
            .await
            .unwrap();
        let (model, tokens, cost): (String, i64, f64) = f
            .ctx
            .db
            .lock()
            .unwrap()
            .query_row("SELECT model, prompt_tokens, cost FROM usage", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((model.as_str(), tokens, cost), ("notes-model", 100, 0.001));
    }

    #[tokio::test]
    async fn untitled_meeting_gets_a_title_from_quick_model() {
        let server = MockServer::start().await;
        mount_ok(&server).await;
        let f = fixture(&server, crate::commands::UNTITLED, 1, 20);
        write_notes(&f.ctx, f.id, 1, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            get_meeting(&f.ctx.db.lock().unwrap(), f.id)
                .unwrap()
                .summary
                .title,
            "Q1 budget review"
        );
        let bodies: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        assert!(bodies.last().unwrap().contains("quick-model"));
    }
}

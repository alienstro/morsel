//! Library queries: the search index, the notes list, people, delete, usage and saved questions.

use super::meetings::{MeetingSummary, SUMMARY_SQL, summary_from_row, to_summary};
use crate::types::{MeetingId, MorselError, Result};
use rusqlite::types::Value;
use crate::ask::AskAnswer;
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde::{Deserialize, Serialize};

/// The filters of the All notes screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotesFilter {
    pub query: Option<String>,
    pub template_id: Option<i64>,
    pub person: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub limit: u32,
    pub offset: u32,
}

/// One page of the notes list, and the count of all matches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotesPage {
    pub items: Vec<MeetingSummary>,
    pub total: u32,
}

/// The token counts of a time range. `cost` is set only when a provider returned a cost.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotal {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: Option<f64>,
}

/// The FTS5 keywords. A search word that equals one of them is dropped.
const FTS_KEYWORDS: &[&str] = &["AND", "OR", "NOT", "NEAR"];

/// Rebuilds the search row of one meeting from its title, rough notes, note and transcript.
pub fn reindex_meeting(conn: &Connection, id: MeetingId) -> Result<()> {
    conn.execute("DELETE FROM meeting_search WHERE rowid = ?1", [id])?;
    conn.execute(
        "INSERT INTO meeting_search (rowid, title, user_notes, note, transcript) \
         SELECT m.id, m.title, COALESCE(u.body, ''), COALESCE(n.body_md, ''), \
                COALESCE((SELECT group_concat(text, char(10)) FROM \
                  (SELECT text FROM transcript_lines t WHERE t.meeting_id = m.id ORDER BY start_ms, id)), '') \
         FROM meetings m \
         LEFT JOIN user_notes u ON u.meeting_id = m.id \
         LEFT JOIN notes n ON n.meeting_id = m.id \
         WHERE m.id = ?1",
        [id],
    )?;
    Ok(())
}

/// The words of a search text, each one quoted, so that no FTS5 operator gets through.
fn fts_terms(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !FTS_KEYWORDS.contains(w))
        .map(|w| format!("\"{w}\""))
        .collect()
}

/// A match expression that finds any of the words. Ask Morsel uses it, because a question has many words.
pub fn fts_query(text: &str) -> Option<String> {
    let terms = fts_terms(text);
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

/// A match expression that finds all the words, each as a prefix, for search while the user types.
fn fts_search_query(text: &str) -> Option<String> {
    let terms = fts_terms(text);
    (!terms.is_empty()).then(|| {
        terms
            .iter()
            .map(|t| format!("{t}*"))
            .collect::<Vec<_>>()
            .join(" AND ")
    })
}

pub fn search_meetings(conn: &Connection, filter: &NotesFilter) -> Result<NotesPage> {
    let mut joins = String::new();
    let mut wheres: Vec<&str> = Vec::new();
    let mut args: Vec<Value> = Vec::new();
    let matched = filter.query.as_deref().and_then(fts_search_query);
    if let Some(q) = &matched {
        joins.push_str(" JOIN meeting_search s ON s.rowid = m.id");
        wheres.push("meeting_search MATCH ?");
        args.push(Value::Text(q.clone()));
    }
    if let Some(t) = filter.template_id {
        wheres.push("m.template_id = ?");
        args.push(Value::Integer(t));
    }
    if let Some(p) = filter.person.as_deref().filter(|p| !p.is_empty()) {
        wheres.push("EXISTS (SELECT 1 FROM json_each(m.people) WHERE value = ?)");
        args.push(Value::Text(p.to_owned()));
    }
    if let Some(from) = filter.from {
        wheres.push("m.started_at >= ?");
        args.push(Value::Integer(from));
    }
    if let Some(to) = filter.to {
        wheres.push("m.started_at <= ?");
        args.push(Value::Integer(to));
    }
    let where_sql = if wheres.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", wheres.join(" AND "))
    };
    let total: u32 = conn.query_row(
        &format!("SELECT COUNT(*) FROM meetings m{joins}{where_sql}"),
        params_from_iter(args.iter()),
        |r| r.get(0),
    )?;
    let order = if matched.is_some() {
        "bm25(meeting_search), m.started_at DESC"
    } else {
        "m.started_at DESC, m.id DESC"
    };
    let mut page_args = args.clone();
    page_args.push(Value::Integer(i64::from(filter.limit)));
    page_args.push(Value::Integer(i64::from(filter.offset)));
    let mut stmt = conn.prepare(&format!(
        "{SUMMARY_SQL}{joins}{where_sql} ORDER BY {order} LIMIT ? OFFSET ?"
    ))?;
    let raws = stmt
        .query_map(params_from_iter(page_args.iter()), summary_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let items = raws.into_iter().map(to_summary).collect::<Result<_>>()?;
    Ok(NotesPage { items, total })
}

/// Every person name that a meeting lists, sorted, each one time.
pub fn list_people(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT value FROM meetings, json_each(meetings.people) ORDER BY value",
    )?;
    let people = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(people)
}

pub fn count_meetings(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("SELECT COUNT(*) FROM meetings", [], |r| r.get(0))?)
}

/// Deletes a meeting. Its lines and notes go with it (foreign keys cascade), and its search row too.
pub fn delete_meeting(conn: &Connection, id: MeetingId) -> Result<()> {
    let changed = conn.execute("DELETE FROM meetings WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(MorselError::NotFound(format!("meeting {id}")));
    }
    conn.execute("DELETE FROM meeting_search WHERE rowid = ?1", [id])?;
    Ok(())
}

pub fn usage_since(conn: &Connection, since_ms: i64) -> Result<UsageTotal> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0), SUM(cost) \
         FROM usage WHERE at >= ?1",
        [since_ms],
        |r| {
            Ok(UsageTotal {
                prompt_tokens: r.get::<_, i64>(0)? as u64,
                completion_tokens: r.get::<_, i64>(1)? as u64,
                cost: r.get(2)?,
            })
        },
    )?)
}

type AskRow = (i64, String, String, String);

fn ask_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AskRow> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn to_ask(raw: AskRow) -> Result<AskAnswer> {
    let sources = serde_json::from_str(&raw.3)
        .map_err(|e| MorselError::Db(format!("ask {} sources: {e}", raw.0)))?;
    Ok(AskAnswer {
        id: Some(raw.0),
        question: raw.1,
        answer: raw.2,
        sources,
    })
}

/// Saves an answer across meetings. Returns its id.
pub fn save_ask(conn: &Connection, answer: &AskAnswer) -> Result<i64> {
    let sources = serde_json::to_string(&answer.sources).expect("sources serialize");
    conn.execute(
        "INSERT INTO asks (question, answer, sources_json, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![answer.question, answer.answer, sources, super::now_ms()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// The newest saved questions first.
pub fn list_asks(conn: &Connection, limit: u32) -> Result<Vec<AskAnswer>> {
    let mut stmt = conn.prepare(
        "SELECT id, question, answer, sources_json FROM asks ORDER BY created_at DESC, id DESC LIMIT ?1",
    )?;
    let raws = stmt
        .query_map([limit], ask_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raws.into_iter().map(to_ask).collect()
}

pub fn get_ask(conn: &Connection, id: i64) -> Result<AskAnswer> {
    let raw = conn
        .query_row(
            "SELECT id, question, answer, sources_json FROM asks WHERE id = ?1",
            [id],
            ask_from_row,
        )
        .optional()?
        .ok_or_else(|| MorselError::NotFound(format!("question {id}")))?;
    to_ask(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::meetings::*;
    use crate::db::{migrations, open};
    use crate::types::{MeetingStatus, Speaker, TranscriptLine};
    use rusqlite::params;

    fn temp_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("morsel.db")).unwrap();
        (dir, conn)
    }

    fn line(meeting_id: i64, start_ms: u64, text: &str) -> TranscriptLine {
        TranscriptLine {
            meeting_id,
            speaker: Speaker::Others,
            start_ms,
            end_ms: start_ms + 1000,
            text: text.into(),
        }
    }

    /// A finished meeting with a note, a template, people and a start time.
    fn meeting(conn: &Connection, title: &str, template: i64, started_at: i64, note: &str) -> i64 {
        let id = create_meeting(conn, title, template).unwrap();
        conn.execute(
            "UPDATE meetings SET started_at = ?1 WHERE id = ?2",
            params![started_at, id],
        )
        .unwrap();
        save_note(conn, id, note, "m").unwrap();
        set_status(conn, id, MeetingStatus::Done).unwrap();
        id
    }

    fn filter() -> NotesFilter {
        NotesFilter {
            limit: 50,
            ..NotesFilter::default()
        }
    }

    fn ids(page: &NotesPage) -> Vec<i64> {
        page.items.iter().map(|m| m.id).collect()
    }

    #[test]
    fn search_finds_words_in_note_and_transcript() {
        let (_d, conn) = temp_db();
        let a = meeting(&conn, "Pricing", 1, 1_000, "We agreed on the discount.");
        let b = meeting(&conn, "Roadmap", 1, 2_000, "Ship in March.");
        add_line(&conn, &line(b, 0, "The budget is twenty thousand")).unwrap();
        set_status(&conn, b, MeetingStatus::Done).unwrap();

        let by_note = search_meetings(
            &conn,
            &NotesFilter {
                query: Some("discount".into()),
                ..filter()
            },
        )
        .unwrap();
        assert_eq!(ids(&by_note), vec![a]);
        assert_eq!(by_note.total, 1);

        let by_line = search_meetings(
            &conn,
            &NotesFilter {
                query: Some("budget".into()),
                ..filter()
            },
        )
        .unwrap();
        assert_eq!(ids(&by_line), vec![b]);
    }

    #[test]
    fn search_without_query_lists_newest_first() {
        let (_d, conn) = temp_db();
        let old = meeting(&conn, "Old", 1, 1_000, "x");
        let new = meeting(&conn, "New", 1, 5_000, "y");
        let page = search_meetings(&conn, &filter()).unwrap();
        assert_eq!(ids(&page), vec![new, old]);
        assert_eq!(page.total, 2);
        assert_eq!(count_meetings(&conn).unwrap(), 2);

        let second = search_meetings(
            &conn,
            &NotesFilter {
                limit: 1,
                offset: 1,
                ..filter()
            },
        )
        .unwrap();
        assert_eq!(ids(&second), vec![old]);
        assert_eq!(second.total, 2);
    }

    #[test]
    fn filter_by_template_person_and_dates() {
        let (_d, conn) = temp_db();
        let a = meeting(&conn, "A", 2, 1_000, "x");
        let b = meeting(&conn, "B", 3, 2_000, "x");
        let c = meeting(&conn, "C", 3, 3_000, "x");
        conn.execute(
            "UPDATE meetings SET people = ?1 WHERE id = ?2",
            params![r#"["Sam","Priya"]"#, b],
        )
        .unwrap();
        conn.execute(
            "UPDATE meetings SET people = ?1 WHERE id = ?2",
            params![r#"["Leo"]"#, c],
        )
        .unwrap();

        let by_template = search_meetings(
            &conn,
            &NotesFilter {
                template_id: Some(3),
                ..filter()
            },
        )
        .unwrap();
        assert_eq!(ids(&by_template), vec![c, b]);

        let by_person = search_meetings(
            &conn,
            &NotesFilter {
                person: Some("Priya".into()),
                ..filter()
            },
        )
        .unwrap();
        assert_eq!(ids(&by_person), vec![b]);
        assert_eq!(by_person.items[0].people, vec!["Sam", "Priya"]);

        let by_dates = search_meetings(
            &conn,
            &NotesFilter {
                from: Some(1_500),
                to: Some(2_500),
                ..filter()
            },
        )
        .unwrap();
        assert_eq!(ids(&by_dates), vec![b]);
        assert_eq!(list_people(&conn).unwrap(), vec!["Leo", "Priya", "Sam"]);
        let _ = a;
    }

    #[test]
    fn fts_query_escapes_operators() {
        assert_eq!(
            fts_query(r#"foo" OR * -bar:"#).as_deref(),
            Some(r#""foo" OR "bar""#)
        );
        assert_eq!(fts_query("  * - \" "), None);
        let (_d, conn) = temp_db();
        meeting(&conn, "A", 1, 1_000, "foo");
        let page = search_meetings(
            &conn,
            &NotesFilter {
                query: Some(r#"foo" AND (NEAR:"#.into()),
                ..filter()
            },
        )
        .unwrap();
        assert_eq!(page.total, 1);
    }

    #[test]
    fn delete_meeting_removes_lines_notes_and_search_row() {
        let (_d, conn) = temp_db();
        let id = meeting(&conn, "Gone", 1, 1_000, "secret plan");
        add_line(&conn, &line(id, 0, "hello")).unwrap();
        save_user_notes(&conn, id, "rough").unwrap();
        delete_meeting(&conn, id).unwrap();
        assert!(get_meeting(&conn, id).is_err());
        let left: i64 = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM transcript_lines) + (SELECT COUNT(*) FROM notes) \
                 + (SELECT COUNT(*) FROM user_notes) + (SELECT COUNT(*) FROM meeting_search)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn usage_since_sums_tokens_and_cost_is_none_without_costs() {
        let (_d, conn) = temp_db();
        record_usage(&conn, "m", 100, 20, None).unwrap();
        record_usage(&conn, "m", 50, 5, None).unwrap();
        conn.execute("UPDATE usage SET at = 1 WHERE id = 1", []).unwrap();
        let recent = usage_since(&conn, 10).unwrap();
        assert_eq!(
            recent,
            UsageTotal {
                prompt_tokens: 50,
                completion_tokens: 5,
                cost: None
            }
        );
        record_usage(&conn, "m", 1, 1, Some(0.25)).unwrap();
        assert_eq!(usage_since(&conn, 10).unwrap().cost, Some(0.25));
    }

    #[test]
    fn migration_2_backfills_existing_meetings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("morsel.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            rusqlite_migration::Migrations::new(migrations().into_iter().take(1).collect())
                .to_latest(&mut conn)
                .unwrap();
            conn.execute(
                "INSERT INTO meetings (id, title, template_id, status, started_at) \
                 VALUES (7, 'Before', 1, 'done', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO notes (meeting_id, body_md, model, created_at) VALUES (7, 'alpaca farm', 'm', 1)",
                [],
            )
            .unwrap();
        }
        let conn = open(&path).unwrap();
        let page = search_meetings(
            &conn,
            &NotesFilter {
                query: Some("alpaca".into()),
                ..filter()
            },
        )
        .unwrap();
        assert_eq!(ids(&page), vec![7]);
        assert!(page.items[0].people.is_empty());
    }
}

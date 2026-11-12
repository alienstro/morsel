use super::meetings::*;
use super::open;
use super::templates;
use crate::settings::TranscribeTiming;
use crate::types::{MeetingStatus, Speaker, TranscriptLine};
use rusqlite::Connection;

fn temp_db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = open(&dir.path().join("morsel.db")).unwrap();
    (dir, conn)
}

fn line(meeting_id: i64, speaker: Speaker, start_ms: u64, text: &str) -> TranscriptLine {
    TranscriptLine {
        meeting_id,
        speaker,
        start_ms,
        end_ms: start_ms + 1000,
        text: text.into(),
    }
}

#[test]
fn open_runs_migrations_twice_without_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("morsel.db");
    drop(open(&path).unwrap());
    let conn = open(&path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM templates", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 5);
}

#[test]
fn seeds_five_templates() {
    let (_d, conn) = temp_db();
    let names: Vec<String> = templates::list(&conn)
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(
        names,
        ["General", "1:1", "Sales call", "Stand-up", "Product sync"]
    );
    let general = templates::get(&conn, 1).unwrap();
    let sections: Vec<&str> = general.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        sections,
        ["Summary", "Key points", "Decisions", "Action items"]
    );
}

#[test]
fn lines_come_back_in_start_order() {
    let (_d, conn) = temp_db();
    let id = create_meeting(&conn, "Weekly product sync", 5).unwrap();
    add_line(&conn, &line(id, Speaker::Others, 20_000, "second")).unwrap();
    add_line(&conn, &line(id, Speaker::You, 5_000, "first")).unwrap();
    let detail = get_meeting(&conn, id).unwrap();
    let texts: Vec<&str> = detail.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["first", "second"]);
    assert_eq!(detail.lines[0].speaker, Speaker::You);
    assert_eq!(detail.summary.template_name, "Product sync");
    assert_eq!(detail.summary.status, MeetingStatus::Recording);
}

#[test]
fn user_notes_upsert_keeps_one_row() {
    let (_d, conn) = temp_db();
    let id = create_meeting(&conn, "1:1 with Sam", 2).unwrap();
    save_user_notes(&conn, id, "budget ok").unwrap();
    save_user_notes(&conn, id, "budget ok for Q1").unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM user_notes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    assert_eq!(
        get_meeting(&conn, id).unwrap().user_notes,
        "budget ok for Q1"
    );
}

#[test]
fn mark_open_meetings_interrupted_changes_only_recording_and_paused() {
    let (_d, conn) = temp_db();
    let recording = create_meeting(&conn, "a", 1).unwrap();
    let paused = create_meeting(&conn, "b", 1).unwrap();
    let done = create_meeting(&conn, "c", 1).unwrap();
    set_status(&conn, paused, MeetingStatus::Paused).unwrap();
    set_status(&conn, done, MeetingStatus::Done).unwrap();
    assert_eq!(mark_open_meetings_interrupted(&conn).unwrap(), 2);
    assert_eq!(
        get_meeting(&conn, recording).unwrap().summary.status,
        MeetingStatus::Interrupted
    );
    assert_eq!(
        get_meeting(&conn, paused).unwrap().summary.status,
        MeetingStatus::Interrupted
    );
    assert_eq!(
        get_meeting(&conn, done).unwrap().summary.status,
        MeetingStatus::Done
    );
}

#[test]
fn save_note_and_list_meetings_newest_first() {
    let (_d, conn) = temp_db();
    let a = create_meeting(&conn, "first", 1).unwrap();
    let b = create_meeting(&conn, "second", 1).unwrap();
    save_note(&conn, a, "## Summary\nok", "model-x").unwrap();
    assert_eq!(
        get_meeting(&conn, a).unwrap().note.as_deref(),
        Some("## Summary\nok")
    );
    let ids: Vec<i64> = list_meetings(&conn, 10)
        .unwrap()
        .iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, [b, a]);
}

#[test]
fn migration_3_adds_the_timing_columns() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open(&dir.path().join("m.db")).unwrap();
    let id = create_meeting(&conn, "Sync", 1).unwrap();
    assert_eq!(timing_of(&conn, id).unwrap(), TranscribeTiming::Live);
    set_timing(&conn, id, TranscribeTiming::After).unwrap();
    assert_eq!(timing_of(&conn, id).unwrap(), TranscribeTiming::After);
    assert_eq!(get_meeting(&conn, id).unwrap().summary.transcribed_ms, 0);
    set_status(&conn, id, MeetingStatus::Transcribing).unwrap();
    assert_eq!(
        list_by_status(&conn, MeetingStatus::Transcribing).unwrap(),
        [id]
    );
}

mod parts {
    use super::temp_db;
    use crate::db::library::delete_meeting;
    use crate::db::meetings::{create_meeting, set_status};
    use crate::db::parts::*;
    use crate::types::MeetingStatus;

    fn part(text: &str) -> NewPart {
        NewPart {
            start_ms: Some(0),
            end_ms: Some(1000),
            text: text.into(),
        }
    }

    fn part_texts(conn: &rusqlite::Connection, meeting: i64) -> Vec<String> {
        conn.prepare("SELECT text FROM chunks WHERE meeting_id = ?1 ORDER BY part")
            .unwrap()
            .query_map([meeting], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn replace_parts_keeps_only_the_new_parts() {
        let (_d, conn) = temp_db();
        let id = create_meeting(&conn, "Sync", 1).unwrap();
        replace_parts(&conn, id, &[part("old one"), part("old two")]).unwrap();
        replace_parts(&conn, id, &[part("new one")]).unwrap();
        assert_eq!(part_texts(&conn, id), vec!["new one".to_string()]);
        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0);
    }

    #[test]
    fn set_embedding_round_trips_768_floats() {
        let (_d, conn) = temp_db();
        let id = create_meeting(&conn, "Sync", 1).unwrap();
        replace_parts(&conn, id, &[part("budget")]).unwrap();
        let (part_id, meeting, text) = parts_without_embedding(&conn, 10).unwrap().remove(0);
        assert_eq!((meeting, text.as_str()), (id, "budget"));
        let vector: Vec<f32> = (0..768).map(|i| i as f32 / 768.0).collect();
        set_embedding(&conn, part_id, &vector).unwrap();
        let blob: Vec<u8> = conn
            .query_row(
                "SELECT embedding FROM chunks WHERE id = ?1",
                [part_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(decode_embedding(&blob), vector);
        assert!(parts_without_embedding(&conn, 10).unwrap().is_empty());
    }

    #[test]
    fn deleting_a_meeting_deletes_its_parts_and_their_search_rows() {
        let (_d, conn) = temp_db();
        let id = create_meeting(&conn, "Sync", 1).unwrap();
        replace_parts(&conn, id, &[part("budget talk")]).unwrap();
        delete_meeting(&conn, id).unwrap();
        assert!(part_texts(&conn, id).is_empty());
        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'budget'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0);
    }

    #[test]
    fn meetings_without_parts_lists_only_done_meetings() {
        let (_d, conn) = temp_db();
        let done = create_meeting(&conn, "Done", 1).unwrap();
        set_status(&conn, done, MeetingStatus::Done).unwrap();
        let recording = create_meeting(&conn, "Live", 1).unwrap();
        set_status(&conn, recording, MeetingStatus::Recording).unwrap();
        let indexed = create_meeting(&conn, "Indexed", 1).unwrap();
        set_status(&conn, indexed, MeetingStatus::Done).unwrap();
        replace_parts(&conn, indexed, &[part("x")]).unwrap();
        assert_eq!(meetings_without_parts(&conn).unwrap(), vec![done]);
    }
}

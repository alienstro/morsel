//! Hybrid search over meeting parts: FTS5 keywords and embeddings, merged with
//! reciprocal rank fusion.

use crate::ask::AskRange;
use crate::db::library::fts_query;
use crate::db::parts::decode_embedding;
use crate::search::embed::cosine;
use crate::types::{MeetingId, Result};
use rusqlite::{Connection, params};
use std::collections::HashMap;

/// The constant of reciprocal rank fusion.
pub const RRF_K: f64 = 60.0;
/// The most parts that Ask gets.
pub const MAX_PARTS: usize = 15;
/// The length of each list before the fusion.
const LIST_LEN: usize = 50;

/// One found part.
#[derive(Debug, Clone, PartialEq)]
pub struct PartHit {
    pub id: i64,
    pub meeting: MeetingId,
    pub start_ms: Option<u64>,
    pub text: String,
}

/// Merges ranked lists of part ids: each id scores 1 / (k + rank), rank from 1.
pub fn rrf(lists: &[Vec<i64>]) -> Vec<i64> {
    let mut scores: Vec<(i64, f64)> = Vec::new();
    let mut index: HashMap<i64, usize> = HashMap::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            let score = 1.0 / (RRF_K + rank as f64 + 1.0);
            match index.get(id) {
                Some(&i) => scores[i].1 += score,
                None => {
                    index.insert(*id, scores.len());
                    scores.push((*id, score));
                }
            }
        }
    }
    // A stable sort keeps the first-seen order for equal scores.
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    scores.into_iter().map(|(id, _)| id).collect()
}

fn keyword_list(conn: &Connection, question: &str, since: i64) -> Result<Vec<i64>> {
    let Some(query) = fts_query(question) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(
        "SELECT c.id FROM chunks_fts f JOIN chunks c ON c.id = f.rowid \
         JOIN meetings m ON m.id = c.meeting_id \
         WHERE chunks_fts MATCH ?1 AND m.started_at >= ?2 ORDER BY bm25(chunks_fts) LIMIT ?3",
    )?;
    let ids = stmt
        .query_map(params![query, since, LIST_LEN as i64], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(ids)
}

/// A plain scan: a few thousand parts of 768 values take a few milliseconds.
fn vector_list(conn: &Connection, query: &[f32], since: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.embedding FROM chunks c JOIN meetings m ON m.id = c.meeting_id \
         WHERE c.embedding IS NOT NULL AND m.started_at >= ?1",
    )?;
    let mut scored = stmt
        .query_map([since], |r| {
            let blob: Vec<u8> = r.get(1)?;
            Ok((r.get::<_, i64>(0)?, blob))
        })?
        .map(|row| row.map(|(id, blob)| (id, cosine(query, &decode_embedding(&blob)))))
        .collect::<rusqlite::Result<Vec<(i64, f32)>>>()?;
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(scored
        .into_iter()
        .take(LIST_LEN)
        .map(|(id, _)| id)
        .collect())
}

fn hit(conn: &Connection, id: i64) -> Result<PartHit> {
    let hit = conn.query_row(
        "SELECT meeting_id, start_ms, text FROM chunks WHERE id = ?1",
        [id],
        |r| {
            Ok(PartHit {
                id,
                meeting: r.get(0)?,
                start_ms: r.get::<_, Option<i64>>(1)?.map(|v| v as u64),
                text: r.get(2)?,
            })
        },
    )?;
    Ok(hit)
}

/// The best parts for a question, best first. `query` is the question's embedding,
/// or `None` when the search model is not there.
pub fn search_parts(
    conn: &Connection,
    question: &str,
    query: Option<&[f32]>,
    range: AskRange,
    now_ms: i64,
) -> Result<Vec<PartHit>> {
    let since = range.since(now_ms);
    let mut lists = vec![keyword_list(conn, question, since)?];
    if let Some(query) = query {
        lists.push(vector_list(conn, query, since)?);
    }
    rrf(&lists)
        .into_iter()
        .take(MAX_PARTS)
        .map(|id| hit(conn, id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::meetings::create_meeting;
    use crate::db::parts::{NewPart, parts_without_embedding, replace_parts, set_embedding};

    const DAY_MS: i64 = 86_400_000;
    const NOW: i64 = 100 * DAY_MS;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&dir.path().join("m.db")).unwrap();
        (dir, conn)
    }

    /// A meeting that started `days_ago`, with one part for each (text, vector).
    fn meeting(conn: &Connection, days_ago: i64, parts: &[(&str, [f32; 3])]) -> Vec<i64> {
        let id = create_meeting(conn, "Sync", 1).unwrap();
        conn.execute(
            "UPDATE meetings SET started_at = ?1 WHERE id = ?2",
            [NOW - days_ago * DAY_MS, id],
        )
        .unwrap();
        let new: Vec<NewPart> = parts
            .iter()
            .map(|(t, _)| NewPart {
                start_ms: Some(0),
                end_ms: Some(1),
                text: (*t).into(),
            })
            .collect();
        replace_parts(conn, id, &new).unwrap();
        let rows = parts_without_embedding(conn, 100).unwrap();
        for ((part, _, _), (_, v)) in rows.iter().zip(parts) {
            set_embedding(conn, *part, v).unwrap();
        }
        rows.iter().map(|r| r.0).collect()
    }

    fn ids(hits: &[PartHit]) -> Vec<i64> {
        hits.iter().map(|h| h.id).collect()
    }

    #[test]
    fn rrf_puts_a_part_found_by_both_lists_first() {
        assert_eq!(rrf(&[vec![1, 2, 3], vec![3, 4]]), vec![3, 1, 2, 4]);
    }

    #[test]
    fn keyword_only_search_works_without_a_vector() {
        let (_d, conn) = db();
        let p = meeting(
            &conn,
            1,
            &[
                ("the budget is 280,000", [1.0, 0.0, 0.0]),
                ("FAQ page", [0.0, 1.0, 0.0]),
            ],
        );
        let hits = search_parts(&conn, "budget", None, AskRange::All, NOW).unwrap();
        assert_eq!(ids(&hits), vec![p[0]]);
    }

    #[test]
    fn vector_search_finds_a_part_with_no_shared_word() {
        let (_d, conn) = db();
        let p = meeting(
            &conn,
            1,
            &[
                ("ilipat sa Marso 29", [1.0, 0.0, 0.0]),
                ("FAQ page", [0.0, 1.0, 0.0]),
            ],
        );
        let hits = search_parts(
            &conn,
            "launch date",
            Some(&[1.0, 0.0, 0.0]),
            AskRange::All,
            NOW,
        )
        .unwrap();
        assert_eq!(hits[0].id, p[0]);
        assert_eq!(hits[0].text, "ilipat sa Marso 29");
    }

    #[test]
    fn a_question_of_only_fts_keywords_still_uses_the_vector() {
        let (_d, conn) = db();
        let p = meeting(&conn, 1, &[("launch moves", [1.0, 0.0, 0.0])]);
        let hits = search_parts(
            &conn,
            "AND OR NOT",
            Some(&[1.0, 0.0, 0.0]),
            AskRange::All,
            NOW,
        )
        .unwrap();
        assert_eq!(ids(&hits), vec![p[0]]);
    }

    #[test]
    fn the_week_range_skips_old_parts() {
        let (_d, conn) = db();
        let new = meeting(&conn, 2, &[("budget new", [1.0, 0.0, 0.0])]);
        meeting(&conn, 20, &[("budget old", [1.0, 0.0, 0.0])]);
        let hits =
            search_parts(&conn, "budget", Some(&[1.0, 0.0, 0.0]), AskRange::Week, NOW).unwrap();
        assert_eq!(ids(&hits), new);
    }

    #[test]
    fn search_returns_at_most_15_parts() {
        let (_d, conn) = db();
        let parts: Vec<(String, [f32; 3])> = (0..30)
            .map(|i| (format!("budget item {i}"), [1.0, 0.0, 0.0]))
            .collect();
        let refs: Vec<(&str, [f32; 3])> = parts.iter().map(|(t, v)| (t.as_str(), *v)).collect();
        meeting(&conn, 1, &refs);
        let hits =
            search_parts(&conn, "budget", Some(&[1.0, 0.0, 0.0]), AskRange::All, NOW).unwrap();
        assert_eq!(hits.len(), MAX_PARTS);
    }
}

//! Note templates: the five built-in templates and the user's own templates.

use crate::types::{MorselError, Result};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde::{Deserialize, Serialize};

/// The template that a meeting gets when its own template is deleted.
pub const GENERAL_ID: i64 = 1;

/// One `##` section of a note. The hint tells the model what the section holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "SectionRepr")]
pub struct Section {
    pub name: String,
    pub hint: String,
}

/// The core loop saved sections as plain strings. Both forms load.
#[derive(Deserialize)]
#[serde(untagged)]
enum SectionRepr {
    Name(String),
    Full {
        name: String,
        #[serde(default)]
        hint: String,
    },
}

impl From<SectionRepr> for Section {
    fn from(repr: SectionRepr) -> Self {
        match repr {
            SectionRepr::Name(name) => Section {
                name,
                hint: String::new(),
            },
            SectionRepr::Full { name, hint } => Section { name, hint },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub sections: Vec<Section>,
    pub instructions: String,
    pub built_in: bool,
    /// The count of meetings that use the template.
    pub uses: u32,
}

/// What the user can change in a template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInput {
    pub name: String,
    pub description: String,
    pub sections: Vec<Section>,
    pub instructions: String,
}

struct BuiltIn {
    id: i64,
    name: &'static str,
    description: &'static str,
    sections: &'static [(&'static str, &'static str)],
    instructions: &'static str,
}

const BUILT_INS: &[BuiltIn] = &[
    BuiltIn {
        id: 1,
        name: "General",
        description: "Any meeting",
        sections: &[
            ("Summary", "The meeting in two or three sentences"),
            ("Key points", "The main facts and ideas"),
            ("Decisions", "What the group agreed"),
            ("Action items", "Tasks, each with an owner"),
        ],
        instructions: "Write short, clear notes for any meeting.",
    },
    BuiltIn {
        id: 2,
        name: "1:1",
        description: "A meeting between a manager and one person",
        sections: &[
            ("Updates", "Progress since the last 1:1"),
            ("Blockers", "What slows the person down"),
            ("Feedback", "Feedback in both directions"),
            ("Action items", "Tasks, each with an owner"),
        ],
        instructions: "Focus on the progress, the problems and the support that each person needs.",
    },
    BuiltIn {
        id: 3,
        name: "Sales call",
        description: "A call with a customer or a prospect",
        sections: &[
            ("Pain points", "The problems of the customer"),
            ("Budget", "Money, price and timing"),
            ("Decision process", "Who decides, and how"),
            ("Next steps", "What happens next, with dates"),
        ],
        instructions: "Focus on the needs of the customer, the money, who decides, and the next step.",
    },
    BuiltIn {
        id: 4,
        name: "Stand-up",
        description: "A short daily team check-in",
        sections: &[
            ("Yesterday", "What each person finished"),
            ("Today", "What each person works on next"),
            ("Blockers", "What stops the work"),
        ],
        instructions: "Group the points by person when the transcript shows who said them.",
    },
    BuiltIn {
        id: 5,
        name: "Product sync",
        description: "Weekly product meetings with the core team",
        sections: &[
            ("Updates", "Progress since the last sync"),
            ("Decisions", "What the team agreed"),
            ("Risks", "What can delay the plan"),
            ("Action items", "Tasks, each with an owner"),
        ],
        instructions: "Focus on the product decisions, the dates, and the risks.",
    },
];

const SELECT_SQL: &str = "SELECT t.id, t.name, t.description, t.sections_json, t.instructions, t.built_in, \
     (SELECT COUNT(*) FROM meetings m WHERE m.template_id = t.id) FROM templates t";

type TemplateRow = (i64, String, String, String, String, bool, u32);

fn from_row(row: &Row<'_>) -> rusqlite::Result<TemplateRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

fn to_template(raw: TemplateRow) -> Result<Template> {
    let sections = serde_json::from_str(&raw.3)
        .map_err(|e| MorselError::Db(format!("template {} sections: {e}", raw.0)))?;
    Ok(Template {
        id: raw.0,
        name: raw.1,
        description: raw.2,
        sections,
        instructions: raw.4,
        built_in: raw.5,
        uses: raw.6,
    })
}

fn sections_json(sections: &[Section]) -> String {
    serde_json::to_string(sections).expect("sections serialize")
}

/// Trims every field and checks that the template has a name and at least one named section.
fn clean(input: &TemplateInput) -> Result<TemplateInput> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(MorselError::Invalid("give the template a name".into()));
    }
    let sections: Vec<Section> = input
        .sections
        .iter()
        .filter(|s| !s.name.trim().is_empty())
        .map(|s| Section {
            name: s.name.trim().to_owned(),
            hint: s.hint.trim().to_owned(),
        })
        .collect();
    if sections.is_empty() {
        return Err(MorselError::Invalid("add at least one section".into()));
    }
    Ok(TemplateInput {
        name: name.to_owned(),
        description: input.description.trim().to_owned(),
        sections,
        instructions: input.instructions.trim().to_owned(),
    })
}

pub fn list(conn: &Connection) -> Result<Vec<Template>> {
    let mut stmt = conn.prepare(&format!("{SELECT_SQL} ORDER BY t.id"))?;
    let raws = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raws.into_iter().map(to_template).collect()
}

pub fn get(conn: &Connection, id: i64) -> Result<Template> {
    let raw = conn
        .query_row(&format!("{SELECT_SQL} WHERE t.id = ?1"), [id], from_row)
        .optional()?
        .ok_or_else(|| MorselError::NotFound(format!("template {id}")))?;
    to_template(raw)
}

pub fn create(conn: &Connection, input: &TemplateInput) -> Result<i64> {
    let input = clean(input)?;
    conn.execute(
        "INSERT INTO templates (name, description, sections_json, instructions, built_in) \
         VALUES (?1, ?2, ?3, ?4, 0)",
        params![
            input.name,
            input.description,
            sections_json(&input.sections),
            input.instructions
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, input: &TemplateInput) -> Result<()> {
    let input = clean(input)?;
    let changed = conn.execute(
        "UPDATE templates SET name = ?1, description = ?2, sections_json = ?3, instructions = ?4 \
         WHERE id = ?5",
        params![
            input.name,
            input.description,
            sections_json(&input.sections),
            input.instructions,
            id
        ],
    )?;
    if changed == 0 {
        return Err(MorselError::NotFound(format!("template {id}")));
    }
    Ok(())
}

/// Makes a custom copy named "<name> copy".
pub fn duplicate(conn: &Connection, id: i64) -> Result<i64> {
    let t = get(conn, id)?;
    create(
        conn,
        &TemplateInput {
            name: format!("{} copy", t.name),
            description: t.description,
            sections: t.sections,
            instructions: t.instructions,
        },
    )
}

/// Deletes a custom template. Its meetings move to General, so their notes stay.
pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    if get(conn, id)?.built_in {
        return Err(MorselError::Invalid(
            "a built-in template cannot be deleted".into(),
        ));
    }
    conn.execute(
        "UPDATE meetings SET template_id = ?1 WHERE template_id = ?2",
        params![GENERAL_ID, id],
    )?;
    conn.execute("DELETE FROM templates WHERE id = ?1", [id])?;
    Ok(())
}

fn write_built_in(conn: &Connection, b: &BuiltIn) -> rusqlite::Result<usize> {
    let sections: Vec<Section> = b
        .sections
        .iter()
        .map(|(name, hint)| Section {
            name: (*name).to_owned(),
            hint: (*hint).to_owned(),
        })
        .collect();
    conn.execute(
        "INSERT INTO templates (id, name, description, sections_json, instructions, built_in) \
         VALUES (?1, ?2, ?3, ?4, ?5, 1) \
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, description = excluded.description, \
         sections_json = excluded.sections_json, instructions = excluded.instructions, built_in = 1",
        params![
            b.id,
            b.name,
            b.description,
            sections_json(&sections),
            b.instructions
        ],
    )
}

/// Puts the built-in values back into a built-in template.
pub fn reset(conn: &Connection, id: i64) -> Result<()> {
    let built_in = BUILT_INS
        .iter()
        .find(|b| b.id == id)
        .ok_or_else(|| MorselError::Invalid("only a built-in template can be reset".into()))?;
    write_built_in(conn, built_in)?;
    Ok(())
}

/// Writes all five built-in templates. "Delete all data" also uses it.
pub fn write_all_built_ins(conn: &Connection) -> Result<()> {
    for b in BUILT_INS {
        write_built_in(conn, b)?;
    }
    Ok(())
}

/// Migration hook: writes the built-in values into the five built-in rows.
pub fn write_built_ins(tx: &Transaction) -> rusqlite_migration::HookResult {
    for b in BUILT_INS {
        write_built_in(tx, b)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::meetings::{create_meeting, get_meeting, save_note};
    use crate::db::open;

    fn temp_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("morsel.db")).unwrap();
        (dir, conn)
    }

    fn section(name: &str, hint: &str) -> Section {
        Section {
            name: name.into(),
            hint: hint.into(),
        }
    }

    fn input(name: &str) -> TemplateInput {
        TemplateInput {
            name: name.into(),
            description: "Weekly design reviews".into(),
            sections: vec![section("Feedback", "What people said"), section("Next", "")],
            instructions: "Be short.".into(),
        }
    }

    #[test]
    fn create_update_and_list_template() {
        let (_d, conn) = temp_db();
        let id = create(&conn, &input("  Design review ")).unwrap();
        let made = get(&conn, id).unwrap();
        assert_eq!(made.name, "Design review");
        assert_eq!(made.sections[0], section("Feedback", "What people said"));
        assert!(!made.built_in);

        let mut changed = input("Design crit");
        changed.sections.push(section("Risks", "What can go wrong"));
        update(&conn, id, &changed).unwrap();
        let after = get(&conn, id).unwrap();
        assert_eq!(after.name, "Design crit");
        assert_eq!(after.sections.len(), 3);

        let all = list(&conn).unwrap();
        assert_eq!(all.len(), 6);
        assert_eq!(all[5].id, id);

        let empty = TemplateInput {
            name: " ".into(),
            ..input("x")
        };
        assert!(matches!(create(&conn, &empty), Err(MorselError::Invalid(_))));
        let no_sections = TemplateInput {
            sections: vec![section("  ", "")],
            ..input("x")
        };
        assert!(matches!(
            create(&conn, &no_sections),
            Err(MorselError::Invalid(_))
        ));
    }

    #[test]
    fn duplicate_adds_copy_suffix() {
        let (_d, conn) = temp_db();
        let copy = duplicate(&conn, 5).unwrap();
        let t = get(&conn, copy).unwrap();
        assert_eq!(t.name, "Product sync copy");
        assert!(!t.built_in);
        assert_eq!(t.sections, get(&conn, 5).unwrap().sections);
    }

    #[test]
    fn delete_built_in_is_refused() {
        let (_d, conn) = temp_db();
        assert!(matches!(delete(&conn, 2), Err(MorselError::Invalid(_))));
        assert_eq!(list(&conn).unwrap().len(), 5);
    }

    #[test]
    fn delete_used_template_moves_meetings_to_general() {
        let (_d, conn) = temp_db();
        let id = create(&conn, &input("Mine")).unwrap();
        let meeting = create_meeting(&conn, "Uses mine", id).unwrap();
        save_note(&conn, meeting, "kept", "m").unwrap();
        delete(&conn, id).unwrap();
        let detail = get_meeting(&conn, meeting).unwrap();
        assert_eq!(detail.summary.template_id, GENERAL_ID);
        assert_eq!(detail.note.as_deref(), Some("kept"));
        assert!(get(&conn, id).is_err());
    }

    #[test]
    fn reset_restores_built_in_values() {
        let (_d, conn) = temp_db();
        let before = get(&conn, 5).unwrap();
        assert_eq!(before.name, "Product sync");
        assert_eq!(before.sections[2], section("Risks", "What can delay the plan"));
        update(&conn, 5, &input("Changed")).unwrap();
        reset(&conn, 5).unwrap();
        assert_eq!(get(&conn, 5).unwrap(), before);
        let mine = create(&conn, &input("Mine")).unwrap();
        assert!(matches!(reset(&conn, mine), Err(MorselError::Invalid(_))));
    }

    #[test]
    fn uses_counts_meetings() {
        let (_d, conn) = temp_db();
        create_meeting(&conn, "a", 3).unwrap();
        create_meeting(&conn, "b", 3).unwrap();
        assert_eq!(get(&conn, 3).unwrap().uses, 2);
        assert_eq!(get(&conn, 1).unwrap().uses, 0);
    }

    #[test]
    fn old_string_sections_still_load() {
        let (_d, conn) = temp_db();
        conn.execute(
            "UPDATE templates SET sections_json = '[\"Summary\",\"Next steps\"]' WHERE id = 1",
            [],
        )
        .unwrap();
        let t = get(&conn, 1).unwrap();
        assert_eq!(t.sections, vec![section("Summary", ""), section("Next steps", "")]);
    }
}

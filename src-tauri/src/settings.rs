//! User settings, saved as one JSON row in the `settings` table.

use crate::types::{MorselError, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const ROW_KEY: &str = "app";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscribeMode {
    Local,
    Cloud,
}

/// Settings never hold an API key. Keys live in the Keychain (see `secrets`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub notes_base_url: String,
    pub notes_model: String,
    pub quick_model: String,
    pub transcribe_mode: TranscribeMode,
    pub transcribe_base_url: String,
    pub transcribe_model: String,
    pub language: String,
    /// False until the user finishes or skips the first-run screens.
    pub onboarded: bool,
    /// Show a reminder one minute before a calendar meeting.
    pub remind_before_meeting: bool,
    /// The global shortcut that starts a note, in the Tauri accelerator format.
    pub global_shortcut: String,
}

pub const DEFAULT_SHORTCUT: &str = "CommandOrControl+Shift+M";

impl Default for Settings {
    fn default() -> Self {
        Self {
            notes_base_url: "https://openrouter.ai/api/v1".into(),
            notes_model: String::new(),
            quick_model: String::new(),
            transcribe_mode: TranscribeMode::Local,
            transcribe_base_url: "https://api.groq.com/openai/v1".into(),
            transcribe_model: "whisper-large-v3-turbo".into(),
            language: "auto".into(),
            onboarded: false,
            remind_before_meeting: true,
            global_shortcut: DEFAULT_SHORTCUT.into(),
        }
    }
}

pub fn load_settings(conn: &Connection) -> Result<Settings> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [ROW_KEY],
            |r| r.get(0),
        )
        .optional()?;
    match raw {
        None => Ok(Settings::default()),
        Some(json) => {
            serde_json::from_str(&json).map_err(|e| MorselError::Db(format!("settings: {e}")))
        }
    }
}

pub fn save_settings(conn: &Connection, settings: &Settings) -> Result<()> {
    let json = serde_json::to_string(settings).map_err(|e| MorselError::Invalid(e.to_string()))?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [ROW_KEY, json.as_str()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemoryStore, SecretStore};

    fn temp_db() -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&dir.path().join("morsel.db")).unwrap();
        (dir, conn)
    }

    #[test]
    fn default_settings_use_local_transcription() {
        let s = Settings::default();
        assert_eq!(s.transcribe_mode, TranscribeMode::Local);
        assert_eq!(s.transcribe_base_url, "https://api.groq.com/openai/v1");
        assert_eq!(s.transcribe_model, "whisper-large-v3-turbo");
        assert_eq!(s.notes_base_url, "https://openrouter.ai/api/v1");
        assert_eq!(s.language, "auto");
    }

    #[test]
    fn default_settings_need_onboarding() {
        let s = Settings::default();
        assert!(!s.onboarded);
        assert!(s.remind_before_meeting);
        assert_eq!(s.global_shortcut, "CommandOrControl+Shift+M");
    }

    #[test]
    fn old_settings_json_loads_with_new_defaults() {
        let (_d, conn) = temp_db();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('app', ?1)",
            [r#"{"notesModel":"old/model","transcribeMode":"cloud"}"#],
        )
        .unwrap();
        let s = load_settings(&conn).unwrap();
        assert_eq!(s.notes_model, "old/model");
        assert!(!s.onboarded);
        assert!(s.remind_before_meeting);
        assert_eq!(s.global_shortcut, "CommandOrControl+Shift+M");
    }

    #[test]
    fn settings_round_trip() {
        let (_d, conn) = temp_db();
        assert_eq!(load_settings(&conn).unwrap(), Settings::default());
        let s = Settings {
            notes_model: "some/model".into(),
            transcribe_mode: TranscribeMode::Cloud,
            ..Settings::default()
        };
        save_settings(&conn, &s).unwrap();
        assert_eq!(load_settings(&conn).unwrap(), s);
    }

    #[test]
    fn settings_table_never_contains_a_key() {
        let (_d, conn) = temp_db();
        let store = MemoryStore::default();
        store.set("notes_api_key", "sk-secret-123").unwrap();
        save_settings(&conn, &Settings::default()).unwrap();
        let mut stmt = conn.prepare("SELECT value FROM settings").unwrap();
        let values: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|v| v.unwrap())
            .collect();
        assert!(!values.is_empty());
        assert!(values.iter().all(|v| !v.contains("sk-secret-123")));
    }
}

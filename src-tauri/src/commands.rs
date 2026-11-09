//! The commands that the frontend calls. Each command is a thin call into an `AppState` method.

use crate::ask::{self, AskAnswer, AskRange};
use crate::audio::{mic::MicSource, source::AudioSource, system_tap::SystemTapSource};
use crate::db::library::{self, NotesFilter, NotesPage, UsageTotal};
use crate::db::meetings::{self, MeetingDetail, MeetingSummary};
use crate::db::templates::{self, Template, TemplateInput};
use crate::db::{SharedDb, now_ms};
use crate::events::{EventSink, MODEL_DOWNLOAD};
use crate::llm::{self, LlmConfig};
use crate::notes::writer::{CancellationToken, NotesContext};
use crate::secrets::{ACCOUNTS, NOTES_KEY, SecretStore, TRANSCRIBE_KEY, is_known_account};
use crate::session::MeetingSession;
use crate::settings::{self, LocalModel, Settings, TranscribeMode};
use crate::transcribe::Transcriber;
use crate::transcribe::cloud::CloudTranscriber;
use crate::transcribe::model_download::{ensure_model, model_ready};
use crate::transcribe::parakeet::ParakeetTranscriber;
use crate::transcribe::whisper::WhisperTranscriber;
use crate::types::{MeetingId, MorselError, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::State;

pub use crate::db::meetings::UNTITLED;

/// Makes the audio sources and the transcriber. Tests use fakes.
pub trait Engines: Send + Sync {
    fn mic(&self) -> Box<dyn AudioSource>;
    fn system(&self) -> Box<dyn AudioSource>;
    fn transcriber(&self, settings: &Settings) -> Result<Arc<dyn Transcriber>>;
}

/// The real microphone, the system tap, and Parakeet or the cloud.
/// The loaded on-device transcriber, with the model and the language that it was loaded for.
type LoadedLocal = (LocalModel, String, Arc<dyn Transcriber>);

pub struct RealEngines {
    models_root: PathBuf,
    http: reqwest::Client,
    secrets: Arc<dyn SecretStore>,
    local: Mutex<Option<LoadedLocal>>,
}

impl RealEngines {
    pub fn new(models_root: PathBuf, http: reqwest::Client, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            models_root,
            http,
            secrets,
            local: Mutex::new(None),
        }
    }
}

impl Engines for RealEngines {
    fn mic(&self) -> Box<dyn AudioSource> {
        Box::new(MicSource::default_device())
    }

    fn system(&self) -> Box<dyn AudioSource> {
        Box::new(SystemTapSource::new())
    }

    fn transcriber(&self, settings: &Settings) -> Result<Arc<dyn Transcriber>> {
        match settings.transcribe_mode {
            TranscribeMode::Local => {
                let model = settings.local_model;
                let mut cached = self.local.lock().expect("local transcriber lock");
                if let Some((m, lang, t)) = cached.as_ref()
                    && *m == model
                    && *lang == settings.language
                {
                    return Ok(t.clone());
                }
                let dir = model.dir_in(&self.models_root);
                if !model_ready(&dir, model) {
                    return Err(MorselError::Invalid(
                        "download the transcription model in Settings first".into(),
                    ));
                }
                let loaded: Arc<dyn Transcriber> = match model {
                    LocalModel::Parakeet => Arc::new(ParakeetTranscriber::new(&dir)?),
                    LocalModel::WhisperTurbo => {
                        Arc::new(WhisperTranscriber::new(&dir, &settings.language)?)
                    }
                };
                *cached = Some((model, settings.language.clone(), loaded.clone()));
                Ok(loaded)
            }
            TranscribeMode::Cloud => {
                let key = self.secrets.get(TRANSCRIBE_KEY)?.ok_or_else(|| {
                    MorselError::Invalid("add the cloud transcription key in Settings first".into())
                })?;
                Ok(Arc::new(CloudTranscriber::new(
                    self.http.clone(),
                    settings.transcribe_base_url.clone(),
                    settings.transcribe_model.clone(),
                    key,
                )))
            }
        }
    }
}

/// Where Morsel keeps its data, the count of notes, and the size on disk without the speech model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataInfo {
    pub path: String,
    pub notes: u32,
    pub bytes: u64,
}

const THIRTY_DAYS_MS: i64 = 30 * 86_400_000;

/// The quick model, or the notes model when the quick model is empty.
fn quick_model_name(settings: &Settings) -> &str {
    if settings.quick_model.trim().is_empty() {
        &settings.notes_model
    } else {
        &settings.quick_model
    }
}

/// A file name that every file system accepts: no path or wildcard characters.
fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        "Untitled".to_owned()
    } else {
        trimmed.chars().take(120).collect()
    }
}

pub struct AppState {
    pub db: SharedDb,
    pub secrets: Arc<dyn SecretStore>,
    pub http: reqwest::Client,
    pub sink: Arc<dyn EventSink>,
    engines: Arc<dyn Engines>,
    data_dir: PathBuf,
    models_root: PathBuf,
    session: tokio::sync::Mutex<Option<MeetingSession>>,
    notes_jobs: Arc<Mutex<HashMap<MeetingId, CancellationToken>>>,
}

impl AppState {
    pub fn new(
        db: SharedDb,
        secrets: Arc<dyn SecretStore>,
        http: reqwest::Client,
        sink: Arc<dyn EventSink>,
        engines: Arc<dyn Engines>,
        data_dir: PathBuf,
        models_root: PathBuf,
    ) -> Self {
        Self {
            db,
            secrets,
            http,
            sink,
            engines,
            data_dir,
            models_root,
            session: tokio::sync::Mutex::new(None),
            notes_jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        self.db.lock().expect("db lock")
    }

    pub fn settings(&self) -> Result<Settings> {
        settings::load_settings(&self.conn())
    }

    pub async fn start_meeting(
        &self,
        title: Option<String>,
        template_id: i64,
    ) -> Result<MeetingId> {
        self.start_meeting_with(title, template_id, Vec::new())
            .await
    }

    /// Starts a meeting with the people of its calendar event.
    pub async fn start_meeting_with(
        &self,
        title: Option<String>,
        template_id: i64,
        people: Vec<String>,
    ) -> Result<MeetingId> {
        let mut slot = self.session.lock().await;
        if slot.is_some() {
            return Err(MorselError::AlreadyRecording);
        }
        let settings = self.settings()?;
        let transcriber = self.engines.transcriber(&settings)?;
        let title = title
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| UNTITLED.into());
        let (session, id) = MeetingSession::start(
            self.db.clone(),
            self.engines.mic(),
            self.engines.system(),
            transcriber,
            self.sink.clone(),
            &title,
            template_id,
            Some(settings.language),
        )?;
        *slot = Some(session);
        if !people.is_empty() {
            let conn = self.db.lock().map_err(|_| MorselError::Db("lock".into()))?;
            meetings::set_people(&conn, id, &people)?;
        }
        Ok(id)
    }

    /// The id of the meeting that records now.
    pub async fn recording_meeting(&self) -> Option<MeetingId> {
        self.session.lock().await.as_ref().map(|s| s.id())
    }

    pub async fn pause_meeting(&self) -> Result<()> {
        self.session
            .lock()
            .await
            .as_ref()
            .ok_or(MorselError::NotRecording)?
            .pause()
    }

    pub async fn resume_meeting(&self) -> Result<()> {
        self.session
            .lock()
            .await
            .as_ref()
            .ok_or(MorselError::NotRecording)?
            .resume()
    }

    pub async fn stop_meeting(&self) -> Result<()> {
        let session = self
            .session
            .lock()
            .await
            .take()
            .ok_or(MorselError::NotRecording)?;
        session.stop().await
    }

    pub fn save_user_notes(&self, meeting_id: MeetingId, body: &str) -> Result<()> {
        meetings::save_user_notes(&self.conn(), meeting_id, body)
    }

    pub fn get_meeting(&self, meeting_id: MeetingId) -> Result<MeetingDetail> {
        meetings::get_meeting(&self.conn(), meeting_id)
    }

    pub fn list_meetings(&self, limit: u32) -> Result<Vec<MeetingSummary>> {
        meetings::list_meetings(&self.conn(), limit)
    }

    pub fn list_templates(&self) -> Result<Vec<Template>> {
        templates::list(&self.conn())
    }

    pub fn save_settings(&self, value: &Settings) -> Result<()> {
        settings::save_settings(&self.conn(), value)
    }

    /// An empty key deletes the saved key.
    pub fn set_api_key(&self, account: &str, key: &str) -> Result<()> {
        if !is_known_account(account) {
            return Err(MorselError::Invalid(format!(
                "unknown key account {account}"
            )));
        }
        if key.trim().is_empty() {
            self.secrets.delete(account)
        } else {
            self.secrets.set(account, key.trim())
        }
    }

    /// Tells only if a key is saved. The key itself never leaves the backend.
    pub fn has_api_key(&self, account: &str) -> Result<bool> {
        if !is_known_account(account) {
            return Err(MorselError::Invalid(format!(
                "unknown key account {account}"
            )));
        }
        Ok(self.secrets.get(account)?.is_some())
    }

    pub fn notes_config(&self, model: &str) -> Result<LlmConfig> {
        let settings = self.settings()?;
        let api_key = self.secrets.get(NOTES_KEY)?.ok_or_else(|| {
            MorselError::Invalid("add the notes API key in Settings first".into())
        })?;
        Ok(LlmConfig {
            base_url: settings.notes_base_url,
            model: model.to_owned(),
            api_key,
        })
    }

    /// Starts writing the note in the background. The result comes as a `notes://done` or `notes://failed` event.
    pub fn write_notes(&self, meeting_id: MeetingId, template_id: i64) -> Result<()> {
        let settings = self.settings()?;
        let notes = self.notes_config(&settings.notes_model)?;
        if notes.model.trim().is_empty() {
            return Err(MorselError::Invalid(
                "choose a notes model in Settings first".into(),
            ));
        }
        let quick = LlmConfig {
            model: quick_model_name(&settings).to_owned(),
            ..notes.clone()
        };
        let ctx = NotesContext {
            db: self.db.clone(),
            http: self.http.clone(),
            sink: self.sink.clone(),
            notes,
            quick,
        };
        let cancel = CancellationToken::new();
        let jobs = self.notes_jobs.clone();
        if let Some(old) = jobs
            .lock()
            .expect("jobs lock")
            .insert(meeting_id, cancel.clone())
        {
            old.cancel();
        }
        tokio::spawn(async move {
            let _ = crate::notes::writer::write_notes(&ctx, meeting_id, template_id, cancel).await;
            jobs.lock().expect("jobs lock").remove(&meeting_id);
        });
        Ok(())
    }

    /// Cancels the note of a meeting. Returns false when no note is being written.
    pub fn cancel_notes(&self, meeting_id: MeetingId) -> bool {
        match self.notes_jobs.lock().expect("jobs lock").get(&meeting_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Writes the note as a Markdown file at `path`, which the user picked in a save dialog.
    pub fn export_note(&self, meeting_id: MeetingId, path: &std::path::Path) -> Result<()> {
        let detail = self.get_meeting(meeting_id)?;
        let note = detail
            .note
            .ok_or_else(|| MorselError::NotFound(format!("note of meeting {meeting_id}")))?;
        std::fs::write(
            path,
            format!("# {}\n\n{}\n", detail.summary.title, note.trim_end()),
        )?;
        Ok(())
    }

    pub fn search_notes(&self, filter: &NotesFilter) -> Result<NotesPage> {
        library::search_meetings(&self.conn(), filter)
    }

    pub fn list_people(&self) -> Result<Vec<String>> {
        library::list_people(&self.conn())
    }

    /// Deletes a meeting and everything in it. The meeting that records now cannot be deleted.
    pub async fn delete_meeting(&self, meeting_id: MeetingId) -> Result<()> {
        if let Some(session) = self.session.lock().await.as_ref()
            && session.id() == meeting_id
        {
            return Err(MorselError::AlreadyRecording);
        }
        self.cancel_notes(meeting_id);
        library::delete_meeting(&self.conn(), meeting_id)
    }

    /// Creates a template when `id` is `None`, else changes it. Returns the id.
    pub fn save_template(&self, id: Option<i64>, input: &TemplateInput) -> Result<i64> {
        let conn = self.conn();
        match id {
            Some(id) => templates::update(&conn, id, input).map(|_| id),
            None => templates::create(&conn, input),
        }
    }

    pub fn duplicate_template(&self, id: i64) -> Result<i64> {
        templates::duplicate(&self.conn(), id)
    }

    pub fn delete_template(&self, id: i64) -> Result<()> {
        templates::delete(&self.conn(), id)
    }

    pub fn reset_template(&self, id: i64) -> Result<()> {
        templates::reset(&self.conn(), id)
    }

    pub async fn ask(
        &self,
        question: &str,
        range: AskRange,
        meeting_id: Option<MeetingId>,
    ) -> Result<AskAnswer> {
        let settings = self.settings()?;
        let quick = self.notes_config(quick_model_name(&settings))?;
        if quick.model.trim().is_empty() {
            return Err(MorselError::Invalid(
                "choose a notes model in Settings first".into(),
            ));
        }
        ask::ask(&self.db, &self.http, &quick, question, range, meeting_id).await
    }

    pub fn list_asks(&self, limit: u32) -> Result<Vec<AskAnswer>> {
        library::list_asks(&self.conn(), limit)
    }

    pub fn get_ask(&self, id: i64) -> Result<AskAnswer> {
        library::get_ask(&self.conn(), id)
    }

    /// The tokens of the last 30 days.
    pub fn usage_summary(&self) -> Result<UsageTotal> {
        library::usage_since(&self.conn(), now_ms() - THIRTY_DAYS_MS)
    }

    pub fn data_info(&self) -> Result<DataInfo> {
        let notes = library::count_meetings(&self.conn())?;
        let mut bytes = 0;
        if let Ok(entries) = std::fs::read_dir(&self.data_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata()
                    && meta.is_file()
                {
                    bytes += meta.len();
                }
            }
        }
        Ok(DataInfo {
            path: self.data_dir.to_string_lossy().into_owned(),
            notes,
            bytes,
        })
    }

    /// Writes each finished note as `<date> <title>.md` into `dir`. Returns the count of files.
    pub fn export_all(&self, dir: &std::path::Path) -> Result<u32> {
        let rows: Vec<(String, String, String)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare(
                "SELECT date(m.started_at / 1000, 'unixepoch', 'localtime'), m.title, n.body_md \
                 FROM meetings m JOIN notes n ON n.meeting_id = m.id ORDER BY m.started_at, m.id",
            )?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut used = std::collections::HashSet::new();
        for (date, title, body) in &rows {
            let base = format!("{date} {}", safe_file_name(title));
            let mut name = format!("{base}.md");
            let mut n = 2;
            while !used.insert(name.clone()) || dir.join(&name).exists() {
                name = format!("{base} ({n}).md");
                n += 1;
            }
            std::fs::write(
                dir.join(&name),
                format!("# {title}\n\n{}\n", body.trim_end()),
            )?;
        }
        Ok(rows.len() as u32)
    }

    /// Deletes every meeting, note, question, usage row, setting and key, and resets the templates.
    pub async fn delete_all_data(&self) -> Result<()> {
        let session = self.session.lock().await;
        if session.is_some() {
            return Err(MorselError::AlreadyRecording);
        }
        for token in self.notes_jobs.lock().expect("jobs lock").values() {
            token.cancel();
        }
        {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            tx.execute_batch(
                "DELETE FROM meetings; DELETE FROM meeting_search; DELETE FROM asks; \
                 DELETE FROM usage; DELETE FROM settings; DELETE FROM templates WHERE built_in = 0;",
            )?;
            templates::write_all_built_ins(&tx)?;
            tx.commit()?;
            conn.execute_batch("VACUUM;")?;
        }
        for account in ACCOUNTS {
            self.secrets.delete(account)?;
        }
        drop(session);
        Ok(())
    }

    pub async fn test_notes_key(&self) -> Result<()> {
        let model = self.settings()?.notes_model;
        llm::test_key(&self.http, &self.notes_config(&model)?).await
    }

    pub fn model_ready(&self, model: LocalModel) -> bool {
        model_ready(&model.dir_in(&self.models_root), model)
    }

    pub async fn download_model(&self, model: LocalModel) -> Result<()> {
        let sink = self.sink.clone();
        ensure_model(
            &self.http,
            &model.dir_in(&self.models_root),
            model,
            move |received, total| {
                sink.emit(
                    MODEL_DOWNLOAD,
                    json!({ "received": received, "total": total }),
                );
            },
        )
        .await
        .map(|_| ())
    }
}

#[tauri::command]
pub async fn start_meeting(
    state: State<'_, AppState>,
    title: Option<String>,
    template_id: i64,
    people: Option<Vec<String>>,
) -> Result<MeetingId> {
    state
        .start_meeting_with(title, template_id, people.unwrap_or_default())
        .await
}

#[tauri::command]
pub async fn pause_meeting(state: State<'_, AppState>) -> Result<()> {
    state.pause_meeting().await
}

#[tauri::command]
pub async fn resume_meeting(state: State<'_, AppState>) -> Result<()> {
    state.resume_meeting().await
}

#[tauri::command]
pub async fn stop_meeting(state: State<'_, AppState>) -> Result<()> {
    state.stop_meeting().await
}

#[tauri::command]
pub async fn save_user_notes(
    state: State<'_, AppState>,
    meeting_id: MeetingId,
    body: String,
) -> Result<()> {
    state.save_user_notes(meeting_id, &body)
}

#[tauri::command]
pub async fn get_meeting(
    state: State<'_, AppState>,
    meeting_id: MeetingId,
) -> Result<MeetingDetail> {
    state.get_meeting(meeting_id)
}

#[tauri::command]
pub async fn list_meetings(state: State<'_, AppState>, limit: u32) -> Result<Vec<MeetingSummary>> {
    state.list_meetings(limit)
}

#[tauri::command]
pub async fn list_templates(state: State<'_, AppState>) -> Result<Vec<Template>> {
    state.list_templates()
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings> {
    state.settings()
}

#[tauri::command]
pub async fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<()> {
    state.save_settings(&settings)
}

#[tauri::command]
pub async fn set_api_key(state: State<'_, AppState>, account: String, key: String) -> Result<()> {
    state.set_api_key(&account, &key)
}

#[tauri::command]
pub async fn has_api_key(state: State<'_, AppState>, account: String) -> Result<bool> {
    state.has_api_key(&account)
}

#[tauri::command]
pub async fn write_notes(
    state: State<'_, AppState>,
    meeting_id: MeetingId,
    template_id: i64,
) -> Result<()> {
    state.write_notes(meeting_id, template_id)
}

#[tauri::command]
pub async fn cancel_notes(state: State<'_, AppState>, meeting_id: MeetingId) -> Result<bool> {
    Ok(state.cancel_notes(meeting_id))
}

#[tauri::command]
pub async fn export_note(
    state: State<'_, AppState>,
    meeting_id: MeetingId,
    path: PathBuf,
) -> Result<()> {
    state.export_note(meeting_id, &path)
}

#[tauri::command]
pub async fn test_notes_key(state: State<'_, AppState>) -> Result<()> {
    state.test_notes_key().await
}

#[tauri::command]
pub async fn model_status(state: State<'_, AppState>, model: LocalModel) -> Result<bool> {
    Ok(state.model_ready(model))
}

#[tauri::command]
pub async fn download_model(state: State<'_, AppState>, model: LocalModel) -> Result<()> {
    state.download_model(model).await
}

#[tauri::command]
pub async fn search_notes(state: State<'_, AppState>, filter: NotesFilter) -> Result<NotesPage> {
    state.search_notes(&filter)
}

#[tauri::command]
pub async fn list_people(state: State<'_, AppState>) -> Result<Vec<String>> {
    state.list_people()
}

#[tauri::command]
pub async fn delete_meeting(state: State<'_, AppState>, meeting_id: MeetingId) -> Result<()> {
    state.delete_meeting(meeting_id).await
}

#[tauri::command]
pub async fn save_template(
    state: State<'_, AppState>,
    id: Option<i64>,
    input: TemplateInput,
) -> Result<i64> {
    state.save_template(id, &input)
}

#[tauri::command]
pub async fn duplicate_template(state: State<'_, AppState>, id: i64) -> Result<i64> {
    state.duplicate_template(id)
}

#[tauri::command]
pub async fn delete_template(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.delete_template(id)
}

#[tauri::command]
pub async fn reset_template(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.reset_template(id)
}

#[tauri::command]
pub async fn ask(
    state: State<'_, AppState>,
    question: String,
    range: AskRange,
    meeting_id: Option<MeetingId>,
) -> Result<AskAnswer> {
    state.ask(&question, range, meeting_id).await
}

#[tauri::command]
pub async fn list_asks(state: State<'_, AppState>, limit: u32) -> Result<Vec<AskAnswer>> {
    state.list_asks(limit)
}

#[tauri::command]
pub async fn get_ask(state: State<'_, AppState>, id: i64) -> Result<AskAnswer> {
    state.get_ask(id)
}

#[tauri::command]
pub async fn usage_summary(state: State<'_, AppState>) -> Result<UsageTotal> {
    state.usage_summary()
}

#[tauri::command]
pub async fn data_info(state: State<'_, AppState>) -> Result<DataInfo> {
    state.data_info()
}

#[tauri::command]
pub async fn export_all(state: State<'_, AppState>, dir: PathBuf) -> Result<u32> {
    state.export_all(&dir)
}

#[tauri::command]
pub async fn delete_all_data(state: State<'_, AppState>) -> Result<()> {
    state.delete_all_data().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::source::AudioSource;
    use crate::events::VecSink;
    use crate::secrets::MemoryStore;
    use crate::transcribe::Transcriber;
    use tokio::sync::mpsc::Sender;

    struct Silent;
    impl AudioSource for Silent {
        fn start(&mut self, _tx: Sender<Vec<f32>>) -> Result<()> {
            Ok(())
        }
        fn stop(&mut self) {}
    }

    struct Echo;
    #[async_trait::async_trait]
    impl Transcriber for Echo {
        async fn transcribe(&self, _s: &[f32], _l: Option<&str>) -> Result<String> {
            Ok("text".into())
        }
    }

    struct FakeEngines;
    impl Engines for FakeEngines {
        fn mic(&self) -> Box<dyn AudioSource> {
            Box::new(Silent)
        }
        fn system(&self) -> Box<dyn AudioSource> {
            Box::new(Silent)
        }
        fn transcriber(&self, _settings: &Settings) -> Result<Arc<dyn Transcriber>> {
            Ok(Arc::new(Echo))
        }
    }

    fn state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::open(&dir.path().join("m.db")).unwrap();
        let state = AppState::new(
            Arc::new(std::sync::Mutex::new(db)),
            Arc::new(MemoryStore::default()),
            reqwest::Client::new(),
            Arc::new(VecSink::default()),
            Arc::new(FakeEngines),
            dir.path().to_path_buf(),
            dir.path().join("models"),
        );
        (dir, state)
    }

    #[test]
    fn set_api_key_rejects_unknown_account() {
        let (_d, s) = state();
        assert!(matches!(
            s.set_api_key("other", "x"),
            Err(MorselError::Invalid(_))
        ));
        s.set_api_key(NOTES_KEY, "sk-1").unwrap();
        assert!(s.has_api_key(NOTES_KEY).unwrap());
    }

    #[test]
    fn has_api_key_never_returns_the_key() {
        let (_d, s) = state();
        s.set_api_key(NOTES_KEY, "sk-secret").unwrap();
        let response = serde_json::to_value(s.has_api_key(NOTES_KEY).unwrap()).unwrap();
        assert_eq!(response, serde_json::json!(true));
        s.set_api_key(NOTES_KEY, "").unwrap();
        assert!(!s.has_api_key(NOTES_KEY).unwrap());
    }

    #[tokio::test]
    async fn start_meeting_twice_returns_already_recording_error() {
        let (_d, s) = state();
        let id = s.start_meeting(None, 1).await.unwrap();
        assert!(matches!(
            s.start_meeting(None, 1).await,
            Err(MorselError::AlreadyRecording)
        ));
        assert_eq!(s.get_meeting(id).unwrap().summary.title, "Untitled meeting");
        s.stop_meeting().await.unwrap();
        assert!(matches!(
            s.stop_meeting().await,
            Err(MorselError::NotRecording)
        ));
    }

    #[tokio::test]
    async fn write_notes_needs_a_notes_key() {
        let (_d, s) = state();
        let id = s.start_meeting(Some("Sync".into()), 1).await.unwrap();
        s.stop_meeting().await.unwrap();
        assert!(matches!(s.write_notes(id, 1), Err(MorselError::Invalid(_))));
        assert!(!s.cancel_notes(id));
    }

    #[test]
    fn export_note_writes_markdown_file() {
        let (dir, s) = state();
        let id =
            crate::db::meetings::create_meeting(&s.db.lock().unwrap(), "Weekly sync", 1).unwrap();
        crate::db::meetings::save_note(&s.db.lock().unwrap(), id, "## Summary\n- budget ok", "m")
            .unwrap();
        let path = dir.path().join("out.md");
        s.export_note(id, &path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# Weekly sync\n\n## Summary\n- budget ok\n"
        );
    }

    #[test]
    fn export_without_note_is_an_error() {
        let (dir, s) = state();
        let id =
            crate::db::meetings::create_meeting(&s.db.lock().unwrap(), "Weekly sync", 1).unwrap();
        assert!(matches!(
            s.export_note(id, &dir.path().join("x.md")),
            Err(MorselError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn pause_without_meeting_returns_not_recording() {
        let (_d, s) = state();
        assert!(matches!(
            s.pause_meeting().await,
            Err(MorselError::NotRecording)
        ));
    }

    fn finished(s: &AppState, title: &str, note: Option<&str>) -> MeetingId {
        let conn = s.db.lock().unwrap();
        let id = crate::db::meetings::create_meeting(&conn, title, 1).unwrap();
        conn.execute(
            "UPDATE meetings SET started_at = 1762560000000 WHERE id = ?1",
            [id],
        )
        .unwrap();
        if let Some(note) = note {
            crate::db::meetings::save_note(&conn, id, note, "m").unwrap();
        }
        id
    }

    fn file_names(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn export_all_writes_one_file_per_note() {
        let (dir, s) = state();
        finished(&s, "Weekly sync", Some("## Summary\n- ok"));
        finished(&s, "Pricing", Some("## Budget\n- 20k"));
        finished(&s, "No note yet", None);
        let out = dir.path().join("export");
        std::fs::create_dir(&out).unwrap();
        assert_eq!(s.export_all(&out).unwrap(), 2);
        let names = file_names(&out);
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n| n.ends_with(" Weekly sync.md")));
        let pricing = names.iter().find(|n| n.ends_with(" Pricing.md")).unwrap();
        assert_eq!(
            std::fs::read_to_string(out.join(pricing)).unwrap(),
            "# Pricing\n\n## Budget\n- 20k\n"
        );
    }

    #[test]
    fn export_all_makes_safe_unique_names() {
        let (dir, s) = state();
        finished(&s, "A/B: plan?", Some("x"));
        finished(&s, "A/B: plan?", Some("y"));
        let out = dir.path().join("export");
        std::fs::create_dir(&out).unwrap();
        s.export_all(&out).unwrap();
        let names = file_names(&out);
        assert_eq!(names.len(), 2);
        assert!(
            names[0].ends_with(" A-B- plan- (2).md") || names[1].ends_with(" A-B- plan- (2).md")
        );
        assert!(
            names
                .iter()
                .all(|n| !n.contains('/') && !n.contains(':') && !n.contains('?'))
        );
    }

    #[tokio::test]
    async fn delete_all_clears_data_and_keys_and_keeps_built_ins() {
        let (_d, s) = state();
        finished(&s, "Gone", Some("note"));
        s.set_api_key(NOTES_KEY, "sk-1").unwrap();
        s.set_api_key(TRANSCRIBE_KEY, "gsk-1").unwrap();
        let mut changed = s.settings().unwrap();
        changed.language = "de".into();
        s.save_settings(&changed).unwrap();
        let custom = s
            .save_template(
                None,
                &templates::TemplateInput {
                    name: "Mine".into(),
                    description: String::new(),
                    sections: vec![templates::Section {
                        name: "A".into(),
                        hint: String::new(),
                    }],
                    instructions: String::new(),
                },
            )
            .unwrap();
        s.save_template(
            Some(2),
            &templates::TemplateInput {
                name: "Changed".into(),
                description: String::new(),
                sections: vec![templates::Section {
                    name: "A".into(),
                    hint: String::new(),
                }],
                instructions: String::new(),
            },
        )
        .unwrap();

        s.delete_all_data().await.unwrap();

        assert!(s.list_meetings(10).unwrap().is_empty());
        assert!(!s.has_api_key(NOTES_KEY).unwrap());
        assert!(!s.has_api_key(TRANSCRIBE_KEY).unwrap());
        assert_eq!(s.settings().unwrap(), Settings::default());
        let all = s.list_templates().unwrap();
        assert_eq!(all.len(), 5);
        assert_eq!(all[1].name, "1:1");
        assert!(all.iter().all(|t| t.id != custom));
        assert_eq!(s.data_info().unwrap().notes, 0);
    }

    #[tokio::test]
    async fn recording_meeting_is_the_live_one() {
        let (_d, s) = state();
        assert_eq!(s.recording_meeting().await, None);
        let id = s.start_meeting(None, 1).await.unwrap();
        assert_eq!(s.recording_meeting().await, Some(id));
        s.stop_meeting().await.unwrap();
        assert_eq!(s.recording_meeting().await, None);
    }

    #[tokio::test]
    async fn start_meeting_saves_the_people() {
        let (_d, s) = state();
        let id = s
            .start_meeting_with(
                Some("Weekly product sync".into()),
                1,
                vec!["Sam".into(), " ".into(), "Priya".into()],
            )
            .await
            .unwrap();
        s.stop_meeting().await.unwrap();
        let summary = s.get_meeting(id).unwrap().summary;
        assert_eq!(summary.title, "Weekly product sync");
        assert_eq!(summary.people, vec!["Sam", "Priya"]);
    }

    #[tokio::test]
    async fn delete_all_data_removes_the_composio_key() {
        let (_d, s) = state();
        s.set_api_key(crate::secrets::COMPOSIO_KEY, "ck-secret")
            .unwrap();
        {
            let db = s.db.lock().unwrap();
            crate::calendar::service::save_link(
                &db,
                &crate::calendar::service::CalendarLink {
                    user_id: "morsel-x".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        s.delete_all_data().await.unwrap();
        assert!(!s.has_api_key(crate::secrets::COMPOSIO_KEY).unwrap());
        let db = s.db.lock().unwrap();
        assert_eq!(
            crate::calendar::service::load_link(&db).unwrap(),
            Default::default()
        );
    }

    #[tokio::test]
    async fn delete_all_refuses_while_recording() {
        let (_d, s) = state();
        s.start_meeting(None, 1).await.unwrap();
        assert!(matches!(
            s.delete_all_data().await,
            Err(MorselError::AlreadyRecording)
        ));
        assert_eq!(s.list_meetings(10).unwrap().len(), 1);
        s.stop_meeting().await.unwrap();
    }

    #[tokio::test]
    async fn delete_recording_meeting_is_refused() {
        let (_d, s) = state();
        let id = s.start_meeting(None, 1).await.unwrap();
        assert!(matches!(
            s.delete_meeting(id).await,
            Err(MorselError::AlreadyRecording)
        ));
        s.stop_meeting().await.unwrap();
        s.delete_meeting(id).await.unwrap();
        assert!(s.get_meeting(id).is_err());
    }

    #[test]
    fn usage_summary_counts_last_30_days() {
        let (_d, s) = state();
        {
            let conn = s.db.lock().unwrap();
            crate::db::meetings::record_usage(&conn, "m", 100, 10, None).unwrap();
            crate::db::meetings::record_usage(&conn, "m", 7, 3, None).unwrap();
            conn.execute("UPDATE usage SET at = at - 31 * 86400000 WHERE id = 1", [])
                .unwrap();
        }
        let total = s.usage_summary().unwrap();
        assert_eq!(total.prompt_tokens, 7);
        assert_eq!(total.completion_tokens, 3);
        assert_eq!(total.cost, None);
    }

    #[test]
    fn data_info_counts_notes_and_skips_the_model() {
        let (dir, s) = state();
        finished(&s, "One", Some("x"));
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path().join("models").join("big.bin"),
            vec![0u8; 5_000_000],
        )
        .unwrap();
        let info = s.data_info().unwrap();
        assert_eq!(info.notes, 1);
        assert!(
            info.bytes > 0 && info.bytes < 5_000_000,
            "bytes {}",
            info.bytes
        );
        assert_eq!(info.path, dir.path().to_string_lossy());
    }
}

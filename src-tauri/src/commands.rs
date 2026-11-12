//! The commands that the frontend calls. Each command is a thin call into an `AppState` method.

use crate::ask::{self, AskAnswer, AskRange};
use crate::audio::recording::audio_dir;
use crate::audio::{mic::MicSource, source::AudioSource, system_tap::SystemTapSource};
use crate::chat_model::{ChatModel, CloudChat};
use crate::db::library::{self, NotesFilter, NotesPage, UsageTotal};
use crate::db::meetings::{self, MeetingDetail, MeetingSummary};
use crate::db::templates::{self, Template, TemplateInput};
use crate::db::{SharedDb, now_ms};
use crate::events::{
    EventSink, MODEL_DOWNLOAD, NOTES_FAILED, NOTES_MODEL_DOWNLOAD, SEARCH_MODEL_DOWNLOAD, STATUS,
};
use crate::llm::{self, LlmConfig};
use crate::local_llm::{
    LocalLlm, NOTES_FILE, notes_model_dir, notes_model_files, notes_model_ready,
};
use crate::model_manager::{LocalChat, ModelManager, Purpose, WhisperControl};
use crate::notes::writer::{CancellationToken, NotesContext};
use crate::search::embed::{
    LocalEmbedder, ManagedEmbed, SEARCH_FILE, search_model_dir, search_model_files,
};
use crate::search::index::{Embed, index_all};
use crate::secrets::{ACCOUNTS, NOTES_KEY, SecretStore, TRANSCRIBE_KEY, is_known_account};
use crate::session::{MeetingSession, MeetingSetup};
use crate::settings::{self, NotesMode, Settings, TranscribeMode, TranscribeTiming};
use crate::transcribe::Transcriber;
use crate::transcribe::after::{AfterQueue, transcribe_meeting};
use crate::transcribe::cloud::CloudTranscriber;
use crate::transcribe::model_download::file_ready;
use crate::transcribe::model_download::{ensure_files, ensure_model, model_dir, model_ready};
use crate::transcribe::whisper::WhisperTranscriber;
use crate::types::{MeetingId, MeetingStatus, MorselError, Result};
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
    /// True while a recording or a transcription uses the on-device Whisper model.
    fn whisper_in_use(&self) -> bool {
        false
    }
    /// Frees the cached Whisper model when nothing uses it.
    fn free_whisper(&self) {}
}

/// Lets the local notes model see Whisper through the engines.
pub struct EnginesWhisper(pub Arc<dyn Engines>);

impl WhisperControl for EnginesWhisper {
    fn in_use(&self) -> bool {
        self.0.whisper_in_use()
    }
    fn free(&self) {
        self.0.free_whisper()
    }
}

/// The loaded on-device transcriber, with the language that it was loaded for.
type LoadedLocal = (String, Arc<dyn Transcriber>);

/// The real microphone, the system tap, and Whisper Turbo or the cloud.
pub struct RealEngines {
    models_root: PathBuf,
    http: reqwest::Client,
    secrets: Arc<dyn SecretStore>,
    local: Mutex<Option<LoadedLocal>>,
    notes_model: Arc<ModelManager<LocalLlm>>,
}

impl RealEngines {
    pub fn new(
        models_root: PathBuf,
        http: reqwest::Client,
        secrets: Arc<dyn SecretStore>,
        notes_model: Arc<ModelManager<LocalLlm>>,
    ) -> Self {
        Self {
            models_root,
            http,
            secrets,
            local: Mutex::new(None),
            notes_model,
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
                let mut cached = self.local.lock().expect("local transcriber lock");
                if let Some((lang, t)) = cached.as_ref()
                    && *lang == settings.language
                {
                    return Ok(t.clone());
                }
                // An 8 GB Mac cannot hold Whisper and the notes model at the same time.
                self.notes_model.unload();
                let dir = model_dir(&self.models_root);
                if !model_ready(&dir) {
                    return Err(MorselError::Invalid(
                        "download the transcription model in Settings first".into(),
                    ));
                }
                let loaded: Arc<dyn Transcriber> =
                    Arc::new(WhisperTranscriber::new(&dir, &settings.language)?);
                *cached = Some((settings.language.clone(), loaded.clone()));
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

    fn whisper_in_use(&self) -> bool {
        let cached = self.local.lock().expect("local transcriber lock");
        cached
            .as_ref()
            .is_some_and(|(_, t)| Arc::strong_count(t) > 1)
    }

    fn free_whisper(&self) {
        let mut cached = self.local.lock().expect("local transcriber lock");
        if cached
            .as_ref()
            .is_some_and(|(_, t)| Arc::strong_count(t) == 1)
        {
            *cached = None;
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

/// The size of all files in a folder and its subfolders. A missing folder has size 0.
fn folder_size(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => folder_size(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

/// Deletes a folder and its files. A missing folder is not an error.
fn remove_folder(dir: &std::path::Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
        _ => Ok(()),
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

/// The on-device models: the folder that holds their files, and the loaded notes model.
pub struct LocalModels {
    pub root: PathBuf,
    pub notes_model: Arc<ModelManager<LocalLlm>>,
    pub search_model: Arc<ModelManager<LocalEmbedder>>,
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
    after: AfterQueue,
    notes_model: Arc<ModelManager<LocalLlm>>,
    search_model: Arc<ModelManager<LocalEmbedder>>,
}

impl AppState {
    pub fn new(
        db: SharedDb,
        secrets: Arc<dyn SecretStore>,
        http: reqwest::Client,
        sink: Arc<dyn EventSink>,
        engines: Arc<dyn Engines>,
        data_dir: PathBuf,
        models: LocalModels,
    ) -> Self {
        let LocalModels {
            root: models_root,
            notes_model,
            search_model,
        } = models;
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
            after: AfterQueue::default(),
            notes_model,
            search_model,
        }
    }

    /// The settings for the Settings screen, with the notes mode that Morsel uses now.
    pub fn shown_settings(&self) -> Result<Settings> {
        let mut settings = self.settings()?;
        settings.notes_mode = Some(self.notes_mode()?);
        Ok(settings)
    }

    /// The saved choice, or Cloud for an install that already has a notes key, or Local.
    pub fn notes_mode(&self) -> Result<NotesMode> {
        if let Some(mode) = self.settings()?.notes_mode {
            return Ok(mode);
        }
        Ok(if self.secrets.get(NOTES_KEY)?.is_some() {
            NotesMode::Cloud
        } else {
            NotesMode::Local
        })
    }

    fn local_chat(&self, purpose: Purpose) -> Result<Arc<dyn ChatModel>> {
        let dir = notes_model_dir(&self.models_root);
        if !notes_model_ready(&dir) {
            return Err(MorselError::Invalid(
                "download the notes model in Settings first".into(),
            ));
        }
        Ok(Arc::new(LocalChat::new(
            self.notes_model.clone(),
            dir.join(NOTES_FILE),
            Arc::new(EnginesWhisper(self.engines.clone())),
            purpose,
        )))
    }

    /// The notes model and the quick model (titles, Ask) for the chosen mode.
    fn chat_models(&self, purpose: Purpose) -> Result<(Arc<dyn ChatModel>, Arc<dyn ChatModel>)> {
        if self.notes_mode()? == NotesMode::Local {
            let local = self.local_chat(purpose)?;
            return Ok((local.clone(), local));
        }
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
        Ok((
            Arc::new(CloudChat::new(self.http.clone(), notes)),
            Arc::new(CloudChat::new(self.http.clone(), quick)),
        ))
    }

    pub fn notes_model_ready(&self) -> bool {
        notes_model_ready(&notes_model_dir(&self.models_root))
    }

    pub async fn download_notes_model(&self) -> Result<()> {
        let sink = self.sink.clone();
        ensure_files(
            &self.http,
            &notes_model_dir(&self.models_root),
            &notes_model_files(),
            move |received, total| {
                sink.emit(
                    NOTES_MODEL_DOWNLOAD,
                    json!({ "received": received, "total": total }),
                );
            },
        )
        .await
    }

    pub fn search_model_ready(&self) -> bool {
        let dir = search_model_dir(&self.models_root);
        search_model_files().iter().all(|f| file_ready(&dir, f))
    }

    /// The search model, or `None` when it is not downloaded.
    pub fn search_embed(&self) -> Option<Arc<dyn Embed>> {
        self.search_model_ready().then(|| {
            Arc::new(ManagedEmbed::new(
                self.search_model.clone(),
                search_model_dir(&self.models_root).join(SEARCH_FILE),
            )) as Arc<dyn Embed>
        })
    }

    /// Indexes old meetings, and embeds parts that have no embedding, on a background thread.
    pub fn start_indexing(&self) -> std::thread::JoinHandle<()> {
        let db = self.db.clone();
        let sink = self.sink.clone();
        let embed = self.search_embed();
        std::thread::spawn(move || {
            if let Err(err) = index_all(&db, embed.as_deref(), sink.as_ref()) {
                eprintln!("cannot index the meetings for search: {err}");
            }
        })
    }

    /// Downloads the search model, then embeds the parts that have no embedding.
    pub async fn download_search_model(&self) -> Result<()> {
        let sink = self.sink.clone();
        ensure_files(
            &self.http,
            &search_model_dir(&self.models_root),
            &search_model_files(),
            move |received, total| {
                sink.emit(
                    SEARCH_MODEL_DOWNLOAD,
                    json!({ "received": received, "total": total }),
                );
            },
        )
        .await?;
        self.start_indexing();
        Ok(())
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
            MeetingSetup {
                title,
                template_id,
                language: Some(settings.language),
                data_dir: self.data_dir.clone(),
                timing: settings.transcribe_timing,
            },
        )?;
        *slot = Some(session);
        // A recording gets the CPU: the after job stops after its chunk and continues later.
        self.after.pause_running();
        meetings::set_timing(&self.conn(), id, settings.transcribe_timing)?;
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

    /// Stops the recording. In after mode, it queues the meeting and returns at once; the after
    /// job writes the notes when the transcript is complete.
    pub async fn stop_meeting(&self) -> Result<TranscribeTiming> {
        let session = self
            .session
            .lock()
            .await
            .take()
            .ok_or(MorselError::NotRecording)?;
        let (id, timing) = (session.id(), session.timing());
        let stopped = session.stop().await;
        if timing == TranscribeTiming::After {
            self.set_status(id, MeetingStatus::Transcribing)?;
            self.after.push(id);
        }
        self.after.wake.notify_one();
        stopped.map(|()| timing)
    }

    fn set_status(&self, id: MeetingId, status: MeetingStatus) -> Result<()> {
        meetings::set_status(&self.conn(), id, status)?;
        self.sink.emit(
            STATUS,
            json!({ "meetingId": id, "status": status, "warning": null }),
        );
        Ok(())
    }

    /// The size in bytes of the saved audio of a meeting, or `None` when it has none.
    pub fn audio_size(&self, meeting_id: MeetingId) -> Option<u64> {
        let dir = audio_dir(&self.data_dir, meeting_id);
        dir.exists().then(|| folder_size(&dir))
    }

    /// Deletes the saved audio of a meeting. The transcript and the notes stay. Morsel refuses
    /// while the meeting records, or while the after job needs the audio.
    pub async fn delete_audio(&self, meeting_id: MeetingId) -> Result<()> {
        if let Some(session) = self.session.lock().await.as_ref()
            && session.id() == meeting_id
        {
            return Err(MorselError::AlreadyRecording);
        }
        if self.get_meeting(meeting_id)?.summary.status == MeetingStatus::Transcribing {
            return Err(MorselError::Invalid(
                "Morsel transcribes this audio now. Delete it when the notes are ready.".into(),
            ));
        }
        remove_folder(&audio_dir(&self.data_dir, meeting_id))
    }

    /// The meetings that wait for the after job, oldest first.
    pub fn queued_after_jobs(&self) -> Vec<MeetingId> {
        self.after.waiting()
    }

    /// At app start: queues the meetings that were transcribing, and the after-mode meetings that
    /// a crash interrupted while they recorded, when their audio exists.
    pub fn queue_unfinished_after_jobs(&self) -> Result<()> {
        let (transcribing, interrupted) = {
            let conn = self.conn();
            (
                meetings::list_by_status(&conn, MeetingStatus::Transcribing)?,
                meetings::list_by_status(&conn, MeetingStatus::Interrupted)?,
            )
        };
        for id in transcribing {
            self.after.push(id);
        }
        for id in interrupted {
            let after = meetings::timing_of(&self.conn(), id)? == TranscribeTiming::After;
            if after && audio_dir(&self.data_dir, id).exists() {
                self.set_status(id, MeetingStatus::Transcribing)?;
                self.after.push(id);
            }
        }
        Ok(())
    }

    /// The after-mode worker: one meeting at a time, and only while no meeting records.
    /// It never returns. Start it one time, when the app starts.
    pub async fn run_after_jobs(&self) {
        loop {
            let token = CancellationToken::new();
            let id = loop {
                {
                    // Hold the session lock while the job is marked as running: a meeting that
                    // starts right after this then finds the job and pauses it.
                    let session = self.session.lock().await;
                    if session.is_none()
                        && let Some(id) = self.after.pop()
                    {
                        self.after.set_running(id, token.clone());
                        break id;
                    }
                }
                self.after.wake.notified().await;
            };
            let result = self.transcribe_after(id, &token).await;
            self.after.clear_running();
            if token.is_cancelled() {
                // A new recording paused the job, or the meeting was deleted.
                if self.get_meeting(id).is_ok() {
                    self.after.push_front(id);
                }
                continue;
            }
            let notes = result.and_then(|()| {
                let template_id = self.get_meeting(id)?.summary.template_id;
                self.write_notes(id, template_id)
            });
            if let Err(err) = notes {
                self.fail_after_job(id, &err);
            }
        }
    }

    async fn transcribe_after(&self, id: MeetingId, token: &CancellationToken) -> Result<()> {
        let settings = self.settings()?;
        let transcriber = self.engines.transcriber(&settings)?;
        transcribe_meeting(
            &self.db,
            transcriber.as_ref(),
            &self.sink,
            &audio_dir(&self.data_dir, id),
            id,
            Some(&settings.language),
            token,
        )
        .await
    }

    fn fail_after_job(&self, id: MeetingId, err: &MorselError) {
        eprintln!("the after job of meeting {id} failed: {err}");
        if let Err(status_err) = self.set_status(id, MeetingStatus::NotesFailed) {
            eprintln!("cannot set the meeting status: {status_err}");
        }
        self.sink.emit(
            NOTES_FAILED,
            json!({ "meetingId": id, "message": err.to_string() }),
        );
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
        let (notes, quick) = self.chat_models(Purpose::Notes)?;
        let ctx = NotesContext {
            db: self.db.clone(),
            sink: self.sink.clone(),
            notes,
            quick,
            embed: self.search_embed(),
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
        self.after.remove(meeting_id);
        library::delete_meeting(&self.conn(), meeting_id)?;
        remove_folder(&audio_dir(&self.data_dir, meeting_id))
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
        let (_, quick) = self.chat_models(Purpose::Ask)?;
        ask::ask(
            &self.db,
            quick.as_ref(),
            self.search_embed(),
            question,
            range,
            meeting_id,
        )
        .await
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
        let mut bytes = folder_size(&self.data_dir.join("audio"));
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
        for id in self.after.waiting() {
            self.after.remove(id);
        }
        self.after.pause_running();
        remove_folder(&self.data_dir.join("audio"))?;
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

    pub fn model_ready(&self) -> bool {
        model_ready(&model_dir(&self.models_root))
    }

    pub async fn download_model(&self) -> Result<()> {
        let sink = self.sink.clone();
        ensure_model(
            &self.http,
            &model_dir(&self.models_root),
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
pub async fn stop_meeting(state: State<'_, AppState>) -> Result<TranscribeTiming> {
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
    state.shown_settings()
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
pub async fn model_status(state: State<'_, AppState>) -> Result<bool> {
    Ok(state.model_ready())
}

#[tauri::command]
pub async fn download_model(state: State<'_, AppState>) -> Result<()> {
    state.download_model().await
}

#[tauri::command]
pub async fn notes_model_status(state: State<'_, AppState>) -> Result<bool> {
    Ok(state.notes_model_ready())
}

#[tauri::command]
pub async fn download_notes_model(state: State<'_, AppState>) -> Result<()> {
    state.download_notes_model().await
}

#[tauri::command]
pub async fn search_model_status(state: State<'_, AppState>) -> Result<bool> {
    Ok(state.search_model_ready())
}

#[tauri::command]
pub async fn download_search_model(state: State<'_, AppState>) -> Result<()> {
    state.download_search_model().await
}

#[tauri::command]
pub async fn audio_size(state: State<'_, AppState>, meeting_id: MeetingId) -> Result<Option<u64>> {
    Ok(state.audio_size(meeting_id))
}

#[tauri::command]
pub async fn delete_audio(state: State<'_, AppState>, meeting_id: MeetingId) -> Result<()> {
    state.delete_audio(meeting_id).await
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
    use crate::settings::TranscribeTiming;
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

    fn test_search_model() -> Arc<ModelManager<LocalEmbedder>> {
        Arc::new(ModelManager::new(
            Box::new(|p| LocalEmbedder::load(p).map(Arc::new)),
            crate::model_manager::EMBED_IDLE,
        ))
    }

    /// Writes an empty file with the size of the search model, so it counts as downloaded.
    fn fake_search_model(state: &AppState) {
        let dir = search_model_dir(&state.models_root);
        std::fs::create_dir_all(&dir).unwrap();
        let file = &search_model_files()[0];
        std::fs::File::create(dir.join(&file.name))
            .unwrap()
            .set_len(file.size)
            .unwrap();
    }

    #[test]
    fn search_model_status_is_false_without_the_file() {
        let (_d, state) = state();
        assert!(!state.search_model_ready());
        assert!(state.search_embed().is_none());
    }

    #[test]
    fn search_model_status_is_true_with_the_file() {
        let (_d, state) = state();
        fake_search_model(&state);
        assert!(state.search_model_ready());
        assert!(state.search_embed().is_some());
    }

    #[test]
    fn indexing_at_start_up_indexes_old_meetings() {
        let (_d, state, sink) = state_with_sink();
        {
            let conn = state.db.lock().unwrap();
            let id = crate::db::meetings::create_meeting(&conn, "Old", 1).unwrap();
            crate::db::meetings::save_note(&conn, id, "budget ok", "m").unwrap();
            crate::db::meetings::set_status(&conn, id, MeetingStatus::Done).unwrap();
        }
        state.start_indexing().join().unwrap();
        assert_eq!(
            sink.named(crate::events::SEARCH_PROGRESS).last().unwrap()["done"],
            1
        );
    }

    fn test_notes_model() -> Arc<ModelManager<LocalLlm>> {
        Arc::new(ModelManager::new(
            Box::new(|p| LocalLlm::load(p).map(Arc::new)),
            crate::model_manager::IDLE,
        ))
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
            LocalModels {
                root: dir.path().join("models"),
                notes_model: test_notes_model(),
                search_model: test_search_model(),
            },
        );
        (dir, state)
    }

    fn state_with_sink() -> (tempfile::TempDir, Arc<AppState>, Arc<VecSink>) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::open(&dir.path().join("m.db")).unwrap();
        let sink = Arc::new(VecSink::default());
        let state = AppState::new(
            Arc::new(std::sync::Mutex::new(db)),
            Arc::new(MemoryStore::default()),
            reqwest::Client::new(),
            sink.clone(),
            Arc::new(FakeEngines),
            dir.path().to_path_buf(),
            LocalModels {
                root: dir.path().join("models"),
                notes_model: test_notes_model(),
                search_model: test_search_model(),
            },
        );
        (dir, Arc::new(state), sink)
    }

    fn use_after_mode(s: &AppState) {
        let mut settings = s.settings().unwrap();
        settings.transcribe_timing = TranscribeTiming::After;
        // These tests check the after job. Cloud mode with no key makes its notes step fail fast.
        settings.notes_mode = Some(NotesMode::Cloud);
        s.save_settings(&settings).unwrap();
    }

    fn spawn_worker(s: &Arc<AppState>) {
        let worker = s.clone();
        tokio::spawn(async move { worker.run_after_jobs().await });
    }

    async fn wait_for_status(s: &AppState, id: MeetingId, status: MeetingStatus) {
        for _ in 0..200 {
            if s.get_meeting(id).unwrap().summary.status == status {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!(
            "meeting {id} is {:?}, not {status:?}",
            s.get_meeting(id).unwrap().summary.status
        );
    }

    fn failure_of(sink: &VecSink, id: MeetingId) -> String {
        sink.named(crate::events::NOTES_FAILED)
            .iter()
            .find(|e| e["meetingId"] == id)
            .map(|e| e["message"].as_str().unwrap_or_default().to_owned())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn after_stop_returns_at_once_and_writes_notes_later() {
        let (_d, s, sink) = state_with_sink();
        use_after_mode(&s);
        let id = s.start_meeting(None, 1).await.unwrap();
        assert_eq!(s.stop_meeting().await.unwrap(), TranscribeTiming::After);
        assert_eq!(
            s.get_meeting(id).unwrap().summary.status,
            MeetingStatus::Transcribing
        );
        spawn_worker(&s);
        // No notes key is set, so the notes step fails: the job got to it.
        wait_for_status(&s, id, MeetingStatus::NotesFailed).await;
        assert!(
            failure_of(&sink, id).contains("notes API key"),
            "{}",
            failure_of(&sink, id)
        );
    }

    #[tokio::test]
    async fn stop_twice_queues_the_meeting_once() {
        let (_d, s, _sink) = state_with_sink();
        use_after_mode(&s);
        let id = s.start_meeting(None, 1).await.unwrap();
        s.stop_meeting().await.unwrap();
        assert!(matches!(
            s.stop_meeting().await,
            Err(MorselError::NotRecording)
        ));
        assert_eq!(s.queued_after_jobs(), [id]);
    }

    #[tokio::test]
    async fn after_job_waits_while_a_meeting_records() {
        let (_d, s, _sink) = state_with_sink();
        use_after_mode(&s);
        let first = s.start_meeting(None, 1).await.unwrap();
        s.stop_meeting().await.unwrap();
        let second = s.start_meeting(None, 1).await.unwrap();
        spawn_worker(&s);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            s.get_meeting(first).unwrap().summary.status,
            MeetingStatus::Transcribing
        );
        s.stop_meeting().await.unwrap();
        wait_for_status(&s, first, MeetingStatus::NotesFailed).await;
        wait_for_status(&s, second, MeetingStatus::NotesFailed).await;
    }

    #[tokio::test]
    async fn missing_audio_fails_only_that_meeting() {
        let (d, s, sink) = state_with_sink();
        use_after_mode(&s);
        let first = s.start_meeting(None, 1).await.unwrap();
        s.stop_meeting().await.unwrap();
        let second = s.start_meeting(None, 1).await.unwrap();
        s.stop_meeting().await.unwrap();
        std::fs::remove_dir_all(crate::audio::recording::audio_dir(d.path(), first)).unwrap();
        spawn_worker(&s);
        wait_for_status(&s, first, MeetingStatus::NotesFailed).await;
        wait_for_status(&s, second, MeetingStatus::NotesFailed).await;
        assert!(
            failure_of(&sink, first).contains("no saved audio"),
            "{}",
            failure_of(&sink, first)
        );
        assert!(
            failure_of(&sink, second).contains("notes API key"),
            "{}",
            failure_of(&sink, second)
        );
    }

    #[tokio::test]
    async fn start_queues_transcribing_and_interrupted_after_meetings() {
        let (d, s, _sink) = state_with_sink();
        let conn_meeting = |timing: TranscribeTiming, status: MeetingStatus, audio: bool| {
            let conn = s.db.lock().unwrap();
            let id = meetings::create_meeting(&conn, "M", 1).unwrap();
            meetings::set_timing(&conn, id, timing).unwrap();
            meetings::set_status(&conn, id, status).unwrap();
            if audio {
                std::fs::create_dir_all(crate::audio::recording::audio_dir(d.path(), id)).unwrap();
            }
            id
        };
        let transcribing = conn_meeting(TranscribeTiming::After, MeetingStatus::Transcribing, true);
        let crashed = conn_meeting(TranscribeTiming::After, MeetingStatus::Interrupted, true);
        conn_meeting(TranscribeTiming::Live, MeetingStatus::Interrupted, true);
        conn_meeting(TranscribeTiming::After, MeetingStatus::Interrupted, false);
        s.queue_unfinished_after_jobs().unwrap();
        assert_eq!(s.queued_after_jobs(), [transcribing, crashed]);
        assert_eq!(
            s.get_meeting(crashed).unwrap().summary.status,
            MeetingStatus::Transcribing
        );
    }

    #[tokio::test]
    async fn delete_meeting_cancels_its_job() {
        let (_d, s, _sink) = state_with_sink();
        use_after_mode(&s);
        let id = s.start_meeting(None, 1).await.unwrap();
        s.stop_meeting().await.unwrap();
        s.delete_meeting(id).await.unwrap();
        assert!(s.queued_after_jobs().is_empty());
    }

    fn saved_audio(d: &tempfile::TempDir, id: MeetingId) -> std::path::PathBuf {
        crate::audio::recording::audio_dir(d.path(), id)
    }

    #[tokio::test]
    async fn delete_audio_keeps_the_transcript() {
        let (d, s, _sink) = state_with_sink();
        let id = s.start_meeting(None, 1).await.unwrap();
        s.stop_meeting().await.unwrap();
        meetings::add_line(
            &s.conn(),
            &crate::types::TranscriptLine {
                meeting_id: id,
                speaker: crate::types::Speaker::You,
                start_ms: 0,
                end_ms: 1_000,
                text: "hello".into(),
            },
        )
        .unwrap();
        assert!(s.audio_size(id).is_some());
        s.delete_audio(id).await.unwrap();
        assert!(!saved_audio(&d, id).exists());
        assert_eq!(s.audio_size(id), None);
        assert_eq!(s.get_meeting(id).unwrap().lines.len(), 1);
    }

    #[tokio::test]
    async fn delete_audio_refuses_while_recording_or_transcribing() {
        let (d, s, _sink) = state_with_sink();
        let id = s.start_meeting(None, 1).await.unwrap();
        assert!(s.delete_audio(id).await.is_err());
        use_after_mode(&s);
        s.stop_meeting().await.unwrap();
        let after = s.start_meeting(None, 1).await.unwrap();
        s.stop_meeting().await.unwrap();
        assert!(s.delete_audio(after).await.is_err());
        assert!(saved_audio(&d, after).exists());
    }

    #[tokio::test]
    async fn delete_meeting_deletes_its_audio() {
        let (d, s, _sink) = state_with_sink();
        let id = s.start_meeting(None, 1).await.unwrap();
        s.stop_meeting().await.unwrap();
        assert!(saved_audio(&d, id).exists());
        s.delete_meeting(id).await.unwrap();
        assert!(!saved_audio(&d, id).exists());
    }

    #[tokio::test]
    async fn delete_all_data_deletes_the_audio() {
        let (d, s, _sink) = state_with_sink();
        let id = s.start_meeting(None, 1).await.unwrap();
        s.stop_meeting().await.unwrap();
        s.delete_all_data().await.unwrap();
        assert!(!saved_audio(&d, id).exists());
    }

    #[test]
    fn data_info_counts_the_audio() {
        let (d, s, _sink) = state_with_sink();
        let before = s.data_info().unwrap().bytes;
        let dir = saved_audio(&d, 7);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("you.wav"), vec![0u8; 5_000]).unwrap();
        assert_eq!(s.data_info().unwrap().bytes, before + 5_000);
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

    #[test]
    fn new_install_uses_local() {
        let (_d, s) = state();
        assert_eq!(s.notes_mode().unwrap(), NotesMode::Local);
    }

    #[test]
    fn install_with_a_notes_key_keeps_cloud() {
        let (_d, s) = state();
        s.set_api_key(NOTES_KEY, "sk-1").unwrap();
        assert_eq!(s.notes_mode().unwrap(), NotesMode::Cloud);
    }

    #[test]
    fn shown_settings_hold_the_effective_mode() {
        let (_d, s) = state();
        assert_eq!(
            s.shown_settings().unwrap().notes_mode,
            Some(NotesMode::Local)
        );
        s.set_api_key(NOTES_KEY, "sk-1").unwrap();
        assert_eq!(
            s.shown_settings().unwrap().notes_mode,
            Some(NotesMode::Cloud)
        );
    }

    #[test]
    fn saved_mode_wins() {
        let (_d, s) = state();
        s.set_api_key(NOTES_KEY, "sk-1").unwrap();
        let mut settings = s.settings().unwrap();
        settings.notes_mode = Some(NotesMode::Local);
        s.save_settings(&settings).unwrap();
        assert_eq!(s.notes_mode().unwrap(), NotesMode::Local);
    }

    #[tokio::test]
    async fn local_write_notes_without_the_model_fails_with_help() {
        let (_d, s) = state();
        let id = s.start_meeting(Some("Sync".into()), 1).await.unwrap();
        s.stop_meeting().await.unwrap();
        let err = s.write_notes(id, 1).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid value: download the notes model in Settings first"
        );
    }

    #[tokio::test]
    async fn write_notes_needs_a_notes_key() {
        let (_d, s) = state();
        let mut settings = s.settings().unwrap();
        settings.notes_mode = Some(NotesMode::Cloud);
        s.save_settings(&settings).unwrap();
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

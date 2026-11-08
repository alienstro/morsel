//! The commands that the frontend calls. Each command is a thin call into an `AppState` method.

use crate::audio::{mic::MicSource, source::AudioSource, system_tap::SystemTapSource};
use crate::db::SharedDb;
use crate::db::meetings::{self, MeetingDetail, MeetingSummary, Template};
use crate::events::{EventSink, MODEL_DOWNLOAD};
use crate::llm::{self, LlmConfig};
use crate::notes::writer::{CancellationToken, NotesContext};
use crate::secrets::{NOTES_KEY, SecretStore, TRANSCRIBE_KEY, is_known_account};
use crate::session::MeetingSession;
use crate::settings::{self, Settings, TranscribeMode};
use crate::transcribe::Transcriber;
use crate::transcribe::cloud::CloudTranscriber;
use crate::transcribe::model_download::{ensure_model, model_ready};
use crate::transcribe::parakeet::ParakeetTranscriber;
use crate::types::{MeetingId, MorselError, Result};
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
pub struct RealEngines {
    model_dir: PathBuf,
    http: reqwest::Client,
    secrets: Arc<dyn SecretStore>,
    parakeet: Mutex<Option<Arc<ParakeetTranscriber>>>,
}

impl RealEngines {
    pub fn new(model_dir: PathBuf, http: reqwest::Client, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            model_dir,
            http,
            secrets,
            parakeet: Mutex::new(None),
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
                let mut cached = self.parakeet.lock().expect("parakeet lock");
                if cached.is_none() {
                    if !model_ready(&self.model_dir) {
                        return Err(MorselError::Invalid(
                            "download the transcription model in Settings first".into(),
                        ));
                    }
                    *cached = Some(Arc::new(ParakeetTranscriber::new(&self.model_dir)?));
                }
                Ok(cached.clone().expect("loaded") as Arc<dyn Transcriber>)
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

pub struct AppState {
    pub db: SharedDb,
    pub secrets: Arc<dyn SecretStore>,
    pub http: reqwest::Client,
    pub sink: Arc<dyn EventSink>,
    engines: Arc<dyn Engines>,
    model_dir: PathBuf,
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
        model_dir: PathBuf,
    ) -> Self {
        Self {
            db,
            secrets,
            http,
            sink,
            engines,
            model_dir,
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
        Ok(id)
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
        meetings::list_templates(&self.conn())
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
        let quick_model = if settings.quick_model.trim().is_empty() {
            &settings.notes_model
        } else {
            &settings.quick_model
        };
        let quick = LlmConfig {
            model: quick_model.clone(),
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

    pub async fn test_notes_key(&self) -> Result<()> {
        let model = self.settings()?.notes_model;
        llm::test_key(&self.http, &self.notes_config(&model)?).await
    }

    pub fn model_ready(&self) -> bool {
        model_ready(&self.model_dir)
    }

    pub async fn download_model(&self) -> Result<()> {
        let sink = self.sink.clone();
        ensure_model(&self.http, &self.model_dir, move |received, total| {
            sink.emit(
                MODEL_DOWNLOAD,
                json!({ "received": received, "total": total }),
            );
        })
        .await
        .map(|_| ())
    }
}

#[tauri::command]
pub async fn start_meeting(
    state: State<'_, AppState>,
    title: Option<String>,
    template_id: i64,
) -> Result<MeetingId> {
    state.start_meeting(title, template_id).await
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

    #[tokio::test]
    async fn pause_without_meeting_returns_not_recording() {
        let (_d, s) = state();
        assert!(matches!(
            s.pause_meeting().await,
            Err(MorselError::NotRecording)
        ));
    }
}

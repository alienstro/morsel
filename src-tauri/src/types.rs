//! Types that the backend modules share.

use serde::{Deserialize, Serialize, Serializer};

pub type MeetingId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    You,
    Others,
}

impl Speaker {
    pub fn as_str(self) -> &'static str {
        match self {
            Speaker::You => "you",
            Speaker::Others => "others",
        }
    }

    pub fn parse(value: &str) -> Result<Self, MorselError> {
        match value {
            "you" => Ok(Speaker::You),
            "others" => Ok(Speaker::Others),
            other => Err(MorselError::Invalid(format!("unknown speaker {other}"))),
        }
    }

    /// The label that the transcript and the prompt show.
    pub fn label(self) -> &'static str {
        match self {
            Speaker::You => "You",
            Speaker::Others => "Others",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStatus {
    Recording,
    Paused,
    Stopped,
    WritingNotes,
    Done,
    NotesCancelled,
    NotesFailed,
    Interrupted,
}

impl MeetingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MeetingStatus::Recording => "recording",
            MeetingStatus::Paused => "paused",
            MeetingStatus::Stopped => "stopped",
            MeetingStatus::WritingNotes => "writing_notes",
            MeetingStatus::Done => "done",
            MeetingStatus::NotesCancelled => "notes_cancelled",
            MeetingStatus::NotesFailed => "notes_failed",
            MeetingStatus::Interrupted => "interrupted",
        }
    }

    pub fn parse(value: &str) -> Result<Self, MorselError> {
        serde_json::from_value(serde_json::Value::String(value.to_owned()))
            .map_err(|_| MorselError::Invalid(format!("unknown status {value}")))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptLine {
    pub meeting_id: MeetingId,
    pub speaker: Speaker,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// 16 kHz mono audio of one speaker. It stays in memory only.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    pub speaker: Speaker,
    pub start_ms: u64,
    pub samples: Vec<f32>,
}

#[derive(Debug, thiserror::Error)]
pub enum MorselError {
    #[error("database error: {0}")]
    Db(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid value: {0}")]
    Invalid(String),
    #[error("the API key is not valid")]
    InvalidKey,
    /// Composio has several kinds of key, and only a project key works for Morsel.
    #[error(
        "Composio did not accept this key. Morsel needs a project API key: on composio.dev, open Settings, then Project Settings, and copy a key from API Keys."
    )]
    InvalidComposioKey,
    #[error("the provider rate limit is reached")]
    RateLimit,
    #[error("network or provider error: {0}")]
    Http(String),
    #[error("Morsel has no microphone permission")]
    MicPermissionDenied,
    #[error("Morsel has no system audio permission")]
    SystemAudioPermissionDenied,
    #[error("system audio needs macOS 14.2 or later")]
    SystemAudioUnsupported,
    #[error("audio error: {0}")]
    Audio(String),
    #[error("transcription error: {0}")]
    Transcribe(String),
    #[error("the downloaded model is damaged")]
    ModelChecksum,
    #[error("a meeting is already recording")]
    AlreadyRecording,
    #[error("no meeting is recording")]
    NotRecording,
    #[error("the request was cancelled")]
    Cancelled,
    #[error("Keychain error: {0}")]
    Keychain(String),
    #[error("file error: {0}")]
    Io(String),
}

impl MorselError {
    /// A stable code that the frontend can test.
    pub fn code(&self) -> &'static str {
        match self {
            MorselError::Db(_) => "db",
            MorselError::NotFound(_) => "not_found",
            MorselError::Invalid(_) => "invalid",
            MorselError::InvalidKey | MorselError::InvalidComposioKey => "invalid_key",
            MorselError::RateLimit => "rate_limit",
            MorselError::Http(_) => "http",
            MorselError::MicPermissionDenied => "mic_permission_denied",
            MorselError::SystemAudioPermissionDenied => "system_audio_permission_denied",
            MorselError::SystemAudioUnsupported => "system_audio_unsupported",
            MorselError::Audio(_) => "audio",
            MorselError::Transcribe(_) => "transcribe",
            MorselError::ModelChecksum => "model_checksum",
            MorselError::AlreadyRecording => "already_recording",
            MorselError::NotRecording => "not_recording",
            MorselError::Cancelled => "cancelled",
            MorselError::Keychain(_) => "keychain",
            MorselError::Io(_) => "io",
        }
    }
}

impl Serialize for MorselError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("MorselError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl From<rusqlite::Error> for MorselError {
    fn from(err: rusqlite::Error) -> Self {
        match err {
            rusqlite::Error::QueryReturnedNoRows => MorselError::NotFound("row".into()),
            other => MorselError::Db(other.to_string()),
        }
    }
}

impl From<std::io::Error> for MorselError {
    fn from(err: std::io::Error) -> Self {
        MorselError::Io(err.to_string())
    }
}

pub type Result<T, E = MorselError> = std::result::Result<T, E>;

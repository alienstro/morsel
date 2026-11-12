//! After mode: transcribes the saved audio of a meeting after it stops.

use crate::audio::PIPELINE_RATE;
use crate::audio::recording::StreamReader;
use crate::chunker::Chunker;
use crate::db::SharedDb;
use crate::db::meetings::{add_line_at, last_line_end, set_transcribed_ms};
use crate::events::{EventSink, TRANSCRIBE_PROGRESS, TRANSCRIPT};
use crate::notes::writer::CancellationToken;
use crate::session::{audio_file, clock};
use crate::transcribe::{Transcriber, transcribe_with_retry};
use crate::types::{AudioChunk, MeetingId, MorselError, Result, Speaker, TranscriptLine};
use serde_json::json;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// The job reads one second of audio at a time, so a long meeting never fills the memory.
const PIECE: usize = PIPELINE_RATE as usize;

/// The saved audio of one speaker, cut into chunks with the same rules as live mode.
struct Stream {
    speaker: Speaker,
    reader: StreamReader,
    chunker: Chunker,
    /// The time where the reader started. Chunk times are relative to it.
    offset_ms: u64,
    pending: VecDeque<AudioChunk>,
    ended: bool,
}

impl Stream {
    /// Opens the file of `speaker` at the end of its last saved line. `None` when there is no file.
    fn open(audio: &Path, speaker: Speaker, from_ms: u64) -> Result<Option<Self>> {
        let path = audio.join(audio_file(speaker));
        if !path.exists() {
            return Ok(None);
        }
        let mut reader = StreamReader::open(&path)?;
        reader.seek_ms(from_ms)?;
        Ok(Some(Self {
            speaker,
            reader,
            chunker: Chunker::new(speaker),
            offset_ms: from_ms,
            pending: VecDeque::new(),
            ended: false,
        }))
    }

    /// Reads until the next chunk is known, or until the end of the file.
    fn fill(&mut self) -> Result<()> {
        while self.pending.is_empty() && !self.ended {
            let piece = self.reader.read(PIECE)?;
            if piece.is_empty() {
                self.ended = true;
                self.pending.extend(self.chunker.flush());
            } else {
                self.pending.extend(self.chunker.push(&piece));
            }
        }
        Ok(())
    }

    fn next_start(&self) -> Option<u64> {
        self.pending.front().map(|c| c.start_ms + self.offset_ms)
    }
}

/// Transcribes `you.wav` and `others.wav` in time order, one chunk at a time. Each line is saved
/// at once, so a new run continues after the last saved line of each speaker. Returns early, with
/// no error, when `cancel` fires.
pub async fn transcribe_meeting(
    db: &SharedDb,
    transcriber: &dyn Transcriber,
    sink: &Arc<dyn EventSink>,
    audio: &Path,
    id: MeetingId,
    language: Option<&str>,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut streams = Vec::new();
    let mut total_ms = 0;
    for speaker in [Speaker::You, Speaker::Others] {
        let from = last_line_end(&db.lock().expect("db lock"), id, speaker)?;
        if let Some(stream) = Stream::open(audio, speaker, from)? {
            total_ms = total_ms.max(stream.reader.duration_ms());
            streams.push(stream);
        }
    }
    if streams.is_empty() {
        return Err(MorselError::Audio(format!(
            "meeting {id} has no saved audio"
        )));
    }
    let mut done_ms = 0;
    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }
        for stream in &mut streams {
            stream.fill()?;
        }
        let Some(next) = streams
            .iter_mut()
            .filter(|s| s.next_start().is_some())
            .min_by_key(|s| s.next_start())
        else {
            break;
        };
        let start_ms = next.next_start().expect("the stream has a chunk");
        let chunk = next.pending.pop_front().expect("the stream has a chunk");
        let speaker = next.speaker;
        let end_ms = start_ms + chunk.samples.len() as u64 * 1_000 / PIPELINE_RATE as u64;
        done_ms = done_ms.max(end_ms.min(total_ms));
        let text = match transcribe_with_retry(transcriber, &chunk.samples, language).await {
            Some(text) if text.trim().is_empty() => None,
            Some(text) => Some(text),
            None => Some(format!("[missing {}–{}]", clock(start_ms), clock(end_ms))),
        };
        if let Some(text) = text {
            let line = TranscriptLine {
                meeting_id: id,
                speaker,
                start_ms,
                end_ms,
                text,
            };
            add_line_at(&db.lock().expect("db lock"), &line, done_ms)?;
            sink.emit(TRANSCRIPT, serde_json::to_value(&line).unwrap_or_default());
        } else {
            set_transcribed_ms(&db.lock().expect("db lock"), id, done_ms)?;
        }
        emit_progress(sink, id, done_ms, total_ms);
    }
    set_transcribed_ms(&db.lock().expect("db lock"), id, total_ms)?;
    emit_progress(sink, id, total_ms, total_ms);
    Ok(())
}

fn emit_progress(sink: &Arc<dyn EventSink>, id: MeetingId, done_ms: u64, total_ms: u64) {
    sink.emit(
        TRANSCRIBE_PROGRESS,
        json!({ "meetingId": id, "doneMs": done_ms, "totalMs": total_ms }),
    );
}

/// The meetings that wait for the after job, oldest first, and the one that runs now.
#[derive(Default)]
pub struct AfterQueue {
    waiting: Mutex<VecDeque<MeetingId>>,
    running: Mutex<Option<(MeetingId, CancellationToken)>>,
    /// Wakes the worker when a meeting is queued or a recording stops.
    pub wake: Notify,
}

impl AfterQueue {
    /// Adds a meeting at the end. Returns false when it is already queued or running.
    pub fn push(&self, id: MeetingId) -> bool {
        let mut waiting = self.waiting.lock().expect("after queue lock");
        if waiting.contains(&id) || self.running_id() == Some(id) {
            return false;
        }
        waiting.push_back(id);
        self.wake.notify_one();
        true
    }

    /// Puts a paused meeting back at the front, so that it continues first.
    pub fn push_front(&self, id: MeetingId) {
        self.waiting
            .lock()
            .expect("after queue lock")
            .push_front(id);
    }

    pub fn pop(&self) -> Option<MeetingId> {
        self.waiting.lock().expect("after queue lock").pop_front()
    }

    pub fn waiting(&self) -> Vec<MeetingId> {
        self.waiting
            .lock()
            .expect("after queue lock")
            .iter()
            .copied()
            .collect()
    }

    pub fn set_running(&self, id: MeetingId, token: CancellationToken) {
        *self.running.lock().expect("after queue lock") = Some((id, token));
    }

    pub fn clear_running(&self) {
        *self.running.lock().expect("after queue lock") = None;
    }

    fn running_id(&self) -> Option<MeetingId> {
        self.running
            .lock()
            .expect("after queue lock")
            .as_ref()
            .map(|(id, _)| *id)
    }

    /// Stops the running job after its current chunk. The worker queues it again at the front.
    pub fn pause_running(&self) {
        if let Some((_, token)) = self.running.lock().expect("after queue lock").as_ref() {
            token.cancel();
        }
    }

    /// Takes a meeting out of the queue, and stops its job when it runs.
    pub fn remove(&self, id: MeetingId) {
        self.waiting
            .lock()
            .expect("after queue lock")
            .retain(|&m| m != id);
        if let Some((running, token)) = self.running.lock().expect("after queue lock").as_ref()
            && *running == id
        {
            token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::recording::StreamWriter;
    use crate::db::meetings::{add_line, create_meeting, get_meeting};
    use crate::events::{TRANSCRIBE_PROGRESS, VecSink};
    use crate::types::{Speaker, TranscriptLine};

    /// Records the length of each call.
    #[derive(Default)]
    struct Recording {
        calls: Mutex<Vec<usize>>,
    }

    #[async_trait::async_trait]
    impl Transcriber for Recording {
        async fn transcribe(&self, samples: &[f32], _l: Option<&str>) -> Result<String> {
            self.calls.lock().unwrap().push(samples.len());
            Ok("words".into())
        }
    }

    fn speech(seconds: f32) -> Vec<f32> {
        (0..(seconds * 16_000.0) as usize)
            .map(|i| (i as f32 * 0.07).sin() * 0.3)
            .collect()
    }

    fn silence(seconds: f32) -> Vec<f32> {
        vec![0.0; (seconds * 16_000.0) as usize]
    }

    fn wav(path: &Path, parts: &[Vec<f32>]) {
        let mut w = StreamWriter::create(path).unwrap();
        for part in parts {
            w.write(part).unwrap();
        }
        w.finish().unwrap();
    }

    fn setup() -> (tempfile::TempDir, SharedDb, MeetingId) {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&dir.path().join("m.db")).unwrap();
        let id = create_meeting(&conn, "Sync", 1).unwrap();
        (dir, Arc::new(Mutex::new(conn)), id)
    }

    /// Speaker and start of each line, in the order of insertion.
    fn inserted(db: &SharedDb, id: MeetingId) -> Vec<(String, i64)> {
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT speaker, start_ms FROM transcript_lines WHERE meeting_id = ?1 ORDER BY id",
            )
            .unwrap();
        stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    async fn run(
        db: &SharedDb,
        t: &Recording,
        sink: &Arc<dyn EventSink>,
        audio: &Path,
        id: MeetingId,
    ) -> Result<()> {
        transcribe_meeting(db, t, sink, audio, id, None, &CancellationToken::new()).await
    }

    #[tokio::test]
    async fn after_job_transcribes_in_time_order() {
        let (d, db, id) = setup();
        let audio = d.path().join("audio");
        wav(
            &audio.join("you.wav"),
            &[speech(12.0), silence(20.0), speech(12.0)],
        );
        wav(&audio.join("others.wav"), &[silence(15.0), speech(12.0)]);
        let t = Recording::default();
        let vec_sink = Arc::new(VecSink::default());
        let sink: Arc<dyn EventSink> = vec_sink.clone();
        run(&db, &t, &sink, &audio, id).await.unwrap();
        let order: Vec<(String, i64)> = inserted(&db, id);
        let speakers: Vec<&str> = order.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(speakers, ["you", "others", "you"], "{order:?}");
        assert!(order.windows(2).all(|w| w[0].1 <= w[1].1), "{order:?}");
        let progress = vec_sink.named(TRANSCRIBE_PROGRESS);
        let last = progress.last().expect("progress events");
        assert_eq!(last["doneMs"], last["totalMs"]);
        assert_eq!(last["totalMs"], 44_000);
        assert_eq!(
            get_meeting(&db.lock().unwrap(), id)
                .unwrap()
                .summary
                .transcribed_ms,
            44_000
        );
    }

    #[tokio::test]
    async fn after_job_continues_from_its_position() {
        let (d, db, id) = setup();
        let audio = d.path().join("audio");
        wav(
            &audio.join("you.wav"),
            &[speech(12.0), silence(20.0), speech(12.0)],
        );
        // The first chunk was done before a quit.
        add_line(
            &db.lock().unwrap(),
            &TranscriptLine {
                meeting_id: id,
                speaker: Speaker::You,
                start_ms: 0,
                end_ms: 12_420,
                text: "first".into(),
            },
        )
        .unwrap();
        let t = Recording::default();
        let vec_sink = Arc::new(VecSink::default());
        let sink: Arc<dyn EventSink> = vec_sink.clone();
        run(&db, &t, &sink, &audio, id).await.unwrap();
        assert_eq!(t.calls.lock().unwrap().len(), 1);
        assert_eq!(inserted(&db, id).len(), 2);
        let progress = vec_sink.named(TRANSCRIBE_PROGRESS);
        assert_eq!(progress.last().unwrap()["totalMs"], 44_000);
    }

    #[tokio::test]
    async fn resume_does_not_repeat_a_line() {
        let (d, db, id) = setup();
        let audio = d.path().join("audio");
        wav(
            &audio.join("you.wav"),
            &[speech(12.0), silence(1.0), speech(5.0)],
        );
        wav(&audio.join("others.wav"), &[speech(11.0)]);
        let t = Recording::default();
        let sink: Arc<dyn EventSink> = Arc::new(VecSink::default());
        run(&db, &t, &sink, &audio, id).await.unwrap();
        let first = inserted(&db, id);
        run(&db, &t, &sink, &audio, id).await.unwrap();
        assert_eq!(inserted(&db, id), first);
    }

    #[tokio::test]
    async fn after_job_works_with_one_stream() {
        let (d, db, id) = setup();
        let audio = d.path().join("audio");
        wav(&audio.join("you.wav"), &[speech(5.0)]);
        let t = Recording::default();
        let sink: Arc<dyn EventSink> = Arc::new(VecSink::default());
        run(&db, &t, &sink, &audio, id).await.unwrap();
        assert_eq!(inserted(&db, id), [("you".to_owned(), 0)]);
    }

    #[tokio::test]
    async fn missing_audio_is_an_error() {
        let (d, db, id) = setup();
        let t = Recording::default();
        let sink: Arc<dyn EventSink> = Arc::new(VecSink::default());
        assert!(
            run(&db, &t, &sink, &d.path().join("audio"), id)
                .await
                .is_err()
        );
    }
}

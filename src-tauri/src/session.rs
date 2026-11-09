//! One meeting: two audio sources → chunkers → transcriber → SQLite and events.

use crate::audio::recording::{StreamWriter, audio_dir};
use crate::audio::{PIPELINE_RATE, level::rms, source::AudioSource};
use crate::chunker::Chunker;
use crate::db::SharedDb;
use crate::db::meetings::{add_line, create_meeting, mark_open_meetings_interrupted, set_status};
use crate::events::{EventSink, LEVELS, STATUS, TRANSCRIPT};
use crate::settings::TranscribeTiming;
use crate::transcribe::{Transcriber, transcribe_with_retry};
use crate::types::{
    AudioChunk, MeetingId, MeetingStatus, MorselError, Result, Speaker, TranscriptLine,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::sync::mpsc::{Receiver, UnboundedSender, channel, unbounded_channel};
use tokio::task::{JoinHandle, JoinSet};

const CHANNEL_FRAMES: usize = 512;
const LEVEL_INTERVAL: Duration = Duration::from_millis(100);

/// `m:ss` from milliseconds.
pub fn clock(ms: u64) -> String {
    let secs = ms / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Marks the meetings that were open when the app stopped. Call it once at app start.
pub fn recover(db: &SharedDb) -> Result<u32> {
    mark_open_meetings_interrupted(&db.lock().expect("db lock"))
}

/// Everything that a transcription job needs.
#[derive(Clone)]
struct Shared {
    id: MeetingId,
    db: SharedDb,
    transcriber: Arc<dyn Transcriber>,
    sink: Arc<dyn EventSink>,
    language: Option<String>,
    /// One permit: the two streams take turns, so only one transcription uses the CPU at a time.
    gate: Arc<Semaphore>,
}

/// What a new meeting needs besides its audio sources and its transcriber.
pub struct MeetingSetup {
    pub title: String,
    pub template_id: i64,
    pub language: Option<String>,
    /// The app data folder. The audio goes to `audio/<meeting id>/` in it.
    pub data_dir: PathBuf,
    pub timing: TranscribeTiming,
}

pub struct MeetingSession {
    shared: Shared,
    mic: Box<dyn AudioSource>,
    system: Option<Box<dyn AudioSource>>,
    paused: Arc<AtomicBool>,
    pumps: Vec<JoinHandle<()>>,
    levels: JoinHandle<()>,
    timing: TranscribeTiming,
}

impl MeetingSession {
    /// Starts the microphone first. Without it, no meeting is created.
    /// Without system audio, the meeting records the microphone only and the status event has a warning.
    /// Each stream is saved to a WAV file. In `TranscribeTiming::After`, the streams make no chunks.
    pub fn start(
        db: SharedDb,
        mut mic: Box<dyn AudioSource>,
        mut system: Box<dyn AudioSource>,
        transcriber: Arc<dyn Transcriber>,
        sink: Arc<dyn EventSink>,
        setup: MeetingSetup,
    ) -> Result<(Self, MeetingId)> {
        let MeetingSetup {
            title,
            template_id,
            language,
            data_dir,
            timing,
        } = setup;
        let (mic_tx, mic_rx) = channel(CHANNEL_FRAMES);
        mic.start(mic_tx)?;
        let id = match create_meeting(&db.lock().expect("db lock"), &title, template_id) {
            Ok(id) => id,
            Err(err) => {
                mic.stop();
                return Err(err);
            }
        };
        let (sys_tx, sys_rx) = channel(CHANNEL_FRAMES);
        let (system, sys_rx, warning) = match system.start(sys_tx) {
            Ok(()) => (Some(system), Some(sys_rx), None),
            Err(err) => (None, None, Some(format!("Others are not recorded: {err}"))),
        };
        sink.emit(
            STATUS,
            json!({ "meetingId": id, "status": MeetingStatus::Recording, "warning": warning }),
        );

        let shared = Shared {
            id,
            db,
            transcriber,
            sink: sink.clone(),
            language,
            gate: Arc::new(Semaphore::new(1)),
        };
        let paused = Arc::new(AtomicBool::new(false));
        let you_level = Arc::new(AtomicU32::new(0));
        let others_level = Arc::new(AtomicU32::new(0));
        let audio = audio_dir(&data_dir, id);
        let mut pumps = vec![tokio::spawn(pump(
            shared.clone(),
            Speaker::You,
            mic_rx,
            paused.clone(),
            you_level.clone(),
            Recorder::start(&shared, &audio.join(audio_file(Speaker::You))),
            timing,
        ))];
        if let Some(rx) = sys_rx {
            pumps.push(tokio::spawn(pump(
                shared.clone(),
                Speaker::Others,
                rx,
                paused.clone(),
                others_level.clone(),
                Recorder::start(&shared, &audio.join(audio_file(Speaker::Others))),
                timing,
            )));
        }
        let levels = tokio::spawn(async move {
            let mut tick = tokio::time::interval(LEVEL_INTERVAL);
            loop {
                tick.tick().await;
                let you = f32::from_bits(you_level.swap(0, Ordering::Relaxed));
                let others = f32::from_bits(others_level.swap(0, Ordering::Relaxed));
                sink.emit(LEVELS, json!({ "you": you, "others": others }));
            }
        });
        Ok((
            Self {
                shared,
                mic,
                system,
                paused,
                pumps,
                levels,
                timing,
            },
            id,
        ))
    }

    pub fn id(&self) -> MeetingId {
        self.shared.id
    }

    pub fn timing(&self) -> TranscribeTiming {
        self.timing
    }

    pub fn pause(&self) -> Result<()> {
        self.paused.store(true, Ordering::SeqCst);
        self.set_status(MeetingStatus::Paused)
    }

    pub fn resume(&self) -> Result<()> {
        self.paused.store(false, Ordering::SeqCst);
        self.set_status(MeetingStatus::Recording)
    }

    /// Stops both sources, transcribes the last audio, and waits for every pending chunk.
    pub async fn stop(mut self) -> Result<()> {
        self.mic.stop();
        if let Some(system) = self.system.as_mut() {
            system.stop();
        }
        for pump in self.pumps.drain(..) {
            pump.await
                .map_err(|e| MorselError::Audio(format!("audio task failed: {e}")))?;
        }
        self.levels.abort();
        self.set_status(MeetingStatus::Stopped)
    }

    fn set_status(&self, status: MeetingStatus) -> Result<()> {
        set_status(
            &self.shared.db.lock().expect("db lock"),
            self.shared.id,
            status,
        )?;
        self.shared.sink.emit(
            STATUS,
            json!({ "meetingId": self.shared.id, "status": status, "warning": null }),
        );
        Ok(())
    }
}

/// The file name of the saved audio of one speaker.
pub fn audio_file(speaker: Speaker) -> &'static str {
    match speaker {
        Speaker::You => "you.wav",
        Speaker::Others => "others.wav",
    }
}

/// Saves one stream on a blocking thread, so a slow disk never holds up the audio pump.
struct Recorder {
    frames: UnboundedSender<Vec<f32>>,
    writer: JoinHandle<()>,
}

impl Recorder {
    /// Returns `None` when the file cannot be made. The meeting continues without saved audio.
    fn start(shared: &Shared, path: &Path) -> Option<Self> {
        let mut file = match StreamWriter::create(path) {
            Ok(file) => file,
            Err(err) => {
                warn_audio(shared, &err);
                return None;
            }
        };
        let (frames, mut inbox) = unbounded_channel::<Vec<f32>>();
        let shared = shared.clone();
        let writer = tokio::task::spawn_blocking(move || {
            while let Some(frame) = inbox.blocking_recv() {
                if let Err(err) = file.write(&frame) {
                    warn_audio(&shared, &err);
                    return;
                }
            }
            if let Err(err) = file.finish() {
                warn_audio(&shared, &err);
            }
        });
        Some(Self { frames, writer })
    }

    fn save(&self, frame: &[f32]) {
        // The writer stops only after an error that it already reported.
        let _ = self.frames.send(frame.to_vec());
    }

    /// Waits until the file has its final header.
    async fn finish(self) {
        drop(self.frames);
        let _ = self.writer.await;
    }
}

fn warn_audio(shared: &Shared, err: &MorselError) {
    eprintln!("cannot save the meeting audio: {err}");
    shared.sink.emit(
        STATUS,
        json!({
            "meetingId": shared.id,
            "status": MeetingStatus::Recording,
            "warning": format!("Morsel cannot save the audio: {err}"),
        }),
    );
}

/// Reads one stream until its source stops. It saves each frame, and in live mode each complete
/// chunk becomes a transcription job.
async fn pump(
    shared: Shared,
    speaker: Speaker,
    mut rx: Receiver<Vec<f32>>,
    paused: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
    recorder: Option<Recorder>,
    timing: TranscribeTiming,
) {
    let mut chunker = Chunker::new(speaker);
    let mut jobs = JoinSet::new();
    let live = timing == TranscribeTiming::Live;
    while let Some(frame) = rx.recv().await {
        if paused.load(Ordering::SeqCst) {
            continue;
        }
        let value = rms(&frame);
        level.fetch_max(value.to_bits(), Ordering::Relaxed);
        if let Some(recorder) = &recorder {
            recorder.save(&frame);
        }
        if live {
            for chunk in chunker.push(&frame) {
                jobs.spawn(transcribe_chunk(shared.clone(), chunk));
            }
        }
    }
    if live && let Some(chunk) = chunker.flush() {
        jobs.spawn(transcribe_chunk(shared.clone(), chunk));
    }
    if let Some(recorder) = recorder {
        recorder.finish().await;
    }
    while jobs.join_next().await.is_some() {}
}

async fn transcribe_chunk(shared: Shared, chunk: AudioChunk) {
    let end_ms = chunk.start_ms + (chunk.samples.len() as u64 * 1000 / PIPELINE_RATE as u64);
    let _turn = shared
        .gate
        .acquire()
        .await
        .expect("the gate is never closed");
    let text = match transcribe_with_retry(
        shared.transcriber.as_ref(),
        &chunk.samples,
        shared.language.as_deref(),
    )
    .await
    {
        Some(text) if text.trim().is_empty() => return,
        Some(text) => text,
        None => format!("[missing {}–{}]", clock(chunk.start_ms), clock(end_ms)),
    };
    let line = TranscriptLine {
        meeting_id: shared.id,
        speaker: chunk.speaker,
        start_ms: chunk.start_ms,
        end_ms,
        text,
    };
    if let Err(err) = add_line(&shared.db.lock().expect("db lock"), &line) {
        eprintln!("cannot save a transcript line: {err}");
        return;
    }
    shared
        .sink
        .emit(TRANSCRIPT, serde_json::to_value(&line).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::source::AudioSource;
    use crate::db::meetings::get_meeting;
    use crate::events::{STATUS, TRANSCRIPT, VecSink};
    use crate::settings::TranscribeTiming;
    use crate::transcribe::Transcriber;
    use std::sync::Mutex;
    use tokio::sync::mpsc::Sender;

    type Slot = Arc<Mutex<Option<Sender<Vec<f32>>>>>;

    struct FakeSource {
        slot: Slot,
        fail: Option<fn() -> MorselError>,
    }

    impl AudioSource for FakeSource {
        fn start(&mut self, tx: Sender<Vec<f32>>) -> Result<()> {
            if let Some(make) = self.fail {
                return Err(make());
            }
            *self.slot.lock().unwrap() = Some(tx);
            Ok(())
        }
        fn stop(&mut self) {
            self.slot.lock().unwrap().take();
        }
    }

    fn source() -> (Box<dyn AudioSource>, Slot) {
        let slot: Slot = Arc::new(Mutex::new(None));
        (
            Box::new(FakeSource {
                slot: slot.clone(),
                fail: None,
            }),
            slot,
        )
    }

    fn failing(make: fn() -> MorselError) -> Box<dyn AudioSource> {
        Box::new(FakeSource {
            slot: Arc::new(Mutex::new(None)),
            fail: Some(make),
        })
    }

    async fn send(slot: &Slot, samples: Vec<f32>) {
        let tx = slot.lock().unwrap().clone().expect("source started");
        for piece in samples.chunks(1600) {
            tx.send(piece.to_vec()).await.unwrap();
        }
    }

    fn speech(seconds: f32) -> Vec<f32> {
        (0..(seconds * 16_000.0) as usize)
            .map(|i| (i as f32 * 0.07).sin() * 0.3)
            .collect()
    }

    struct FakeTranscriber {
        fail: bool,
        delay_ms: u64,
    }

    #[async_trait::async_trait]
    impl Transcriber for FakeTranscriber {
        async fn transcribe(&self, samples: &[f32], _l: Option<&str>) -> Result<String> {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            if self.fail {
                Err(MorselError::Http("down".into()))
            } else {
                Ok(format!("{} samples", samples.len()))
            }
        }
    }

    fn db() -> (tempfile::TempDir, SharedDb) {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open(&dir.path().join("m.db")).unwrap();
        (dir, Arc::new(Mutex::new(conn)))
    }

    fn deps(fail: bool, delay_ms: u64) -> (Arc<dyn Transcriber>, Arc<VecSink>) {
        (
            Arc::new(FakeTranscriber { fail, delay_ms }),
            Arc::new(VecSink::default()),
        )
    }

    fn start(
        db: &SharedDb,
        mic: Box<dyn AudioSource>,
        sys: Box<dyn AudioSource>,
        t: Arc<dyn Transcriber>,
        sink: Arc<VecSink>,
    ) -> Result<(MeetingSession, MeetingId)> {
        let data_dir = tempfile::tempdir().unwrap().keep();
        start_with(db, mic, sys, t, sink, data_dir, TranscribeTiming::Live)
    }

    fn start_with(
        db: &SharedDb,
        mic: Box<dyn AudioSource>,
        sys: Box<dyn AudioSource>,
        t: Arc<dyn Transcriber>,
        sink: Arc<VecSink>,
        data_dir: std::path::PathBuf,
        timing: TranscribeTiming,
    ) -> Result<(MeetingSession, MeetingId)> {
        MeetingSession::start(
            db.clone(),
            mic,
            sys,
            t,
            sink,
            MeetingSetup {
                title: "Weekly product sync".into(),
                template_id: 5,
                language: None,
                data_dir,
                timing,
            },
        )
    }

    fn saved_ms(data_dir: &std::path::Path, id: MeetingId, file: &str) -> u64 {
        let path = crate::audio::recording::audio_dir(data_dir, id).join(file);
        crate::audio::recording::StreamReader::open(&path)
            .unwrap()
            .duration_ms()
    }

    #[tokio::test]
    async fn meeting_audio_is_saved_for_each_speaker() {
        let (d, db) = db();
        let (t, sink) = deps(false, 0);
        let ((mic, mic_slot), (sys, sys_slot)) = (source(), source());
        let data = d.path().to_path_buf();
        let (session, id) =
            start_with(&db, mic, sys, t, sink, data.clone(), TranscribeTiming::Live).unwrap();
        send(&mic_slot, speech(3.0)).await;
        send(&sys_slot, speech(4.0)).await;
        session.stop().await.unwrap();
        assert_eq!(saved_ms(&data, id, "you.wav"), 3_000);
        assert_eq!(saved_ms(&data, id, "others.wav"), 4_000);
    }

    #[tokio::test]
    async fn paused_audio_is_not_saved() {
        let (d, db) = db();
        let (t, sink) = deps(false, 0);
        let ((mic, mic_slot), (sys, _s)) = (source(), source());
        let data = d.path().to_path_buf();
        let (session, id) =
            start_with(&db, mic, sys, t, sink, data.clone(), TranscribeTiming::Live).unwrap();
        send(&mic_slot, speech(5.0)).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        session.pause().unwrap();
        send(&mic_slot, speech(20.0)).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        session.resume().unwrap();
        send(&mic_slot, speech(5.0)).await;
        session.stop().await.unwrap();
        assert_eq!(saved_ms(&data, id, "you.wav"), 10_000);
    }

    #[tokio::test]
    async fn after_mode_records_and_does_not_transcribe() {
        let (d, db) = db();
        let counting = Arc::new(CountingTranscriber::default());
        let sink = Arc::new(VecSink::default());
        let ((mic, mic_slot), (sys, _s)) = (source(), source());
        let data = d.path().to_path_buf();
        let (session, id) = start_with(
            &db,
            mic,
            sys,
            counting.clone(),
            sink,
            data.clone(),
            TranscribeTiming::After,
        )
        .unwrap();
        send(&mic_slot, speech(12.0)).await;
        session.stop().await.unwrap();
        assert!(
            get_meeting(&db.lock().unwrap(), id)
                .unwrap()
                .lines
                .is_empty()
        );
        assert_eq!(counting.most.load(Ordering::SeqCst), 0);
        assert_eq!(saved_ms(&data, id, "you.wav"), 12_000);
    }

    #[tokio::test]
    async fn audio_write_error_keeps_live_transcription() {
        let (d, db) = db();
        let (t, sink) = deps(false, 0);
        let ((mic, mic_slot), (sys, _s)) = (source(), source());
        // A file where the audio folder must be: the folder cannot be made.
        let data = d.path().join("blocked");
        std::fs::write(&data, b"not a folder").unwrap();
        let (session, id) =
            start_with(&db, mic, sys, t, sink.clone(), data, TranscribeTiming::Live).unwrap();
        send(&mic_slot, speech(3.0)).await;
        session.stop().await.unwrap();
        assert_eq!(get_meeting(&db.lock().unwrap(), id).unwrap().lines.len(), 1);
        let warned = sink.named(STATUS).iter().any(|e| {
            e["warning"]
                .as_str()
                .is_some_and(|w| w.contains("cannot save the audio"))
        });
        assert!(warned, "{:?}", sink.named(STATUS));
    }

    /// Counts the calls that run at the same time.
    #[derive(Default)]
    struct CountingTranscriber {
        now: std::sync::atomic::AtomicUsize,
        most: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Transcriber for CountingTranscriber {
        async fn transcribe(&self, _samples: &[f32], _l: Option<&str>) -> Result<String> {
            let running = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(running, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            self.now.fetch_sub(1, Ordering::SeqCst);
            Ok("words".into())
        }
    }

    #[tokio::test]
    async fn live_mode_runs_one_transcription_at_a_time() {
        let (_d, db) = db();
        let counting = Arc::new(CountingTranscriber::default());
        let sink = Arc::new(VecSink::default());
        let ((mic, mic_slot), (sys, sys_slot)) = (source(), source());
        let (session, id) = start(&db, mic, sys, counting.clone(), sink).unwrap();
        tokio::join!(send(&mic_slot, speech(35.0)), send(&sys_slot, speech(35.0)));
        session.stop().await.unwrap();
        assert_eq!(get_meeting(&db.lock().unwrap(), id).unwrap().lines.len(), 4);
        assert_eq!(counting.most.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn lines_from_each_source_get_the_right_speaker() {
        let (_d, db) = db();
        let (t, sink) = deps(false, 0);
        let ((mic, mic_slot), (sys, sys_slot)) = (source(), source());
        let (session, id) = start(&db, mic, sys, t, sink.clone()).unwrap();
        send(&mic_slot, speech(3.0)).await;
        send(&sys_slot, speech(4.0)).await;
        session.stop().await.unwrap();
        let detail = get_meeting(&db.lock().unwrap(), id).unwrap();
        let speakers: Vec<Speaker> = detail.lines.iter().map(|l| l.speaker).collect();
        assert_eq!(speakers.len(), 2);
        assert!(speakers.contains(&Speaker::You) && speakers.contains(&Speaker::Others));
        assert_eq!(sink.named(TRANSCRIPT).len(), 2);
        assert_eq!(detail.summary.status, MeetingStatus::Stopped);
    }

    #[tokio::test]
    async fn pause_drops_audio_and_resume_continues_time() {
        let (_d, db) = db();
        let (t, sink) = deps(false, 0);
        let ((mic, mic_slot), (sys, _s)) = (source(), source());
        let (session, id) = start(&db, mic, sys, t, sink).unwrap();
        send(&mic_slot, speech(5.0)).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        session.pause().unwrap();
        send(&mic_slot, speech(20.0)).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        session.resume().unwrap();
        send(&mic_slot, speech(5.0)).await;
        session.stop().await.unwrap();
        let lines = get_meeting(&db.lock().unwrap(), id).unwrap().lines;
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].start_ms, 0);
        assert!(
            (9_900..=10_100).contains(&lines[0].end_ms),
            "end {}",
            lines[0].end_ms
        );
    }

    #[tokio::test]
    async fn stop_waits_for_pending_chunks() {
        let (_d, db) = db();
        let (t, sink) = deps(false, 300);
        let ((mic, mic_slot), (sys, _s)) = (source(), source());
        let (session, id) = start(&db, mic, sys, t, sink).unwrap();
        send(&mic_slot, speech(3.0)).await;
        session.stop().await.unwrap();
        assert_eq!(get_meeting(&db.lock().unwrap(), id).unwrap().lines.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn failed_chunk_adds_missing_line() {
        let (_d, db) = db();
        let (t, sink) = deps(true, 0);
        let ((mic, mic_slot), (sys, _s)) = (source(), source());
        let (session, id) = start(&db, mic, sys, t, sink).unwrap();
        send(&mic_slot, speech(3.0)).await;
        session.stop().await.unwrap();
        let lines = get_meeting(&db.lock().unwrap(), id).unwrap().lines;
        assert_eq!(lines[0].text, "[missing 0:00–0:03]");
    }

    #[tokio::test]
    async fn start_without_system_audio_permission_records_mic_only() {
        let (_d, db) = db();
        let (t, sink) = deps(false, 0);
        let (mic, mic_slot) = source();
        let (session, id) = start(
            &db,
            mic,
            failing(|| MorselError::SystemAudioPermissionDenied),
            t,
            sink.clone(),
        )
        .unwrap();
        let status = &sink.named(STATUS)[0];
        assert_eq!(status["meetingId"], id);
        assert!(status["warning"].as_str().unwrap().contains("Others"));
        send(&mic_slot, speech(2.0)).await;
        session.stop().await.unwrap();
        assert_eq!(get_meeting(&db.lock().unwrap(), id).unwrap().lines.len(), 1);
    }

    #[tokio::test]
    async fn start_without_mic_permission_returns_error_and_creates_no_meeting() {
        let (_d, db) = db();
        let (t, sink) = deps(false, 0);
        let (sys, _s) = source();
        let result = start(
            &db,
            failing(|| MorselError::MicPermissionDenied),
            sys,
            t,
            sink,
        );
        assert!(matches!(result, Err(MorselError::MicPermissionDenied)));
        let n: i64 = db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM meetings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn recover_marks_recording_meetings_interrupted() {
        let (_d, db) = db();
        let id = crate::db::meetings::create_meeting(&db.lock().unwrap(), "x", 1).unwrap();
        assert_eq!(recover(&db).unwrap(), 1);
        assert_eq!(
            get_meeting(&db.lock().unwrap(), id).unwrap().summary.status,
            MeetingStatus::Interrupted
        );
    }

    #[test]
    fn clock_formats_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(31_400), "0:31");
        assert_eq!(clock(754_000), "12:34");
    }
}

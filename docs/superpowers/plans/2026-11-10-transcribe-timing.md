# Transcribe Timing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user transcribe during the meeting or after it, save the meeting audio, and cut the
CPU load of live transcription.

**Architecture:** The session pumps write each stream to a WAV file. In live mode, chunks go through one
shared queue to the transcriber. In after mode, the pumps only record, and one app-wide after job reads
the WAV files, transcribes in time order, saves its position, and then starts the notes.

**Tech Stack:** Rust (Tauri 2, tokio, rusqlite, sherpa-onnx, `hound`, `libc`), React + Vitest.

**Spec:** `docs/superpowers/specs/2026-11-10-transcribe-timing-design.md`

## Global Constraints

- Audio: 16 kHz, mono, 16-bit PCM WAV, `<app data>/audio/<meeting id>/you.wav` and `others.wav`.
- Longest chunk: 28 s. Silence rule uses `SILENCE_RMS` (0.01) of `chunker.rs`.
- Setting: `transcribeTiming`, values `"live"` (default) and `"after"`.
- Status: `"transcribing"` between `"stopped"` and `"writing_notes"`.
- Event: `transcribe://progress` with `{ meetingId, doneMs, totalMs }`.
- UI copy: "When to transcribe", "While the meeting runs", "After the meeting", "Delete audio (<size>)",
  "Transcribing: <done> of <total> min".
- Keep the text "audio never leaves your Mac". Tests run with `NODE_OPTIONS=--max-old-space-size=1024`.

## Review Focus

1. **Disk full or file error during recording:** the live transcript must continue; the error goes to
   the log and the status warning. Test in Task 6: `audio_write_error_keeps_live_transcription`.
2. **A 3-hour meeting in after mode:** the job must read the WAV in pieces, not load 700 MB of `f32`.
   Test in Task 5: `reader_yields_audio_in_pieces`.
3. **No system audio (no `others.wav`):** the after job transcribes `you.wav` only. Test in Task 7:
   `after_job_works_with_one_stream`.
4. **Quit between a saved line and a saved position:** a restart must not add the line twice. Save
   both in one SQLite transaction. Test in Task 7: `resume_does_not_repeat_a_line`.
5. **A double click on Stop in after mode:** the meeting is queued one time only. Test in Task 8:
   `stop_twice_queues_the_meeting_once`.

---

### Task 1: Shorter chunks and the silence rule

**Files:** Modify `src-tauri/src/chunker.rs`, `src-tauri/src/session.rs`.

**Produces:** `pub const MAX_SAMPLES: usize = RATE * 28;` and `pub fn is_silent(samples: &[f32]) -> bool`
in `chunker.rs` (true when no 30 ms frame has `rms >= SILENCE_RMS`).

- [ ] Rename the test `always_cuts_at_30s` to `always_cuts_at_28s` (expects 28.0 s), and add
      `is_silent_is_true_for_zeros_and_false_for_a_tone`. Run them and see them fail.
- [ ] Add `session::tests::silent_chunks_get_no_line` (a silent mic stream gives no transcriber call).
- [ ] Implement the constant, `is_silent`, and the skip in `transcribe_chunk`.
- [ ] Check: `cargo test --manifest-path src-tauri/Cargo.toml chunker session` passes. Commit.

### Task 2: One live transcription at a time

**Files:** Modify `src-tauri/src/session.rs`.

**Produces:** `Shared.gate: Arc<tokio::sync::Semaphore>` with 1 permit. `transcribe_chunk` holds a permit
for the call.

- [ ] Add `live_mode_runs_one_transcription_at_a_time`: a fake transcriber that sleeps 50 ms and records
      the highest count of calls at the same time; two streams send 3 chunks each; expect 1. See it fail.
- [ ] Implement the semaphore.
- [ ] Check: `cargo test --manifest-path src-tauri/Cargo.toml session` passes. Commit.

### Task 3: Lower CPU priority for Whisper

**Files:** Modify `src-tauri/src/transcribe/whisper.rs`, `src-tauri/Cargo.toml`.

**Produces:** `fn set_utility_qos()` (macOS: `libc::pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0)`;
other targets: no-op). Create the recognizer and run each decode on threads that call it first, because
onnxruntime threads take the QoS of the thread that creates them.

- [ ] Add `set_utility_qos_sets_the_thread_class` (reads `pthread_get_qos_class_np`, macOS only). See it fail.
- [ ] Implement. Extend the ignored real-model test: two 30 s chunks one after the other must each take
      less than 15 s.
- [ ] Check: `cargo test --manifest-path src-tauri/Cargo.toml --release --lib -- --include-ignored whisper`.
      Write the measured times in the commit message. If the limit fails, remove the QoS call and record
      why. Commit.

### Task 4: Setting and status

**Files:** Modify `src-tauri/src/settings.rs`, `src-tauri/src/types.rs`, `src/lib/api.ts`.

**Produces:** `pub enum TranscribeTiming { #[default] Live, After }` (serde `snake_case`),
`Settings.transcribe_timing`, `MeetingStatus::Transcribing` (`as_str` = `"transcribing"`, and in every
match of `MeetingStatus`). TS: `transcribeTiming: "live" | "after"` and `"transcribing"` in the status type.

- [ ] Add `old_settings_default_to_live_timing` and `transcribing_status_round_trips`. See them fail.
- [ ] Implement, and add `transcribeTiming: "live"` to the TS test fixtures.
- [ ] Check: `cargo test --manifest-path src-tauri/Cargo.toml settings types` and `pnpm exec tsc --noEmit`. Commit.

### Task 5: WAV recorder and reader

**Files:** Create `src-tauri/src/audio/recording.rs`. Modify `src-tauri/src/audio/mod.rs`, `src-tauri/Cargo.toml`.

**Produces:**
- `pub fn audio_dir(data_dir: &Path, id: MeetingId) -> PathBuf` (`<data>/audio/<id>`).
- `pub struct StreamWriter` with `create(path: &Path) -> Result<Self>`, `write(&mut self, samples: &[f32]) -> Result<()>`,
  `finish(self) -> Result<()>` (`hound`, 16 kHz, mono, 16 bit).
- `pub struct StreamReader` with `open(path: &Path) -> Result<Self>`, `duration_ms(&self) -> u64`,
  `seek_ms(&mut self, ms: u64) -> Result<()>`, `read(&mut self, max: usize) -> Result<Vec<f32>>` (empty at the end).
  It reads the sample count from the file size, not from the header.

- [ ] Add `writes_and_reads_back_the_samples`, `a_wav_with_a_wrong_header_length_is_still_read`
      (write, drop the writer without `finish`, read all), `reader_yields_audio_in_pieces`,
      `seek_ms_skips_the_audio_before_it`. See them fail.
- [ ] Implement.
- [ ] Check: `cargo test --manifest-path src-tauri/Cargo.toml recording`. Commit.

### Task 6: Session saves audio; after mode only records

**Files:** Modify `src-tauri/src/session.rs`, `src-tauri/src/commands.rs`.

**Consumes:** Task 5 `StreamWriter`, `audio_dir`; Task 4 `TranscribeTiming`.
**Produces:** `MeetingSession::start(..., audio_dir: PathBuf, timing: TranscribeTiming)`. The pump writes
each frame that is not paused. In `After`, the pump makes no chunks. `stop` finishes the writers.

- [ ] Add `meeting_audio_is_saved_for_each_speaker`, `paused_audio_is_not_saved`,
      `after_mode_records_and_does_not_transcribe`, `audio_write_error_keeps_live_transcription`. See them fail.
- [ ] Implement. Writes must not block the pump (`spawn_blocking` writer task fed by a channel).
- [ ] Update the callers in `commands.rs` (pass `audio_dir(&self.data_dir, id)` and the setting).
- [ ] Check: `cargo test --manifest-path src-tauri/Cargo.toml session commands`. Commit.

### Task 7: The after job

**Files:** Create `src-tauri/src/transcribe/after.rs`, `src-tauri/src/db/migrations_3.sql`. Modify
`src-tauri/src/db/mod.rs`, `src-tauri/src/db/meetings.rs`.

**Produces:**
- Migration 3: `ALTER TABLE meetings ADD COLUMN transcribe_timing TEXT NOT NULL DEFAULT 'live';`
  `ALTER TABLE meetings ADD COLUMN transcribed_ms INTEGER NOT NULL DEFAULT 0;`
- `meetings::add_line_at(conn, line, transcribed_ms)` (line and position in one transaction),
  `meetings::set_timing`, `meetings::transcribed_ms`, `meetings::list_by_status`.
- `pub async fn transcribe_meeting(db, transcriber, sink, audio_dir, id, language, cancel: CancellationToken) -> Result<()>`:
  reads both streams from `transcribed_ms`, chunks each with `Chunker`, transcribes in `start_ms` order
  with the silence skip, emits `transcribe://progress`.

- [ ] Add `after_job_transcribes_in_time_order`, `after_job_continues_from_its_position`,
      `resume_does_not_repeat_a_line`, `after_job_works_with_one_stream`, `missing_audio_is_an_error`,
      and `migration_3_adds_the_timing_columns`. See them fail.
- [ ] Implement.
- [ ] Check: `cargo test --manifest-path src-tauri/Cargo.toml after db`. Commit.

### Task 8: Queue the after job in the app

**Files:** Modify `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/events.rs`.

**Consumes:** Task 7 `transcribe_meeting`. **Produces:** `AppState.after: AfterQueue` (one task, FIFO,
waits while `session` is `Some`). `stop_meeting` returns `TranscribeTiming`. In `After`, it sets
`Transcribing`, queues the meeting, and returns at once. At the end of a job, it calls the code of
`write_notes` with the meeting template. At app start, it queues each `Transcribing` meeting, and each
`Interrupted` after-mode meeting that has audio.

- [ ] Add `after_stop_returns_at_once_and_writes_notes_later`, `stop_twice_queues_the_meeting_once`,
      `after_job_waits_while_a_meeting_records`, `missing_audio_fails_only_that_meeting`,
      `start_queues_transcribing_and_interrupted_after_meetings`, `delete_meeting_cancels_its_job`. See them fail.
- [ ] Implement. `start_meeting` checks that the model is ready in both modes.
- [ ] Check: `cargo test --manifest-path src-tauri/Cargo.toml commands`. Commit.

### Task 9: Delete audio and count it

**Files:** Modify `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/types.rs`.

**Produces:** commands `audio_size(meeting_id) -> Option<u64>` and `delete_audio(meeting_id)`.
`delete_meeting` and `delete_all_data` also delete the audio. `data_info.bytes` counts the audio folder.

- [ ] Add `delete_audio_keeps_the_transcript`, `delete_audio_refuses_while_recording_or_transcribing`,
      `delete_meeting_deletes_its_audio`, `data_info_counts_the_audio`. See them fail.
- [ ] Implement and register the commands.
- [ ] Check: `cargo test --manifest-path src-tauri/Cargo.toml commands`. Commit.

### Task 10: Timing choice in Settings and onboarding

**Files:** Modify `src/screens/SettingsModelsScreen.tsx`, `src/screens/onboarding/AiStep.tsx`,
`src/screens/screens.test.tsx`, `src/screens/onboarding/onboarding.test.tsx`.

- [ ] Add `settings_saves_the_after_timing` and `ai_step_saves_the_after_timing` (radio group
      "When to transcribe", copy from the spec). See them fail.
- [ ] Implement.
- [ ] Check: `pnpm exec vitest run src/screens/screens.test.tsx src/screens/onboarding`. Commit.

### Task 11: After-mode stop and the transcribing step

**Files:** Modify `src/lib/api.ts`, `src/screens/meeting/MeetingScreen.tsx`, `src/screens/note/WritingScreen.tsx`,
`src/screens/note/note.test.tsx` (and `MeetingScreen.test.tsx`).

**Produces:** `api.stopMeeting(): Promise<"live" | "after">`, `onTranscribeProgress`, `MeetingSummary.transcribedMs`.

- [ ] Add `after_stop_opens_the_writing_page_without_write_notes` and
      `writing_page_shows_transcribing_progress` ("Transcribing: 12 of 60 min"). See them fail.
- [ ] Implement.
- [ ] Check: `pnpm exec vitest run src/screens/meeting src/screens/note`. Commit.

### Task 12: Delete audio button

**Files:** Modify `src/lib/api.ts`, `src/screens/note/NoteScreen.tsx`, `src/screens/note/note.test.tsx`.

The meeting page is the live page, so the button goes on the note page only (my call; the spec names both).

- [ ] Add `note_page_deletes_the_audio_after_a_confirmation` (shows "Delete audio (230 MB)", then no button). See it fail.
- [ ] Implement with the app's confirmation pattern.
- [ ] Check: `pnpm exec vitest run src/screens/note`. Commit.

### Task 13: Full gate and text

**Files:** `src-tauri/src/transcribe/mod.rs` (`wav_bytes` comment), `docs/superpowers/specs/2026-11-08-morsel-v1-design.md` (Parakeet lines).

- [ ] Update the text. Run `/do-tests full` and the release real-model test. Commit.

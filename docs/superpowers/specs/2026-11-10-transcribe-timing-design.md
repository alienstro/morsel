# Transcribe while the meeting runs, or after it

Date: 2026-11-10. Branch: `feat/local-tagalog`.

## Why

Whisper Turbo is the only on-device model now. It is about 3 times slower than Parakeet. During a
call, Morsel and the call app use the CPU at the same time, and the owner sees hardware lag. The
owner wants a choice: transcribe during the meeting, or transcribe after the meeting when the Mac
is free.

## Goals

1. A setting "When to transcribe" with two values:
   - **While the meeting runs** (`live`, the default): the behavior of today, with less CPU load.
   - **After the meeting** (`after`): Morsel records only. When the meeting stops, Morsel transcribes
     the saved audio and then writes the notes automatically.
2. Morsel saves the audio of each meeting on the Mac, in both modes. The user can delete it.
3. Less lag in live mode: one transcription at a time, a lower CPU priority, and no transcription of
   silent chunks.
4. No lost words at a forced 30 s cut (sherpa-onnx Whisper accepts only audio shorter than 30 s).

## Non-goals

- No audio player in the app. (My call. The user can open the file in the folder.)
- No compressed audio (Opus, FLAC). WAV first, compression later if disk space becomes a problem.
- No "transcribe again" button for a finished meeting.
- No new model or GPU work (Core ML, Metal).

## Anti-goals (a failure even if it works)

- Lag during a call in either mode.
- A meeting that loses its audio or its transcript because the app quit or crashed.
- Transcript lines out of time order, or text that nobody said (from silent audio).
- A stop button that blocks the page for minutes.

## Decided by the owner

- Two modes: live and after.
- Both modes save the audio. The user can delete it.
- After the meeting, the user sees the same pages as in live mode, and the notes come automatically.

## Design

### Setting

- `Settings.transcribe_timing: TranscribeTiming` with `Live` (default) and `After`, serde `snake_case`,
  JSON `transcribeTiming`. Old saved settings load as `Live`.
- The setting applies to the local model and to the cloud mode.
- UI: a "When to transcribe" radio group in Settings → Models (Transcription section) and on the
  first-run AI step:
  - "While the meeting runs": "The transcript grows during the call."
  - "After the meeting": "The Mac stays fast during the call. A 1-hour meeting takes about 20 minutes
    after you stop."

### Saved audio

- Folder: `<app data>/audio/<meeting id>/`, files `you.wav` and `others.wav` (the two `Speaker`
  streams). Format: 16 kHz, mono, 16-bit PCM WAV (`PIPELINE_RATE`), the format that the transcribers
  read. About 115 MB for each stream for each hour.
- The pump writes each frame to the file of its stream when the meeting is not paused. Paused audio
  is not saved, so the file time matches `start_ms` of the transcript lines.
- Use the `hound` crate for the WAV writer. The writer must not block the audio pump (write on a
  blocking task or through a buffered channel; builder's call).
- A crash leaves a WAV header with a wrong length. When Morsel reads a file, it must take the length
  from the file size, so that an interrupted recording is still readable.
- Delete:
  - The meeting page and the note page show "Delete audio (230 MB)" when the folder exists. The click
    asks for a confirmation, then deletes the folder. The transcript and the notes stay.
  - Morsel refuses to delete the audio of a meeting that records or transcribes now.
  - Delete meeting and delete all data also delete the audio.
  - The data size in Settings counts the audio folder.
- Text: the app keeps "audio never leaves your Mac". Code comments that say "audio never goes to disk"
  change (for example `transcribe::wav_bytes`).

### Live mode

- One transcription job at a time for the whole meeting (both streams share one queue, for example a
  `Semaphore` of 1). The chunks wait in memory.
- A chunk with an RMS level below `SILENCE_RMS` over the whole chunk gets no transcription and no
  line. (The builder picks the exact rule, for example "no 30 ms frame above the threshold".)
- The longest chunk is 28 s (was 30 s). Update the chunker test `always_cuts_at_30s`.
- Lower CPU priority: run the Whisper decode on a thread with the macOS QoS class `utility`. Check
  with a measurement that Whisper still keeps up with two streams (below 0.5 of real time for each
  stream). If `utility` is too slow, keep the default class and write down the result.
- Stop works as today: `stop_meeting` waits for the pending chunks, then the page calls `write_notes`.

### After mode

- During the meeting, the pumps write the audio and the levels, and make no chunks.
- New status `MeetingStatus::Transcribing` (`"transcribing"`), between `Stopped` and `WritingNotes`.
- `stop_meeting` closes the WAV files, sets `Transcribing`, puts the meeting in the after-job queue, and
  returns at once. It returns the timing so that the page knows the next step (or the page reads the
  setting; builder's call).
- The after job (one background task for the app):
  - Reads `you.wav` and `others.wav`, cuts each one with the same `Chunker`, and transcribes the chunks
    in time order across both speakers (`start_ms`), one at a time, with the live-mode rules (silence
    skip, 28 s, QoS `utility` or `background`).
  - Saves each line at once and emits `transcribe://progress` `{ meetingId, doneMs, totalMs }`.
  - Saves its position (`meetings.transcribed_ms`, new column in migration 3) after each chunk.
  - At the end, it calls the same code as `write_notes` with the template of the meeting. The status
    then goes `Transcribing` → `WritingNotes` → `Done`, with the same events as today.
  - Pauses while a meeting records, and continues when it stops.
  - Runs one meeting at a time, oldest first.
- Restart: at app start, after `recover`, Morsel puts each meeting with status `Transcribing` back in
  the queue. The job continues from `transcribed_ms`.
- Crash during an after-mode recording: `recover` marks the meeting `Interrupted` as today. If its audio
  exists, Morsel sets it to `Transcribing` and queues it, so the saved audio is not lost. (My call.)
- UI: in after mode, the stop button calls `stop_meeting` and goes to the writing page without a
  `write_notes` call. The writing page shows a first step "Transcribing: 12 of 60 min" from the
  progress event (and from `transcribed_ms` when the page opens), then the steps of today.

### Calls made in the builder check (my calls)

- Each meeting keeps the timing that it started with, in a new column `meetings.transcribe_timing`
  (migration 3, default `'live'`). A change of the setting during a meeting has no effect on it.
  `recover` uses this column to find after-mode meetings.
- `start_meeting` checks that the model is ready in both modes, so that the after job cannot fail later
  for a missing model.
- The after job uses the meeting language of the settings at the time of the job.
- Delete meeting during `Transcribing` removes the meeting from the queue, stops its job after the
  current chunk, and then deletes the meeting and its audio.
- An interrupted live-mode meeting is not queued again. Its transcript has the lines that were done.

### Errors

- A chunk that fails 3 times gets the line `[missing m:ss–m:ss]`, as today.
- A missing or unreadable audio file stops the job for that meeting with status `NotesFailed` and a
  message. The other meetings in the queue continue.

## Left to the builder

- The exact module layout (for example `audio/recording.rs` for the WAV writer and
  `transcribe/after.rs` for the job).
- The QoS call (`pthread_set_qos_class_self_np` through `libc`), and whether the after job uses
  `background` or `utility`.
- How the job shares the `Transcriber` with the live session (one loaded Whisper is enough).

## Tests (names are a guide)

- `settings`: `old_settings_default_to_live_timing`.
- `chunker`: `always_cuts_at_28s`.
- `session`: `live_mode_runs_one_transcription_at_a_time`, `silent_chunks_get_no_line`,
  `meeting_audio_is_saved_for_each_speaker`, `paused_audio_is_not_saved`,
  `after_mode_records_and_does_not_transcribe`.
- `recording`: `a_wav_with_a_wrong_header_length_is_still_read`.
- `after`: `after_job_transcribes_in_time_order_then_writes_notes`, `after_job_continues_from_its_position`,
  `after_job_waits_while_a_meeting_records`, `missing_audio_fails_only_that_meeting`.
- `commands`: `delete_audio_keeps_the_transcript`, `delete_audio_refuses_while_transcribing`,
  `delete_meeting_deletes_its_audio`.
- UI: the timing radio in Settings and onboarding, the "Delete audio" button, the writing page in
  after mode (no `write_notes` call, the "Transcribing" step with progress).
- Real model (ignored test): Whisper with QoS `utility` keeps up with two streams.

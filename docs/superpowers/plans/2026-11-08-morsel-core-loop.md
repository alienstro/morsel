# Morsel core loop implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The user starts a meeting, sees a live "You" and "Others" transcript, types rough notes, clicks "Stop and write notes", and gets a finished note that is saved on the Mac.

**Architecture:** A Tauri 2 app. The Rust backend captures two audio streams, cuts them into 10–30 s chunks, transcribes each chunk, saves each line in SQLite, and sends events to the React frontend. On stop, the backend sends the template, the rough notes, and the transcript to an OpenAI-compatible model and saves the Markdown note.

**Tech Stack:** Tauri 2, Rust stable, React + TypeScript + Vite, Tiptap, react-router, rusqlite (`bundled`), rusqlite_migration, keyring, cpal, objc2-core-audio, rubato, sherpa-onnx, reqwest, tokio, tokio-util (`CancellationToken`), async-trait, serde, thiserror, wiremock (tests), Vitest + Testing Library.

**Spec:** `docs/superpowers/specs/2026-11-08-morsel-v1-design.md`

## Global constraints

- macOS 14.2 or later. Bundle identifier `app.morsel`. Product name `Morsel`.
- Minimum window size 1100 × 700 px.
- Rust installed with `rustup`. Run `rustup target add x86_64-apple-darwin` once.
- `Info.plist` has `NSMicrophoneUsageDescription` and `NSAudioCaptureUsageDescription`.
- Morsel never writes audio to disk.
- API keys live only in the Keychain, service `app.morsel`, accounts `notes_api_key` and `transcribe_api_key`. No key goes to the frontend, a log, or SQLite.
- Audio inside the pipeline is 16 kHz mono `f32`.
- Chunks: cut at the first silence ≥ 400 ms after 10 s; always cut at 30 s. Silence = RMS < 0.01 over 30 ms frames.
- Long transcript limit: 100,000 tokens, estimated as characters ÷ 4.
- Cloud transcription default: base URL `https://api.groq.com/openai/v1`, model `whisper-large-v3-turbo`.
- Colors: `#EEEFEA`, `#FAFAF7`, `#22261F`, `#5D6354`, `#D5D8CD`, `#A2513C`, `#D9826A`. Fonts: EB Garamond, Inter.
- Buttons never shrink or wrap (`flex-shrink: 0; white-space: nowrap`).
- Commit messages: Conventional Commits. No AI attribution lines.

## Shared types

These names are fixed. All tasks use them.

```rust
pub type MeetingId = i64;
pub enum Speaker { You, Others }                       // serde: "you" | "others"
pub enum MeetingStatus { Recording, Paused, Stopped, WritingNotes, Done,
                         NotesCancelled, NotesFailed, Interrupted } // serde: snake_case
pub struct TranscriptLine { pub meeting_id: MeetingId, pub speaker: Speaker,
                            pub start_ms: u64, pub end_ms: u64, pub text: String }
pub struct AudioChunk { pub speaker: Speaker, pub start_ms: u64, pub samples: Vec<f32> }
pub enum MorselError { /* thiserror; serializes to { code: String, message: String } */ }
```

Events (payloads in camelCase): `meeting://transcript` (`TranscriptLine`), `meeting://levels` (`{ you: f32, others: f32 }`), `meeting://status` (`{ meetingId, status, warning: Option<String> }`), `notes://done` (`{ meetingId }`), `notes://failed` (`{ meetingId, message }`), `model://download` (`{ received: u64, total: u64 }`).

## Review focus

1. **Permission denied:** without system-audio permission, the meeting records the microphone only and warns. Without microphone permission, the meeting does not start. Tests in Task 12.
2. **Crash during a meeting:** saved lines and rough notes survive, and the meeting becomes `interrupted` on the next start. Tests in Task 12 and Task 15.
3. **Notes request fails:** the rough notes and the transcript stay, and the user can try again. Test in Task 14.
4. **Very long meeting:** a 3-hour transcript is split into parts, not sent as one request. Test in Task 14.
5. **Audio device changes during a meeting** (headphones plugged in): the stream restarts on the new device. Test in Task 7.

## Task loop

Use this loop for each task:

- [ ] Write the tests of the task. Run the check command. The new tests fail.
- [ ] Write the code. Run the check command. All tests pass.
- [ ] Commit with the message of the task.

---

### Task 1: Project setup

**Goal:** `pnpm tauri dev` opens an empty Morsel window with the right size limits and permissions text.

**Files:** create with `pnpm create tauri-app` (React + TypeScript, pnpm), then change `src-tauri/tauri.conf.json` (identifier, product name, `minWidth: 1100`, `minHeight: 700`), `src-tauri/Info.plist` (two usage keys), `.gitignore` (add `.superpowers/`, `src-tauri/target/`), `package.json` (add `vitest`, `@testing-library/react`, `jsdom`, script `"test": "vitest run"`).

**Tests:** `src/App.test.tsx` → `renders the Morsel window title`. `src-tauri/src/lib.rs` → `#[test] fn app_builds()` (builds the `tauri::Builder` with no window).

**Check:** `pnpm test && cargo test --manifest-path src-tauri/Cargo.toml`

**Commit:** `chore: set up the Tauri 2 project`

### Task 2: Design tokens and app shell

**Goal:** The left sidebar of the canvas (Morsel, New note, Home, Notes, Templates, Ask Morsel, Recent, Settings) and the main surface show on every screen.

**Files:** create `src/styles/tokens.css`, `src/app/AppShell.tsx`, `src/app/routes.tsx`; test `src/app/AppShell.test.tsx`.

**Interfaces:** Routes: `/`, `/meeting/:id`, `/meeting/:id/writing`, `/note/:id`, `/settings/models`. Notes, Templates, and Ask Morsel links go to a "Coming soon" page in this plan.

**Tests:** `shows all sidebar links`, `marks the current page with aria-current`, `New note button has an aria-label`.

**Check:** `pnpm vitest run src/app`

**Commit:** `feat: add design tokens and app shell`

### Task 3: Database and meetings store

**Goal:** One SQLite file holds meetings, transcript lines, rough notes, notes, templates, settings, and usage.

**Files:** create `src-tauri/src/db/mod.rs`, `src-tauri/src/db/migrations.sql`, `src-tauri/src/db/meetings.rs`, `src-tauri/src/types.rs` (shared types).

**Interfaces:**
- `pub fn open(path: &Path) -> Result<Connection, MorselError>` (runs migrations; WAL mode).
- Tables: `meetings(id, title, template_id, status, started_at, ended_at)`, `transcript_lines(id, meeting_id, speaker, start_ms, end_ms, text)`, `user_notes(meeting_id PK, body, updated_at)`, `notes(meeting_id PK, body_md, model, created_at)`, `templates(id, name, sections_json, instructions, built_in)`, `settings(key PK, value)`, `usage(id, at, model, prompt_tokens, completion_tokens, cost)`.
- Seed the five templates of spec section 9.
- `pub struct Template { id: i64, name: String, sections: Vec<String>, instructions: String }`; `pub struct MeetingSummary { id: MeetingId, title: String, template_name: String, status: MeetingStatus, started_at: i64 }`; `pub struct MeetingDetail { summary: MeetingSummary, lines: Vec<TranscriptLine>, user_notes: String, note: Option<String> }`; `get_template(&Connection, i64) -> Result<Template>`.
- `create_meeting(&Connection, title: &str, template_id: i64) -> Result<MeetingId>`; `set_status(&Connection, MeetingId, MeetingStatus)`; `add_line(&Connection, &TranscriptLine)`; `save_user_notes(&Connection, MeetingId, &str)`; `save_note(&Connection, MeetingId, body_md: &str, model: &str)`; `get_meeting(&Connection, MeetingId) -> Result<MeetingDetail>`; `list_meetings(&Connection, limit: u32) -> Result<Vec<MeetingSummary>>`; `mark_open_meetings_interrupted(&Connection) -> Result<u32>`.

**Tests:** `open_runs_migrations_twice_without_error`, `seeds_five_templates`, `lines_come_back_in_start_order`, `user_notes_upsert_keeps_one_row`, `mark_open_meetings_interrupted_changes_only_recording_and_paused` (returns 2 for one recording and one paused meeting; a `done` meeting stays `done`).

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml db::`

**Commit:** `feat: add SQLite store for meetings`

### Task 4: Settings and Keychain

**Goal:** The backend reads and writes settings, and keeps keys only in the Keychain.

**Files:** create `src-tauri/src/settings.rs`, `src-tauri/src/secrets.rs`.

**Interfaces:**
- `pub struct Settings { notes_base_url: String, notes_model: String, quick_model: String, transcribe_mode: TranscribeMode /* Local | Cloud */, transcribe_base_url: String, transcribe_model: String, language: String /* "auto" */ }` with `Default` (notes base URL `https://openrouter.ai/api/v1`, empty model names, Groq values of the constraints, mode `Local`).
- `load_settings(&Connection) -> Result<Settings>`; `save_settings(&Connection, &Settings) -> Result<()>`.
- `pub trait SecretStore: Send + Sync { fn get(&self, account: &str) -> Result<Option<String>>; fn set(&self, account: &str, value: &str) -> Result<()>; fn delete(&self, account: &str) -> Result<()>; }` with `KeychainStore` (service `app.morsel`) and `MemoryStore` (tests).

**Tests:** `default_settings_use_local_transcription`, `settings_round_trip`, `memory_store_set_get_delete`, `settings_table_never_contains_a_key` (save settings and a key; no `settings` row value equals the key).

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml settings:: secrets::`

**Commit:** `feat: add settings and Keychain store`

### Task 5: OpenAI-compatible client

**Goal:** One client sends chat requests to OpenRouter, OpenAI, Groq, or a local server.

**Files:** create `src-tauri/src/llm.rs`.

**Interfaces:** `pub struct LlmConfig { base_url: String, model: String, api_key: String }`; `pub struct ChatMessage { role: String, content: String }`; `pub struct ChatResult { text: String, prompt_tokens: u32, completion_tokens: u32, cost: Option<f64> }`; `pub async fn chat(http: &reqwest::Client, cfg: &LlmConfig, messages: &[ChatMessage]) -> Result<ChatResult, MorselError>` (POST `{base_url}/chat/completions`, timeout 120 s; `cost` from `usage.cost` when present); `pub async fn test_key(http, cfg) -> Result<(), MorselError>` (GET `{base_url}/models`).

**Tests (wiremock):** `chat_sends_bearer_key_and_model`, `chat_reads_usage_and_optional_cost`, `http_401_maps_to_invalid_key_error`, `http_429_maps_to_rate_limit_error`, `trailing_slash_in_base_url_is_ignored`.

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml llm::`

**Commit:** `feat: add OpenAI-compatible chat client`

### Task 6: Resampler and level meter

**Goal:** Any device audio becomes 16 kHz mono, and each stream reports a level.

**Files:** create `src-tauri/src/audio/mod.rs`, `src-tauri/src/audio/resample.rs`, `src-tauri/src/audio/level.rs`.

**Interfaces:** `pub struct Resampler` with `new(input_rate: u32, channels: u16) -> Result<Self>` and `process(&mut self, interleaved: &[f32]) -> Vec<f32>` (rubato, output 16,000 Hz mono); `pub fn rms(samples: &[f32]) -> f32` (result clamped to 0.0–1.0).

**Tests:** `stereo_48k_one_second_gives_16000_samples` (±1 %), `mono_16k_passes_through`, `rms_of_silence_is_zero`, `rms_of_full_scale_sine_is_about_0_707`.

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml audio::`

**Commit:** `feat: add resampler and level meter`

### Task 7: Microphone source

**Goal:** The microphone stream sends 16 kHz mono frames and restarts on a device change.

**Files:** create `src-tauri/src/audio/source.rs`, `src-tauri/src/audio/mic.rs`.

**Interfaces:** `pub trait AudioSource: Send { fn start(&mut self, tx: tokio::sync::mpsc::Sender<Vec<f32>>) -> Result<(), MorselError>; fn stop(&mut self); }`; `pub struct MicSource` (cpal default input; uses `Resampler`). `MicSource` takes a `DeviceWatcher` trait object (`fn default_input_changed(&self) -> bool`, polled each 1 s) so tests can fake a device change. Permission denied maps to `MorselError::MicPermissionDenied`.

**Tests:** `restarts_on_default_device_change` (fake watcher returns `true` once; the fake stream factory is called 2 times; frames continue), `stop_ends_the_stream`. Manual check: `cargo run --example mic_check` prints a level above 0.05 when you speak.

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml audio::mic`

**Commit:** `feat: add microphone source`

### Task 8: System audio source

**Goal:** Morsel records the sound that the Mac plays, as the "Others" stream.

**Files:** create `src-tauri/src/audio/system_tap.rs`, `src-tauri/examples/tap_check.rs`.

**Interfaces:** `pub struct SystemTapSource` implements `AudioSource`. It creates a global process tap (`CATapDescription`, excludes Morsel's own process), an aggregate device that holds the tap, and an IO proc that feeds `Resampler`. `stop` destroys the IO proc, the aggregate device, and the tap. Errors: `MorselError::SystemAudioPermissionDenied`, `MorselError::SystemAudioUnsupported` (macOS < 14.2).

**Tests:** `unsupported_os_version_returns_unsupported` (version check is a pure function `fn tap_supported(major: u32, minor: u32) -> bool`; 14.1 → false, 14.2 → true). Manual check: play a YouTube video, run `cargo run --example tap_check`; it prints a level above 0.05 and asks for permission on the first run.

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml audio::system_tap`

**Commit:** `feat: add system audio tap source`

### Task 9: Chunker

**Goal:** Each stream is cut into 10–30 s chunks at silences.

**Files:** create `src-tauri/src/chunker.rs`.

**Interfaces:** `pub struct Chunker` with `new(speaker: Speaker) -> Self`, `push(&mut self, samples: &[f32]) -> Vec<AudioChunk>`, `flush(&mut self) -> Option<AudioChunk>`. Values from the global constraints. A chunk with RMS < 0.01 over all frames is dropped. `start_ms` counts from the meeting start; pause time is not counted.

**Tests:** `cuts_at_first_400ms_silence_after_10s` (12 s speech, 0.5 s silence, 5 s speech → first chunk is 12–12.5 s long), `never_cuts_before_10s`, `always_cuts_at_30s`, `drops_all_silent_chunk`, `start_ms_of_second_chunk_equals_length_of_first`, `flush_returns_the_rest`.

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml chunker::`

**Commit:** `feat: add audio chunker`

### Task 10: Cloud transcriber

**Goal:** A chunk is transcribed by any OpenAI-compatible `/audio/transcriptions` endpoint, with retries.

**Files:** create `src-tauri/src/transcribe/mod.rs`, `src-tauri/src/transcribe/cloud.rs`.

**Interfaces:** `#[async_trait] pub trait Transcriber: Send + Sync { async fn transcribe(&self, samples: &[f32], language: Option<&str>) -> Result<String, MorselError>; }`; `pub fn wav_bytes(samples: &[f32]) -> Vec<u8>` (16-bit PCM, 16 kHz, in memory only); `pub struct CloudTranscriber { http, base_url, model, api_key }`; `pub async fn transcribe_with_retry(t: &dyn Transcriber, samples: &[f32], language: Option<&str>) -> Option<String>` (3 tries, waits 2 s and 4 s; `None` after the last failure).

**Tests (wiremock, paused tokio time):** `sends_multipart_with_file_and_model`, `language_auto_is_not_sent`, `wav_header_says_16000_hz_mono`, `retries_twice_then_returns_none`, `second_try_success_returns_text`.

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml transcribe::cloud`

**Commit:** `feat: add cloud transcriber with retries`

### Task 11: Local Parakeet transcriber

**Goal:** Morsel downloads Parakeet one time and transcribes on the Mac.

**Files:** create `src-tauri/src/transcribe/parakeet.rs`, `src-tauri/src/transcribe/model_download.rs`, `src-tauri/tests/fixtures/hello_16k.wav` (3 s, "hello Morsel this is a test").

**Interfaces:** `pub const PARAKEET_URL: &str` and `pub const PARAKEET_SHA256: &str` (the sherpa-onnx release `parakeet-tdt-0.6b-v3-int8`; pin both when you write the task); `pub async fn ensure_model(http, dir: &Path, on_progress: impl Fn(u64, u64)) -> Result<PathBuf, MorselError>` (download to a `.part` file, check SHA-256, then rename; a wrong hash deletes the file and returns `MorselError::ModelChecksum`); `pub struct ParakeetTranscriber` implements `Transcriber` (sherpa-onnx offline recognizer, runs in `spawn_blocking`).

**Tests:** `wrong_checksum_deletes_the_part_file` (wiremock), `existing_model_is_not_downloaded_again`, `#[ignore] transcribes_fixture_with_words_hello_and_test` (needs the model; run with `--ignored`). Measure on this Mac: a 30 s chunk takes less than 3 s. Write the result in the commit message.

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml transcribe::parakeet -- --include-ignored`

**Commit:** `feat: add local Parakeet transcriber`

### Task 12: Meeting session

**Goal:** One object runs a meeting: sources → chunkers → transcriber → SQLite → events.

**Files:** create `src-tauri/src/session.rs`, `src-tauri/src/events.rs`.

**Interfaces:** `pub trait EventSink: Send + Sync { fn emit(&self, name: &str, payload: serde_json::Value); }` (Tauri version in Task 13, `VecSink` for tests). `pub struct MeetingSession` with `start(db, mic: Box<dyn AudioSource>, system: Box<dyn AudioSource>, transcriber: Arc<dyn Transcriber>, sink: Arc<dyn EventSink>, title: &str, template_id: i64) -> Result<(Self, MeetingId)>`, `pause()`, `resume()`, `stop() -> Result<()>` (flushes both chunkers and waits for all transcriptions). A missing chunk adds the line `[missing m:ss–m:ss]`. Levels go out 10 times each second. On app start, call `mark_open_meetings_interrupted`.

**Tests (fake sources and a fake transcriber):** `lines_from_each_source_get_the_right_speaker`, `pause_drops_audio_and_resume_continues_time`, `stop_waits_for_pending_chunks`, `failed_chunk_adds_missing_line`, `start_without_system_audio_permission_records_mic_only` (status event has a warning), `start_without_mic_permission_returns_error_and_creates_no_meeting`, `recover_marks_recording_meetings_interrupted`.

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml session::`

**Commit:** `feat: add meeting session`

### Task 13: Tauri commands

**Goal:** The frontend controls Morsel only through typed commands and events.

**Files:** create `src-tauri/src/commands.rs`; change `src-tauri/src/lib.rs` (app state, `TauriSink`, register commands, run recovery), `src-tauri/capabilities/default.json` (core events plus the Morsel commands only).

**Interfaces:** commands `start_meeting(title: Option<String>, template_id: i64) -> MeetingId` (no title → `Untitled meeting`), `pause_meeting()`, `resume_meeting()`, `stop_meeting()`, `save_user_notes(meeting_id, body: String)`, `write_notes(meeting_id, template_id)`, `cancel_notes(meeting_id)`, `get_meeting(meeting_id) -> MeetingDetail`, `list_meetings(limit: u32) -> Vec<MeetingSummary>`, `list_templates() -> Vec<TemplateSummary>`, `get_settings() -> Settings`, `save_settings(settings)`, `set_api_key(account: String, key: String)`, `has_api_key(account: String) -> bool`, `test_notes_key() -> ()`, `download_model() -> ()`. Each command is a thin call into an `AppState` method, so the tests call the methods directly.

**Tests:** `set_api_key_rejects_unknown_account`, `has_api_key_never_returns_the_key` (the return type is `bool`; a test checks the serialized response), `start_meeting_twice_returns_already_recording_error`.

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml commands::`

**Commit:** `feat: expose meeting commands to the frontend`

### Task 14: Notes writer

**Goal:** Stop makes a full Markdown note from the template, the rough notes, and the transcript.

**Files:** create `src-tauri/src/notes/prompt.rs`, `src-tauri/src/notes/writer.rs`.

**Interfaces:** `pub fn build_messages(template: &Template, user_notes: &str, lines: &[TranscriptLine]) -> Vec<ChatMessage>` (system message: write one `## ` heading for each section, keep each rough-notes line word for word, action items as `- [ ] Owner: task`, use only facts from the input; transcript lines as `[m:ss] You: …` / `[m:ss] Others: …`); `pub fn estimate_tokens(text: &str) -> usize` (chars ÷ 4); `pub async fn write_notes(state, meeting_id, template_id, cancel: CancellationToken) -> Result<(), MorselError>` (status `writing_notes` → `done`, or `notes_cancelled`, or `notes_failed`; saves each request in `usage`; emits `notes://done` or `notes://failed`). When the meeting title is `Untitled meeting`, the quick model writes a title of at most 6 words after the note. Above 100,000 tokens: split the transcript into parts of at most 80,000 tokens at line ends, summarize each part with the notes model, then write the note from the summaries.

**Tests (wiremock):** `prompt_has_one_heading_per_section`, `prompt_marks_speakers_and_times`, `long_transcript_is_split_into_parts` (a 500,000-char transcript makes 3 summary requests + 1 final request), `cancel_keeps_user_notes_and_transcript`, `failed_request_keeps_user_notes_and_transcript` (status `notes_failed`, a second call works), `empty_transcript_uses_user_notes_only`, `usage_row_is_saved`, `untitled_meeting_gets_a_title_from_quick_model`.

**Check:** `cargo test --manifest-path src-tauri/Cargo.toml notes::`

**Commit:** `feat: write notes from rough notes and transcript`

### Task 15: Meeting screen

**Goal:** Screen 2.3 works: meeting list, notepad, live transcript on the right with Hide / Show and `⌘T`, levels, Pause, Stop.

**Files:** create `src/lib/api.ts` (typed `invoke` and `listen` wrappers for Task 13), `src/screens/meeting/MeetingScreen.tsx`, `src/screens/meeting/TranscriptPanel.tsx`; test `src/screens/meeting/MeetingScreen.test.tsx`.

**Interfaces:** `api.ts` exports one function for each command of Task 13, with the same names in camelCase, and `onTranscript`, `onLevels`, `onStatus`, `onNotesDone`, `onNotesFailed`. The notepad is Tiptap. It saves rough notes with `saveUserNotes` 2 s after the last keystroke and on stop. `⌘T` toggles the transcript panel. The panel scrolls to the newest line unless the user scrolled up. "Stop and write notes" calls `stopMeeting`, then `writeNotes`, then goes to `/meeting/:id/writing`. The "Ask about this meeting" box shows but is disabled until Plan 2.

**Tests (mock `@tauri-apps/api`):** `new_transcript_event_adds_a_line_with_speaker_label`, `hide_and_cmd_t_toggle_the_panel`, `rough_notes_save_2s_after_typing`, `stop_saves_notes_then_writes_then_navigates`, `system_audio_warning_shows_in_the_status_bar`.

**Check:** `pnpm vitest run src/screens/meeting`

**Commit:** `feat: add meeting screen`

### Task 16: Writing and finished note screens

**Goal:** Screens 3.1 and 3.2 work: progress, cancel, the finished note with your words dark and Morsel's words grey, Copy, Export as Markdown, Write again.

**Files:** create `src/lib/markYours.ts`, `src/screens/note/WritingScreen.tsx`, `src/screens/note/NoteScreen.tsx`; change `src-tauri/src/commands.rs` (add `export_note`); test `src/screens/note/note.test.tsx`.

**Interfaces:** `markYours(noteMd: string, userNotes: string): { text: string; yours: boolean }[]` (a line is yours when it equals a rough-notes line after trim and lower case). Export uses the Tauri dialog plugin to pick a file and writes `<title>.md` through a backend command `export_note(meeting_id, path)` (add to Task 13's `commands.rs`).

**Tests:** `markYours_matches_case_and_space_insensitive`, `markYours_ignores_empty_lines`, `writing_screen_cancel_calls_cancelNotes_and_shows_rough_notes`, `notes_failed_shows_message_and_try_again`, `note_screen_renders_legend_and_grey_lines`.

**Check:** `pnpm vitest run src/screens/note src/lib`

**Commit:** `feat: add writing and finished note screens`

### Task 17: Basic Home and Settings 5.1

**Goal:** The user can enter keys and models, download Parakeet, and start an unscheduled meeting from Home.

**Files:** create `src/screens/HomeScreen.tsx`, `src/screens/SettingsModelsScreen.tsx`; test `src/screens/screens.test.tsx`.

**Interfaces:** Home shows "Start notes" with a template picker and the 5 latest meetings (`listMeetings(5)`); the calendar card is not in this plan. Settings shows notes base URL, notes model, quick model, key field (write only; it shows "Saved" from `hasApiKey`, never the key), "Test key", transcription Local / Cloud, cloud base URL, model, and key, language, and the Parakeet download with a progress bar.

**Tests:** `start_notes_uses_the_selected_template`, `key_field_never_shows_a_saved_key`, `test_key_shows_ok_and_error_messages`, `download_shows_progress_from_events`.

**Check:** `pnpm vitest run src/screens`

**Commit:** `feat: add basic home and model settings`

---

## Final check

Run once at the end:

```bash
pnpm test && cargo test --manifest-path src-tauri/Cargo.toml && pnpm tauri build --debug
```

Then a real test: a 5-minute call with one other person. Pass = "You" and "Others" lines show during the call, and a finished note opens after "Stop and write notes".

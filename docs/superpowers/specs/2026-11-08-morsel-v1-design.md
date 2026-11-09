# Morsel v1 design

Date: 2026-11-08. Status: approved screens, spec waiting for review.

Screens: https://claude.ai/artifact/JdHBV2LwQbUN4FVRzT7mxp (version 28). The screens are the source of truth for the look. This file is the source of truth for the behavior.

## 1. Goal

Morsel is a macOS app that writes meeting notes. The user opens Morsel, clicks "Start notes", and types rough points during the call. Morsel records the microphone and the system audio, makes a live transcript, and then writes full notes from the rough points and the transcript.

Morsel never joins the call. The other people see nothing in the call.

## 2. Users and cost

- Morsel is for the owner and for other people. Each user brings their own API keys (BYOK). Morsel has no server and no bill of its own.
- The default cost of a 1-hour meeting is $0 for the transcript (local Whisper Turbo) plus the cost of the notes model of the user.

## 3. Platform

- macOS 14.2 or later, Apple Silicon and Intel. The app must run on a Mac with 8 GB of RAM.
- Windows, Linux, and the web are not in v1.

## 4. Stack

| Part | Choice |
|---|---|
| App shell | Tauri 2 |
| Backend | Rust (stable, installed with `rustup`) |
| Frontend | React, TypeScript, Vite |
| Notes editor | Tiptap |
| Database | SQLite through `rusqlite` (feature `bundled`), one file in the app data folder |
| Secrets | macOS Keychain through the `keyring` crate |
| Frontend tests | Vitest and Testing Library |
| Backend tests | `cargo test` |
| Package manager | pnpm |

The backend does all audio, all file and database access, all network calls, and all key access. The frontend never sees an API key.

## 5. Architecture

```
React screens ──invoke()──▶ Tauri commands ──▶ MeetingSession
      ▲                                          │
      └──────── Tauri events ◀───────────────────┤
                                                 ▼
 mic source ─┐                          Chunker (VAD) ─▶ Transcriber ─▶ SQLite
 system tap ─┴─▶ resample 16 kHz mono ─▶  (per stream)    (local or cloud)
                                                 │
 Stop ─▶ NotesWriter ─▶ OpenAI-compatible client ─▶ SQLite ─▶ event to UI
```

- Two audio streams stay separate: "You" (microphone) and "Others" (system audio). The speaker label of a transcript line comes from its stream.
- Morsel saves the audio of each meeting on the Mac, in `<app data>/audio/<meeting id>/`, and the user can delete it (changed on 2026-11-10, see `2026-11-10-transcribe-timing-design.md`). Audio never leaves the Mac in local mode.

## 6. Audio

- Microphone: `cpal`, default input device.
- System audio: a Core Audio process tap (`AudioHardwareCreateProcessTap` with a global `CATapDescription`, through the `objc2-core-audio` crates). It needs macOS 14.2 and the `NSAudioCaptureUsageDescription` key in `Info.plist`.
- Both streams are resampled to 16 kHz mono `f32` with `rubato`.
- Each stream sends a level value (RMS, 0.0 to 1.0) to the UI 10 times each second, for the "You" and "Others" bars.
- If the default input or output device changes during a meeting, the source restarts on the new device. The meeting continues.

## 7. Chunks and transcription

- A chunker cuts each stream into chunks of 10 to 30 seconds. It cuts at the first silence of 400 ms or more after 10 seconds, and it always cuts at 30 seconds. Silence is RMS below 0.01 over 30 ms frames.
- A chunk that is all silence is dropped. It is not sent to a transcriber.
- 10 seconds is the minimum because Groq bills each request for at least 10 seconds.
- `Transcriber` has two implementations:
  - **Local (default):** Whisper large-v3-turbo, int8, through `sherpa-onnx` (Parakeet was removed on 2026-11-10). Morsel downloads the model on first run, shows the progress, and checks a pinned SHA-256.
  - **Cloud:** any OpenAI-compatible `POST /audio/transcriptions`. The suggested default is Groq, base URL `https://api.groq.com/openai/v1`, model `whisper-large-v3-turbo` (about $0.04 for each hour).
- The user sets the meeting language in Settings. The default is "auto".

## 8. Notes

- One OpenAI-compatible client covers OpenRouter, Groq, OpenAI, and local servers. The user sets a base URL, a key, and a model.
- There are two models:
  - **Notes model:** writes the full notes.
  - **Quick model:** writes meeting titles and answers short questions.
- Input to the notes model: the template (sections and instructions), the user's rough notes, and the transcript with speaker labels and times.
- Output: Markdown with one `##` heading for each template section. Action items use `- [ ] Owner: task`.
- The model must keep each line of the user's rough notes word for word. The finished note shows a line in dark text when it matches a line of the rough notes (case and outer spaces ignored). All other lines show in grey as Morsel's words.
- If the transcript is more than 100,000 tokens (estimate: characters ÷ 4), Morsel splits it into parts, makes a summary of each part, and writes the notes from the summaries.
- "Cancel and keep my notes" stops the request. The meeting keeps the rough notes and the transcript, and gets the status `notes_cancelled`.
- "Write again" writes the notes again with the same or a different template.
- Morsel saves the token counts of each request. Settings shows the tokens of the last 30 days, and the cost only when the provider returns a cost.

## 9. Templates

v1 has five built-in templates: General, 1:1, Sales call, Stand-up, Product sync. The user can edit the sections and the AI instructions, and can create a new template. A built-in template can be reset.

## 10. Ask Morsel

- The user asks a question across all meetings, or across a time range.
- Morsel finds the 8 best meetings with SQLite FTS5 over notes and transcripts. Then the quick model answers from those meetings only.
- Each answer lists the meetings that it comes from, as links.
- On the meeting screen, "Ask about this meeting" uses only the current meeting.

## 11. Calendar, reminder, shortcut

- The calendar link uses the user's own Composio account (REST API from Rust). The user connects Google Calendar or Outlook. The Tauri deep-link plugin brings the user back to Morsel after the login.
- The calendar is optional. Morsel works without it.
- One minute before a calendar meeting, Morsel shows a reminder with "Start notes". The user can turn the reminder off.
- Auto-start is off, and v1 has no auto-start setting. Recording a call needs the consent of the other people, so the user starts each recording.
- A global shortcut starts a note for a meeting that is not in the calendar.

## 12. Screens

| Number | Screen | Canvas board |
|---|---|---|
| 1.1 | Welcome and permissions | Onboard-Welcome |
| 1.2 | AI setup | Onboard-AI |
| 1.3 | Calendar | Onboard-Calendar |
| 2.1 | Home | Main |
| 2.2 | Meeting reminder | Reminder |
| 2.3 | Meeting: meeting list, notepad, transcript on the right with Hide / Show (`⌘T`), Ask box | Meeting-C |
| 3.1 | Writing notes | Generating |
| 3.2 | Finished note | Note |
| 4.1 | All notes | Notes |
| 4.2 | Ask Morsel | Ask |
| 4.3 | Templates | Templates |
| 5.1 | Settings: models and transcription | Settings-Models |
| 5.2 | Settings: calendar and privacy | Settings-Privacy |

- The minimum window size is 1100 × 700 px, so the three columns of the meeting screen always fit.
- Colors and fonts come from the canvas: background `#EEEFEA`, surface `#FAFAF7`, text `#22261F`, muted text `#5D6354`, border `#D5D8CD`, recording `#A2513C`, accent dot `#D9826A`; EB Garamond for titles and notes, Inter for the UI.
- Buttons and other controls never shrink or wrap their labels. Long text truncates with an ellipsis.

## 13. Privacy and security

- API keys stay in the Keychain (service `app.morsel`). They never go to the frontend, to a log, or to the database.
- Tauri capabilities give the main window only the Morsel commands and the events it needs.
- No audio on disk (section 5).
- "Export all data" writes every note as Markdown to a folder that the user selects. "Delete all data" deletes the database and the keys, after the warning "You cannot undo this."
- Morsel sends no analytics.

## 14. Errors

| Condition | Behavior |
|---|---|
| Microphone permission denied | The meeting does not start. Morsel shows how to open System Settings › Privacy › Microphone. |
| System-audio permission denied, or macOS older than 14.2 | The meeting starts with the microphone only. A warning says "Others" is not recorded. |
| Transcriber error on one chunk | Morsel retries the chunk 2 times with a wait of 2 s and 4 s. Then the transcript shows "[missing 0:12–0:31]" and the meeting continues. |
| Notes request fails (key, network, limit) | The meeting keeps the rough notes and the transcript. The screen shows the error and "Try again". |
| App quits or crashes during a meeting | Transcript lines and rough notes are already saved. On the next start, the meeting gets the status `interrupted`, and the user can write the notes. |

## 15. Not in v1

Sync, sharing links, speaker names inside "Others", Windows, Linux, the web, a mobile app, auto-start, and a Morsel server.

## 16. Delivery

| Plan | Content |
|---|---|
| 1. Core loop | Project setup, storage, keys, audio, transcription, the meeting screen, notes, the finished note, a basic Home, and Settings 5.1 |
| 2. Library | All notes, Ask Morsel, Templates, Settings 5.2, export and delete |
| 3. First run and calendar | Screens 1.1 to 1.3, Composio, the reminder, the global shortcut |
| 4. Release | Universal build, signing, notarization, updates |

Each plan starts after the previous plan works on a real meeting.

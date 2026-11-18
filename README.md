# Morsel

Morsel is a meeting-notes app for macOS. It records a meeting, writes the transcript on the Mac, and turns your rough notes into finished notes. You can then ask questions about all your meetings with Ask.

Meeting content is private. By default, the transcript, the notes and the Ask answers stay on the Mac. You can select a cloud model in Settings.

## What Morsel does

- **Records** your microphone and the sound of the other people in the call.
- **Transcribes** on the Mac with Whisper large-v3-turbo, in English and Tagalog, during the meeting or after it.
- **Writes notes** from the transcript and your rough notes, with a template that you select.
- **Answers questions** with Ask. Ask finds the best parts of all meetings by keywords and by meaning, and names the meetings that it used.
- **Reminds you** of meetings from Google Calendar or Outlook, through Composio.
- **Updates itself** when a new release is published.

## Models

Morsel downloads each local model one time, from the app, and checks its SHA-256.

| Model | Job | Size |
| --- | --- | --- |
| Whisper large-v3-turbo (int8) | Transcription | about 1 GB |
| MiniCPM-V-4 (Q6_K) | Notes and Ask answers | 2.96 GB |
| EmbeddingGemma 2 (Q8_0) | Search for Ask | 310 MB |

The cloud options are OpenRouter for notes and Ask, and Groq for transcription. Each cloud option needs your own API key.

## Requirements

- macOS 14.2 or later. An Apple Silicon Mac with 8 GB of memory or more is the target.
- To build: Rust (stable), Node.js, pnpm, and the Xcode command line tools.

## Develop

```sh
pnpm install
pnpm tauri dev
```

## Test

```sh
pnpm test                                  # frontend tests (Vitest)
cd src-tauri && cargo test --lib           # backend tests
cd src-tauri && cargo clippy --all-targets # backend lint
```

Some backend tests need the real models. These tests are ignored by default. Run them with `cargo test --lib -- --ignored`.

## Build

```sh
NODE_OPTIONS=--max-old-space-size=2048 pnpm tauri build --bundles app
```

The app is in `src-tauri/target/release/bundle/macos/Morsel.app`. To publish a signed release, read [docs/RELEASE.md](docs/RELEASE.md).

## Where Morsel keeps your data

| Data | Place |
| --- | --- |
| Meetings, transcripts, notes, search index | `~/Library/Application Support/app.morsel/morsel.db` |
| Meeting recordings | `~/Library/Application Support/app.morsel/audio/` |
| Local models | `~/Library/Application Support/app.morsel/models/` |
| API keys | `~/Library/Application Support/app.morsel/secrets.json` (owner-only file) |

## Project layout

| Folder | Content |
| --- | --- |
| `src/` | The React frontend |
| `src-tauri/` | The Rust backend: audio, transcription, notes, search, calendar |
| `docs/` | The release steps, the design specs and the build plans |

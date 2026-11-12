# Local notes, local Ask with hybrid search, and spoken answers

Date: 2026-11-10. Branch: `feat/local-tagalog`.

## Intent

Meeting content is private. Morsel must write notes and answer Ask questions with no meeting text leaving the Mac. A user can still choose the cloud model. The Ask agent also speaks its answer.

The owner said:
- Privacy is the main reason. Both Local and Cloud stay available.
- Morsel runs the local model itself. Users install nothing else.
- The target is an 8 GB Apple Silicon Mac.
- Notes that take 1–3 minutes after a 1-hour meeting are acceptable.
- Transcription does not change: local Whisper large-v3-turbo stays.
- Note model: `openbmb/MiniCPM-V-4-gguf`, quantization `Q6_K` (2.96 GB).
- Embedding model: `google/embeddinggemma-2` (GGUF from `unsloth/embeddinggemma-2-GGUF`).
- Text-to-speech model: `Zyphra/ZONOS3-Mini`. The Ask agent speaks the answer to a sent message.

## Goals

1. A **Notes model** switch in Settings: Local or Cloud. One switch covers note writing and Ask.
2. Local note writing with MiniCPM-V-4 that works on an 8 GB Mac.
3. Hybrid search for Ask (keywords plus embeddings), in both modes. The answer names its meetings, as now.
4. Spoken Ask answers with ZONOS3-Mini, sentence by sentence, with stop and play-again controls.
5. In-app model downloads with progress and checksum checks, as for Whisper now.

## Non-goals

- No change to transcription.
- No image input. MiniCPM-V-4 loads without its vision part (`mmproj`).
- No voice cloning and no voice choice in this version.
- No Ollama or other external model server.
- No cloud text-to-speech.

## Anti-goals (a failure even if it works)

- Meeting text leaves the Mac in Local mode. This includes embeddings, search, logs and crash reports.
- The Mac swaps or freezes on 8 GB because two large models are loaded at the same time.
- A spoken answer starts with no way to stop it.

## Constraints

- Apple Silicon, 8 GB, macOS. Metal through llama.cpp.
- Rust backend (Tauri 2), React frontend. Keep the existing `SecretStore`, settings, and `model_download.rs` patterns.
- Local context budget: about 8K–16K tokens. Do not send full transcripts to the local model.
- **Caution, license:** the MiniCPM-V-4 weights use the MiniCPM Model License. Commercial use needs a registration with OpenBMB. The owner must clear this before a public release.

## Design

### 1. Model runtime

- Use llama.cpp through the Rust crate `llama-cpp-2`, with Metal on. Use a version that bundles llama.cpp from 2026-10-06 or later, because EmbeddingGemma 2 support is in PR #30054.
- Add a `ChatModel` trait with two implementations:
  - `CloudChat`: the current `llm::chat` (OpenAI-compatible HTTP).
  - `LocalChat`: llama.cpp with MiniCPM-V-4 Q6_K.
- The trait supports streamed output, because spoken answers need each sentence as soon as it is complete. `CloudChat` gets streaming through `"stream": true` (SSE).
- `notes/writer.rs` and `ask.rs` call the trait, not `llm::chat` directly.

### 2. Memory rules for 8 GB

- One **model manager** in the backend owns the loaded models, and it is the only part that loads or frees them.
- Morsel never keeps Whisper and the note model loaded at the same time. When a transcription ends, the manager frees Whisper and then loads the note model.
- The note model and ZONOS3-Mini can be loaded together, when Whisper is not loaded.
- The manager frees the note model and ZONOS3-Mini after 5 minutes with no use.
- The embedding model (text part only, about 270M) loads for indexing and for each search, and the manager frees it after 1 minute with no use.
- **While Whisper is loaded** (a Live recording, or an "after" transcription that still runs), local Ask shows "Ask is available when the transcription ends." Cloud Ask still works then.

### 3. Local note writing

- A short transcript (within the local budget) goes to the model in one step, as now.
- A long transcript goes in two steps:
  1. Cut it into parts of about 3K tokens, and summarize each part.
  2. Write the final notes, from the template, from the part summaries and the user's own notes.
- The meeting status shows the progress, for example "Writing notes (2 of 5)".
- Cloud mode keeps the current one-step method.
- The note request ends with the headings again, because a small model forgets the format after a long transcript.
- A loop guard stops the local output when one unit (8 to 1,000 bytes, with a letter or a digit) repeats 3 times. The output keeps the first copy.

### 4. Hybrid search for Ask

- **Indexing:** after a meeting's notes are saved, cut the transcript into parts of about 400 tokens, with an overlap of about 50 tokens. The written notes are extra parts. Compute an embedding for each part on the Mac, in both modes.
- **Embedding model:** EmbeddingGemma 2, text only, 768 dimensions. Use the required labels:
  - question: `task: search result | query: {question}`
  - part: `title: {meeting title} | text: {part}`
- **Storage:** a new table `chunks` (meeting id, part number, start and end time, text, embedding as a BLOB of 768 `f32` values). Add a new FTS5 table on the part text. A new migration file adds them.
- **Search:** run keyword search (FTS5) and embedding search (cosine similarity, a plain scan in Rust) on the parts. Merge the two lists with reciprocal rank fusion (k = 60). Keep the time range filter (All, Week, Month).
- **Context:** send the best 10–15 parts (about 6K tokens) to the model, grouped by meeting. The answer keeps the current source list.
- **Old meetings:** at the first run after the update, index all saved meetings in the background, and show the progress in Settings.
- When a meeting is deleted, delete its parts.

### 5. Spoken answers

- ZONOS3-Mini runs on Zyphra's C++/ggml runtime with Metal. This is a second native library, beside llama.cpp. Bundle it and call it from Rust.
- As the answer streams, cut it into sentences. Speak each sentence as soon as it is complete. The first audio comes about 1 second after the first sentence.
- Each answer has a **Stop** button and a **Play again** button. The Stop button also stops the sentences that are still queued.
- Settings has a **Speak answers** switch, on by default. *(Builder's call; the owner did not choose.)*
- One built-in Zyphra voice.
- Speech is always local, also in Cloud mode.

### 6. Settings and downloads

- New installs start with Local. An existing install that has a notes key keeps Cloud, so an update changes nothing for that user.

- Settings → Models: Notes model (Local or Cloud). Local shows the model names, the download sizes, and the download state of each model.
- The downloads are the note model (2.96 GB), the embedding model, and ZONOS3-Mini. Each download shows progress and is checked against a SHA-256 value, as `model_download.rs` does for Whisper.
- The first-run screens offer the local download, with the total size stated before it starts.

## Build order

This is three projects that share the model manager. Each gets its own plan, in this order:

1. **Local runtime and local notes:** sections 1, 2, 3, and the note model part of section 6.
2. **Hybrid search for Ask:** section 4 and the embedding download.
3. **Spoken answers:** section 5 and the ZONOS3-Mini download. Start only after spike 2 passes.

## Risks and checks before the full build

1. **Spike: note quality and speed.** Run MiniCPM-V-4 Q6_K on 2 real saved Taglish meetings on an 8 GB Mac. Record the tokens per second, the peak memory, and the note quality. Compare with Gemma 4 E2B on the same meetings. If MiniCPM-V-4 is clearly worse, bring the results to the owner.
2. **Spike: Tagalog speech.** ZONOS3-Mini does not list Tagalog. Speak 4 real Taglish answers. If they sound wrong, bring the samples to the owner before the build of section 5.
   **Result (2026-11-11):** The owner keeps MiniCPM-V-4. The test used a written Taglish sample, because the database has no real Taglish meeting. It ran on a 16 GB Mac, so it shows no swap behavior.

   | | MiniCPM-V-4 Q6_K | Gemma 4 E2B QAT Q4_0 |
   |---|---|---|
   | Short meeting (614 tokens) | 10.2 s. The action items loop. One owner is wrong. | 24.5 s. All facts are correct. |
   | Long meeting (9,221 tokens, two-step) | 267 s. The note is looped text. | 136 s. Usable, one wrong number. |
   | Max resident memory | 2.6 to 3.3 GB | about 2.7 GB |
   | License | Commercial use needs a registration | Apache 2.0 |

   - The reference llama.cpp loops in the same way with MiniCPM-V-4, so the runtime code is not the cause.
   - Without a format reminder after the transcript, MiniCPM-V-4 ignores the headings. The note request now ends with that reminder.
   - A repetition penalty of 1.1 made the loops worse.
   - Gemma 4 E2B does not run in `llama-cpp-2` 0.1.161 without a Jinja template engine, such as `minijinja`.
3. **Check:** the `llama-cpp-2` version in use loads EmbeddingGemma 2. If it does not, use the newest crate version, or pin a llama.cpp revision after PR #30054.

## The builder's calls (the owner did not decide these)

- One switch for notes and Ask, not two.
- Cloud Ask also uses the new hybrid search.
- Embeddings run locally in both modes.
- Sentence-by-sentence speech, Stop and Play again buttons, and **Speak answers** on by default.
- The memory rules in section 2, including "Ask is available when the recording ends" for local Ask during a Live recording.
- The part sizes (3K tokens for notes, 400 tokens for search), the overlap, k = 60, and 10–15 parts for each answer.
- 768 embedding dimensions, stored in SQLite with a plain scan, no vector database.
- Background indexing of old meetings.
- Speech is local in both modes.
- New installs start with Local. Existing installs with a notes key keep Cloud.
- The build order: runtime and notes, then search, then speech.

## Left to the builder

- The exact `llama-cpp-2` version and the ZONOS3-Mini runtime build steps.
- The prompt text for the two-step notes and for Ask.
- The sentence cutter and the audio output code.
- The download URLs and the SHA-256 values.

## Sources

- MiniCPM-V-4 GGUF: https://huggingface.co/openbmb/MiniCPM-V-4-gguf
- EmbeddingGemma 2: https://huggingface.co/google/embeddinggemma-2
- EmbeddingGemma 2 GGUF guide (Unsloth): https://unsloth.ai/docs/models/embeddinggemma-2
- llama.cpp PR #30054: https://github.com/ggml-org/llama.cpp/pull/30054
- ZONOS3-Mini: https://huggingface.co/Zyphra/ZONOS3-Mini

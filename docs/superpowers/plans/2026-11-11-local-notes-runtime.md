# Local notes runtime Implementation Plan (part 1 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Morsel writes notes and answers Ask with MiniCPM-V-4 Q6_K on the Mac, with a Local or Cloud switch.

**Architecture:** A `ChatModel` trait has two implementations: `CloudChat` (the current HTTP `llm::chat`) and `LocalChat` (llama.cpp through `llama-cpp-2`). A `ModelManager` owns the one loaded local model, frees Whisper before it loads, and frees the model after 5 minutes with no use. The notes writer and Ask call the trait.

**Tech Stack:** Rust (Tauri 2), `llama-cpp-2 = 0.1.161` (Metal), React + Vitest.

**Spec:** `docs/superpowers/specs/2026-11-10-local-notes-ask-voice-design.md` (sections 1, 2, 3, 6).

## Global Constraints

- Note model file: `https://huggingface.co/openbmb/MiniCPM-V-4-gguf/resolve/main/ggml-model-Q6_K.gguf`, size `2960134016`, SHA-256 `fc78a557d5bcc3cb21f99667f97fe3e0381fc129d586d1532fffe1926ec53bd1`. Folder `models/minicpm-v-4-q6k`. No `mmproj`.
- Local context: `n_ctx = 8192`. Local long-transcript limit `6_000` tokens, local part size `3_000` tokens. Local Ask budget `6_000` tokens.
- Never keep Whisper and the note model loaded at the same time. Free the note model after 300 s with no use.
- While Whisper is loaded, local Ask fails with "Ask is available when the transcription ends."
- New installs start with Local. An install that has a notes key and no saved `notesMode` keeps Cloud.
- In Local mode, no meeting text goes over the network.

## Review Focus

1. A transcript longer than 6,000 tokens in Local mode: the two-step method runs, and no prompt goes over `n_ctx`.
2. A prompt that is still too long (one huge part): Morsel cuts the prompt and does not crash.
3. Local mode with no downloaded model: Write notes fails with "download the notes model in Settings first".
4. A new recording starts while the note model is loaded: Morsel frees the note model before Whisper loads.
5. Cancel while the local model generates: the job stops in under 1 s, and the rough notes stay.

---

### Task 1: Local engine and model files

**Files:**
- Create: `src-tauri/src/local_llm.rs`
- Modify: `src-tauri/src/transcribe/model_download.rs` (add `notes_model_files()`, `notes_model_dir()`, `notes_model_ready()`), `src-tauri/src/lib.rs` (module), `src-tauri/Cargo.toml`

**Interfaces:**
- Produces: `LocalLlm::load(path: &Path) -> Result<LocalLlm>`, `LocalLlm::complete(&self, messages: &[ChatMessage], max_tokens: u32, cancel: &AtomicBool) -> Result<ChatResult>` (blocking), `fit_prompt(tokens: Vec<LlamaToken>, n_ctx: u32, max_tokens: u32) -> Vec<LlamaToken>`.

- [ ] Test `fit_prompt_keeps_the_end_of_a_long_prompt`: 10_000 tokens, n_ctx 8192, max 1024 → length 7168 and the last token is kept.
- [ ] Test `notes_model_files_pin_the_q6k_file`: one file, the size and SHA-256 from Global Constraints.
- [ ] Implement. Use the model chat template (`chat_template(None)` + `apply_chat_template`), greedy-free sampling `temp(0.3)`, `top_p(0.9, 1)`, `dist(seed)`. Stop on `is_eog` or `cancel`. Prompt tokens and output tokens go into `ChatResult`, `cost: None`.
- [ ] Ignored test `local_llm_answers_with_the_real_model` (runs with `--include-ignored local_llm`).
- [ ] Check: `cargo test --lib local_llm model_download`.

### Task 2: Spike — speed and memory

**Files:** none in the product. Script in the scratchpad.

- [ ] Download the Q6_K file with `ensure_files`. Run the ignored test on one saved meeting transcript through the two-step method.
- [ ] Record tokens per second, peak memory (`/usr/bin/time -l`), and the note. Report to the owner. Stop if the peak is over 4.5 GB.

### Task 3: `ChatModel` trait and local budgets

**Files:**
- Create: `src-tauri/src/chat_model.rs`
- Modify: `src-tauri/src/notes/writer.rs`, `src-tauri/src/ask.rs`, `src-tauri/src/notes/prompt.rs`

**Interfaces:**
- Produces: `#[async_trait] trait ChatModel: Send + Sync { async fn chat(&self, messages: &[ChatMessage], cancel: &CancellationToken) -> Result<ChatResult>; fn name(&self) -> &str; fn budget(&self) -> Budget; }`, `struct Budget { long_transcript: usize, part: usize, ask: usize }`, `Budget::CLOUD` (80_000 / current part / 100_000), `Budget::LOCAL` (6_000 / 3_000 / 6_000), `CloudChat::new(http, LlmConfig)`.
- `NotesContext.notes` and `.quick` become `Arc<dyn ChatModel>`.

- [ ] Test `local_budget_splits_a_7000_token_transcript`: a fake `ChatModel` with `Budget::LOCAL` gets part summaries and then the final call.
- [ ] Test `ask_uses_the_model_budget`: a fake local model gets at most 6_000 × 4 characters of meeting text.
- [ ] Implement. Existing writer and Ask tests still pass with `CloudChat`.
- [ ] Check: `cargo test --lib notes ask chat_model`.

### Task 4: Model manager and memory rules

**Files:**
- Create: `src-tauri/src/model_manager.rs`
- Modify: `src-tauri/src/commands.rs` (`RealEngines`: `unload_whisper()`, `whisper_loaded()`; free the note model before Whisper loads)

**Interfaces:**
- Produces: `ModelManager::new(loader: Box<dyn Fn(&Path) -> Result<Arc<LocalLlm>>>, idle: Duration)`, `ModelManager::get(&self, path: &Path) -> Result<Arc<LocalLlm>>`, `ModelManager::unload(&self)`, `LocalChat::new(manager, path, whisper_loaded: Arc<dyn Fn() -> bool>)`.

- [ ] Test `manager_loads_once_and_frees_after_idle` (tokio paused time, 300 s).
- [ ] Test `local_ask_waits_while_whisper_is_loaded`: error text "Ask is available when the transcription ends."
- [ ] Test `starting_whisper_frees_the_note_model`.
- [ ] Implement. `LocalChat::chat` runs `complete` in `spawn_blocking` and sets the `AtomicBool` when the token is cancelled.
- [ ] Check: `cargo test --lib model_manager commands`.

### Task 5: Settings and commands

**Files:**
- Modify: `src-tauri/src/settings.rs` (`notes_mode: Option<NotesMode>`), `src-tauri/src/commands.rs` (`notes_mode()`, `write_notes`, `ask`, `download_notes_model`, `notes_model_status`), `src-tauri/src/lib.rs` (register commands), `src/lib/api.ts`

**Interfaces:**
- Produces: commands `download_notes_model` (event `notes-model://progress` `{received, total}`) and `notes_model_status -> { ready: bool, bytes: u64 }`; `AppState::notes_mode(&self) -> Result<NotesMode>`.

- [ ] Test `new_install_uses_local`, `install_with_a_notes_key_keeps_cloud`, `saved_mode_wins`.
- [ ] Test `local_write_notes_without_the_model_fails_with_help`: "download the notes model in Settings first".
- [ ] Implement. In Local mode, write notes needs no key and no model name.
- [ ] Check: `cargo test --lib settings commands`.

### Task 6: Settings screen

**Files:**
- Modify: `src/screens/SettingsModelsScreen.tsx`, `src/screens/screens.test.tsx`

- [ ] Test `notes_mode_switch_saves_local`, `local_mode_shows_the_download_and_size` ("MiniCPM-V-4 · 2.96 GB"), `cloud_mode_shows_the_key_fields`.
- [ ] Implement. Local hides the notes URL, key and model fields. The download shows progress.
- [ ] Check: `npx vitest run src/screens`, then the full gate: `npx tsc --noEmit`, `npx vitest run`, `cargo clippy --all-targets`, `cargo test --lib`.

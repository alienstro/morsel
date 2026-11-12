# Hybrid Search for Ask Implementation Plan (part 2 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ask finds the best parts of all meetings with keyword search plus EmbeddingGemma 2 vectors, and sends only those parts to the model.

**Architecture:** A new `search` module cuts each meeting into parts, stores them in a `chunks` table with an FTS5 index, and embeds them with a local llama.cpp embedder that the model manager frees after 1 minute. Ask merges the FTS5 list and the cosine list with reciprocal rank fusion and builds its context from the best parts. Without the search model, Ask uses the keyword list only.

**Tech Stack:** Rust, `llama-cpp-2` (git, pinned), rusqlite + FTS5, React + Vitest.

**Spec:** `docs/superpowers/specs/2026-11-10-local-notes-ask-voice-design.md` (sections 1, 2, 4, 6)

## Global Constraints

- Embedding model: EmbeddingGemma 2, `unsloth/embeddinggemma-2-GGUF`, file `embeddinggemma-2-Q8_0.gguf`, size `309855520`, SHA-256 `6f1bd4ac6c5df7444f9cca7ca36cafe6cfa34cd6f49fefb1e0b4be8143aed8bc`, output 768 dimensions.
- Labels: question `task: search result | query: {question}`; part `title: {meeting title} | text: {part}`.
- Parts: about 400 tokens, overlap about 50 tokens (tokens = characters ÷ 4).
- Fusion: reciprocal rank fusion with k = 60. Context: the best 15 parts, at most the model budget `ask` tokens, grouped by meeting.
- The manager frees the embedding model after 1 minute with no use.
- `llama-cpp-2` must bundle llama.cpp with PR #30054 (merged 2026-10-06). Crate 0.1.161 does not. Pin `utilityai/llama-cpp-rs` git rev `955aa2f72463c2535cc5ab894d5d52f27a38cde2` (main on 2026-11-11, llama.cpp v0.9.0) until a crates.io release has it.
- Keep the time range filter (All, Week, Month) and the current source list of an answer.

## Review Focus

1. A question that is only FTS5 keywords or punctuation (`fts_query` gives `None`): Ask must still search with the vector list, and not answer "No meeting matches" when the model is ready. Test in Task 5.
2. The search model is not downloaded: Ask and indexing must work with keywords only, with no error. Tests in Tasks 5 and 6.
3. A deleted meeting: its parts and their FTS5 rows must go too. Test in Task 2.
4. One transcript line longer than 400 tokens: the splitter must cut inside the line, so no part passes the limit. Test in Task 3.
5. A meeting with notes but no transcript: its notes must still be parts. Test in Task 3.

---

### Task 1: Pin a `llama-cpp-2` that loads EmbeddingGemma 2

**Files:**
- Modify: `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/src/local_llm.rs` (only if the API changed)

**Interfaces:**
- Produces: the same `LocalLlm` API as today, on the new crate.

- [ ] **Step 1:** Set `llama-cpp-2 = { git = "https://github.com/utilityai/llama-cpp-rs", rev = "955aa2f72463c2535cc5ab894d5d52f27a38cde2" }`, with a comment: go back to crates.io at the first release after 0.1.161.
- [ ] **Step 2:** Run `cargo build --lib`. Fix only API breaks, one at a time.
- [ ] **Step 3:** Run `cargo test --lib` (expect 201 passed) and `cargo test --lib local_llm_answers_with_the_real_model -- --ignored` (expect PASS with the real MiniCPM-V-4 file).
- [ ] **Step 4:** Commit `chore: pin the llama.cpp bindings to a version that knows EmbeddingGemma 2`.

### Task 2: Parts table, FTS5 index and store functions

**Files:**
- Create: `src-tauri/src/db/migrations_4.sql`, `src-tauri/src/db/parts.rs`
- Modify: `src-tauri/src/db/mod.rs` (add the migration and `pub mod parts`), `src-tauri/src/db/tests.rs`

**Interfaces:**
- Produces in `db::parts`:
  - `pub struct NewPart { pub start_ms: Option<u64>, pub end_ms: Option<u64>, pub text: String }`
  - `pub fn replace_parts(conn: &Connection, meeting: MeetingId, parts: &[NewPart]) -> Result<()>` (deletes the old parts of the meeting first)
  - `pub fn parts_without_embedding(conn: &Connection, limit: usize) -> Result<Vec<(i64, MeetingId, String)>>` (part id, meeting, text)
  - `pub fn set_embedding(conn: &Connection, part: i64, vector: &[f32]) -> Result<()>` (BLOB of little-endian `f32`)
  - `pub fn meetings_without_parts(conn: &Connection) -> Result<Vec<MeetingId>>` (only status `done`)

Schema: `chunks(id INTEGER PRIMARY KEY, meeting_id INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE, part INTEGER NOT NULL, start_ms INTEGER, end_ms INTEGER, text TEXT NOT NULL, embedding BLOB)`, `chunks_fts` FTS5 with `content='chunks', content_rowid='id'`, and insert and delete triggers that keep `chunks_fts` in step.

- [ ] **Step 1:** Write tests `replace_parts_keeps_only_the_new_parts`, `set_embedding_round_trips_768_floats`, `deleting_a_meeting_deletes_its_parts_and_their_search_rows` (after `delete_meeting`, `SELECT count(*) FROM chunks_fts WHERE chunks_fts MATCH 'budget'` is 0), `meetings_without_parts_lists_only_done_meetings`.
- [ ] **Step 2:** Run `cargo test --lib db::` and see them fail.
- [ ] **Step 3:** Write the migration and the four functions.
- [ ] **Step 4:** Run `cargo test --lib db::` (all pass).
- [ ] **Step 5:** Commit `feat: store meeting parts with a keyword index`.

### Task 3: Cut a meeting into parts

**Files:**
- Create: `src-tauri/src/search/mod.rs`, `src-tauri/src/search/parts.rs`
- Modify: `src-tauri/src/lib.rs` (`pub mod search`)

**Interfaces:**
- Consumes: `NewPart` (Task 2), `TranscriptLine`, `notes::prompt::transcript_text`.
- Produces: `pub const PART_TOKENS: usize = 400; pub const OVERLAP_TOKENS: usize = 50;` and `pub fn meeting_parts(lines: &[TranscriptLine], note: Option<&str>) -> Vec<NewPart>` (transcript parts in time order, then note parts with `None` times).

- [ ] **Step 1:** Write tests `parts_stay_under_400_tokens_and_overlap_by_about_50`, `a_line_longer_than_a_part_is_cut_inside`, `note_parts_follow_the_transcript_parts_without_times`, `a_meeting_with_only_notes_still_has_parts`, `parts_keep_the_start_and_end_time_of_their_lines`.
- [ ] **Step 2:** Run `cargo test --lib search::parts` and see them fail.
- [ ] **Step 3:** Implement `meeting_parts`: walk the `[m:ss] Speaker: text` lines, close a part at 400 tokens, start the next part with the last lines that fit in 50 tokens; cut a single long line at a character boundary.
- [ ] **Step 4:** Run `cargo test --lib search::parts` (all pass).
- [ ] **Step 5:** Commit `feat: cut each meeting into overlapping parts for search`.

### Task 4: Local embedder and its download

**Files:**
- Create: `src-tauri/src/search/embed.rs`
- Modify: `src-tauri/src/model_manager.rs` (add `pub const EMBED_IDLE: Duration = Duration::from_secs(60);`)

**Interfaces:**
- Produces:
  - `pub fn search_model_files() -> Vec<ModelFile>` and `pub fn search_model_dir(root: &Path) -> PathBuf` (folder `embeddinggemma-2-q8`), with the values in Global Constraints.
  - `pub fn query_text(question: &str) -> String`, `pub fn part_text(title: &str, part: &str) -> String` (the two labels).
  - `pub struct LocalEmbedder` with `pub fn load(path: &Path) -> Result<Self>` and `pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>` (each vector L2-normalized, 768 long).
  - `pub fn cosine(a: &[f32], b: &[f32]) -> f32` (dot product of normalized vectors).

- [ ] **Step 1:** Write tests `search_model_files_pin_the_q8_file`, `labels_match_the_spec`, `cosine_of_a_vector_with_itself_is_one`, and the ignored real-model test `the_same_question_is_closer_to_its_answer_than_to_another_topic` (query "kailan ang launch?" is closer to "The launch moves to March 29." than to "Ana writes the FAQ.").
- [ ] **Step 2:** Run `cargo test --lib search::embed` and see them fail.
- [ ] **Step 3:** Implement with a llama.cpp context with embeddings on and mean pooling from the model (pooling type 1), one sequence per text.
- [ ] **Step 4:** Run `cargo test --lib search::embed` and the ignored test with the downloaded file (all pass, 768 dimensions).
- [ ] **Step 5:** Commit `feat: embed text on the Mac with EmbeddingGemma 2`.

### Task 5: Hybrid search with reciprocal rank fusion

**Files:**
- Create: `src-tauri/src/search/hybrid.rs`

**Interfaces:**
- Consumes: `db::library::fts_query`, `AskRange::since`, `cosine` (Task 4).
- Produces:
  - `pub const RRF_K: f64 = 60.0; pub const MAX_PARTS: usize = 15;`
  - `pub fn rrf(lists: &[Vec<i64>]) -> Vec<i64>` (part ids, best first)
  - `pub struct PartHit { pub id: i64, pub meeting: MeetingId, pub start_ms: Option<u64>, pub text: String }`
  - `pub fn search_parts(conn: &Connection, question: &str, query: Option<&[f32]>, range: AskRange, now_ms: i64) -> Result<Vec<PartHit>>`

- [ ] **Step 1:** Write tests `rrf_puts_a_part_found_by_both_lists_first`, `keyword_only_search_works_without_a_vector`, `vector_search_finds_a_part_with_no_shared_word`, `a_question_of_only_fts_keywords_still_uses_the_vector`, `the_week_range_skips_old_parts`, `search_returns_at_most_15_parts`.
- [ ] **Step 2:** Run `cargo test --lib search::hybrid` and see them fail.
- [ ] **Step 3:** Implement: FTS5 list by `bm25` (limit 50), cosine list over parts with an embedding in the range (plain scan, limit 50), `rrf`, then the first 15.
- [ ] **Step 4:** Run `cargo test --lib search::hybrid` (all pass).
- [ ] **Step 5:** Commit `feat: find meeting parts with keywords and meaning together`.

### Task 6: Index new and old meetings

**Files:**
- Create: `src-tauri/src/search/index.rs`
- Modify: `src-tauri/src/notes/writer.rs` (index after the note is saved), `src-tauri/src/events.rs` (`search://progress`)

**Interfaces:**
- Consumes: Tasks 2–4, `ModelManager<LocalEmbedder>`.
- Produces:
  - `pub trait Embed: Send + Sync { fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>; }` (`None` model = keyword only)
  - `pub fn index_meeting(db: &SharedDb, embed: Option<&dyn Embed>, meeting: MeetingId) -> Result<()>`
  - `pub fn index_all(db: &SharedDb, embed: Option<&dyn Embed>, sink: &dyn EventSink) -> Result<()>` (meetings without parts first, then parts without embedding; emits `search://progress` `{ done, total }`)

- [ ] **Step 1:** Write tests `index_meeting_stores_parts_with_embeddings`, `index_meeting_without_a_model_stores_parts_only`, `index_all_fills_missing_embeddings_when_the_model_arrives`, `index_all_reports_progress`, `a_saved_note_indexes_its_meeting` (writer test with `FakeChat`).
- [ ] **Step 2:** Run `cargo test --lib search::index notes::writer` and see them fail.
- [ ] **Step 3:** Implement; embed in batches of 16 and hold the DB lock only for each short write.
- [ ] **Step 4:** Run the same tests (all pass).
- [ ] **Step 5:** Commit `feat: index each meeting for search, and old meetings in the background`.

### Task 7: Ask answers from the best parts

**Files:**
- Modify: `src-tauri/src/ask.rs`, `src-tauri/src/commands.rs` (pass the embedder)

**Interfaces:**
- Consumes: `search_parts` (Task 5), `query_text` (Task 4).
- Produces: `ask(db, model, embed: Option<&dyn Embed>, question, range, meeting)`; the context groups hits by meeting as `Meeting: {title} ({date})` then `[m:ss] {text}` lines; `sources` list each meeting once, in rank order.

- [ ] **Step 1:** Write tests `ask_sends_only_the_found_parts_grouped_by_meeting`, `ask_lists_each_source_meeting_once`, `ask_without_the_search_model_uses_keywords`; update the old tests for the new signature.
- [ ] **Step 2:** Run `cargo test --lib ask::` and see the new tests fail.
- [ ] **Step 3:** Implement; a question about one meeting keeps today's path.
- [ ] **Step 4:** Run `cargo test --lib` (all pass).
- [ ] **Step 5:** Commit `feat: answer Ask from the best meeting parts`.

### Task 8: Search model download and start-up indexing (backend)

**Files:**
- Modify: `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `search_model_files`, `index_all` (Tasks 4, 6).
- Produces commands `search_model_status() -> bool` and `download_search_model()` (event `search-model://download` `{ received, total }`). `setup` runs `index_all` on a background thread at start-up, and the download runs it again when it ends.

- [ ] **Step 1:** Write tests `download_search_model_reports_progress` (wiremock) and `search_model_status_is_false_without_the_file`.
- [ ] **Step 2:** Run `cargo test --lib commands::` and see them fail.
- [ ] **Step 3:** Implement with `model_download` as the notes model does.
- [ ] **Step 4:** Run `cargo test --lib` (all pass).
- [ ] **Step 5:** Commit `feat: download the search model and index old meetings at start-up`.

### Task 9: Search model and index progress on the screens

**Files:**
- Modify: `src/lib/api.ts`, `src/screens/SettingsModelsScreen.tsx`, `src/screens/onboarding/AiStep.tsx`, `src/screens/screens.test.tsx`

**Interfaces:**
- Consumes: the commands and events of Task 8, and `search://progress` `{ done, total }`.
- UI: Settings → Models shows "Search model (EmbeddingGemma 2, 310 MB)" with its state and a download button in both modes, and "Search index: {done} of {total} meetings" while the index runs. The first-run local option says "3.27 GB in all" and downloads the notes model, then the search model.

- [ ] **Step 1:** Write tests `settings_shows_the_search_model_and_its_download`, `settings_shows_the_index_progress`, `ai_step_states_the_total_download_size`.
- [ ] **Step 2:** Run `pnpm exec vitest run src/screens` and see them fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `pnpm exec vitest run` and `pnpm exec tsc --noEmit` (all pass).
- [ ] **Step 5:** Commit `feat: show the search model and the index progress in Settings and the first run`.

### Final check

Run `/do-tests full`, then the real-model check: index the saved meetings and ask one question in Tagalog and one in English.

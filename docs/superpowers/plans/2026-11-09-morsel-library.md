# Morsel library implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The user finds, asks about, reuses and removes notes: All notes (4.1), Ask Morsel (4.2), Templates (4.3), Settings 5.2 privacy and data, export and delete.

**Architecture:** A second migration adds a full-text index (`meeting_search`, FTS5), template descriptions and section hints, a `people` column, and an `asks` table. New Rust modules hold the library queries, the template store and the ask engine. Thin Tauri commands expose them. Four React screens use them.

**Tech Stack:** Same as the core loop plan, plus SQLite FTS5 (in the `bundled` build) and `tauri-plugin-opener` (Show in Finder).

**Spec:** `docs/superpowers/specs/2026-11-08-morsel-v1-design.md` (sections 8, 9, 10, 12, 13). Screens: boards 2.1, 2.3, 4.1, 4.2, 4.3, 5.1 and 5.2 of the design canvas.

## Global constraints

- All constraints of `2026-11-08-morsel-core-loop.md` stay in force.
- Ask Morsel uses the 8 best meetings from FTS5 over notes and transcripts, then the quick model answers from those meetings only.
- Each answer lists the meetings that it comes from, as links.
- Five built-in templates: General, 1:1, Sales call, Stand-up, Product sync. A built-in template can be reset. A built-in template cannot be deleted.
- "Export all data" writes every note as Markdown to a folder that the user selects.
- "Delete all data" deletes all meetings, notes, asks, usage and settings, resets the templates, and deletes the keys, after the warning "You cannot undo this."
- Settings shows the tokens of the last 30 days, and the cost only when the provider returned a cost.
- The design shows a "Delete the audio" switch and an auto-start switch. The spec wins: Morsel never saves audio, and v1 has no auto-start. The privacy page shows the fixed line "Morsel never saves audio. It keeps only the text." and no switches for these two items.
- Confirmations use an in-app dialog, never `window.confirm`.

## Review focus

1. **Search text with FTS5 syntax** (`"`, `*`, `-`, `AND`, `:`): the search must not fail. Test `fts_query_escapes_operators` (Task 1).
2. **A template that meetings use is deleted:** the meetings move to General, and their notes stay. Test `delete_used_template_moves_meetings_to_general` (Task 2).
3. **Ask with no saved notes, or no match:** the answer says that no meeting matches, and no model call happens. Test `ask_with_no_meetings_makes_no_model_call` (Task 3).
4. **A very long meeting in Ask:** the context sent to the model stays under 100,000 tokens. Test `ask_context_stays_under_the_limit` (Task 3).
5. **Delete all data while a meeting records:** the command refuses. Test `delete_all_refuses_while_recording` (Task 4).

---

### Task 1: Library store and search index

**Files:**
- Create: `src-tauri/src/db/migrations_2.sql`, `src-tauri/src/db/library.rs`
- Modify: `src-tauri/src/db/mod.rs`, `src-tauri/src/db/meetings.rs`

**Interfaces:**
- Produces:
  - Migration 2: `ALTER TABLE templates ADD description TEXT NOT NULL DEFAULT ''`; `ALTER TABLE meetings ADD people TEXT NOT NULL DEFAULT '[]'`; `CREATE VIRTUAL TABLE meeting_search USING fts5(title, user_notes, note, transcript)` with `rowid` = meeting id; `CREATE TABLE asks(id, question, answer, sources_json, created_at)`; a backfill of `meeting_search` for the existing meetings.
  - `MeetingSummary` gains `ended_at: Option<i64>` and `people: Vec<String>`.
  - `library::reindex_meeting(conn, id) -> Result<()>`. `save_user_notes`, `save_note`, `set_title` and `set_status` call it.
  - `library::fts_query(text: &str) -> Option<String>`: each word becomes a quoted term, terms join with ` OR `; `None` when the text has no word.
  - `library::NotesFilter { query: Option<String>, template_id: Option<i64>, person: Option<String>, from: Option<i64>, to: Option<i64>, limit: u32, offset: u32 }` (serde camelCase).
  - `library::search_meetings(conn, &NotesFilter) -> Result<NotesPage>`, `NotesPage { items: Vec<MeetingSummary>, total: u32 }`. A query orders by `bm25`, no query orders by `started_at DESC`.
  - `library::list_people(conn) -> Result<Vec<String>>`, `library::count_meetings(conn) -> Result<u32>`, `library::delete_meeting(conn, id) -> Result<()>` (also removes the search row).
  - `library::usage_since(conn, since_ms) -> Result<UsageTotal>`, `UsageTotal { prompt_tokens: u64, completion_tokens: u64, cost: Option<f64> }`.

- [ ] **Step 1: Write the failing tests** in `library.rs` `mod tests`: `search_finds_words_in_note_and_transcript`, `search_without_query_lists_newest_first`, `filter_by_template_person_and_dates`, `fts_query_escapes_operators` (input `foo" OR * -bar:` gives `"foo" OR "bar"`), `delete_meeting_removes_lines_notes_and_search_row`, `usage_since_sums_tokens_and_cost_is_none_without_costs`, `migration_2_backfills_existing_meetings`.
- [ ] **Step 2: Run** `cargo test --manifest-path src-tauri/Cargo.toml db::` — the new tests fail.
- [ ] **Step 3: Write the migration and the module.** Add `M::up(include_str!("migrations_2.sql"))` to `MIGRATIONS`.
- [ ] **Step 4: Run** the same command — all pass.
- [ ] **Step 5: Commit** `feat: add a search index and library queries for notes`

### Task 2: Template store

**Files:**
- Create: `src-tauri/src/db/templates.rs`
- Modify: `src-tauri/src/db/meetings.rs` (move template code out), `src-tauri/src/notes/prompt.rs`, `src-tauri/src/db/mod.rs`

**Interfaces:**
- Produces:
  - `Section { name: String, hint: String }`. Serde reads an old string section as `{ name, hint: "" }`.
  - `Template { id, name, description, sections: Vec<Section>, instructions, built_in: bool, uses: u32 }`.
  - `templates::{list, get, create(conn, &TemplateInput) -> Result<i64>, update(conn, id, &TemplateInput), duplicate(conn, id) -> Result<i64>, delete(conn, id), reset(conn, id)}`. `TemplateInput { name, description, sections, instructions }`.
  - `BUILT_INS`: the five built-in templates with ids 1 to 5, their sections, hints and instructions. `reset` copies the built-in values back.
  - The notes prompt writes `## <name>` for each section, and adds "(<hint>)" in the heading list only when the hint is not empty.

- [ ] **Step 1: Write the failing tests**: `create_update_and_list_template`, `duplicate_adds_copy_suffix` (name "Product sync copy"), `delete_built_in_is_refused`, `delete_used_template_moves_meetings_to_general`, `reset_restores_built_in_values`, `uses_counts_meetings`, `old_string_sections_still_load`, and in `prompt.rs` `section_hints_show_in_the_system_prompt`.
- [ ] **Step 2: Run** `cargo test --manifest-path src-tauri/Cargo.toml templates prompt` — they fail.
- [ ] **Step 3: Write the code.** Names are trimmed. An empty name or no section gives `MorselError::Invalid`.
- [ ] **Step 4: Run** the full `cargo test` — all pass.
- [ ] **Step 5: Commit** `feat: let users edit, copy, reset and delete templates`

### Task 3: Ask engine

**Files:**
- Create: `src-tauri/src/ask.rs`
- Modify: `src-tauri/src/lib.rs` (module), `src-tauri/src/db/library.rs` (asks rows)

**Interfaces:**
- Consumes: `library::fts_query`, `llm::chat`, `LlmConfig`.
- Produces:
  - `AskRange { All, Week, Month }` (serde lowercase). Week = the last 7 days, Month = the last 30 days.
  - `AskSource { meeting_id, title, started_at }`, `AskAnswer { id: Option<i64>, question, answer, sources: Vec<AskSource> }`.
  - `ask::find_meetings(conn, question, range, now_ms) -> Result<Vec<MeetingId>>`: up to 8 by `bm25`.
  - `ask::ask(db, http, quick: &LlmConfig, question, range, meeting: Option<MeetingId>) -> Result<AskAnswer>`. With `meeting`, only that meeting is the context, and the answer is not saved. Without it, the answer is saved in `asks`.
  - `library::list_asks(conn, limit) -> Result<Vec<AskAnswer>>`, `library::get_ask(conn, id)`.
  - `ask::ASK_TOKENS: usize = 100_000`. Each meeting gets an equal share: title, date, note (or rough notes), then the transcript, cut to its share.

- [ ] **Step 1: Write the failing tests** (wiremock for the model): `finds_best_meetings_for_question`, `range_week_skips_old_meetings`, `ask_sends_only_found_meetings_and_returns_sources`, `ask_with_no_meetings_makes_no_model_call` (answer "No meeting matches this question."), `ask_context_stays_under_the_limit`, `meeting_ask_is_not_saved`.
- [ ] **Step 2: Run** `cargo test --manifest-path src-tauri/Cargo.toml ask` — they fail.
- [ ] **Step 3: Write the code.** The system prompt says: answer only from the meetings below, say when they do not hold the answer, keep the answer short.
- [ ] **Step 4: Run** the same command — all pass.
- [ ] **Step 5: Commit** `feat: answer questions across meetings`

### Task 4: Library commands

**Files:**
- Modify: `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`, `src-tauri/build.rs`, `src-tauri/capabilities/default.json` (plus `Cargo.toml` for `tauri-plugin-opener`)

**Interfaces:**
- Produces these commands (camelCase args): `search_notes(filter) -> NotesPage`, `list_people() -> Vec<String>`, `delete_meeting(meetingId)`, `save_template(id: Option<i64>, input) -> i64`, `duplicate_template(id) -> i64`, `delete_template(id)`, `reset_template(id)`, `ask(question, range, meetingId: Option<i64>) -> AskAnswer`, `list_asks(limit) -> Vec<AskAnswer>`, `get_ask(id) -> AskAnswer`, `usage_summary() -> UsageTotal` (last 30 days), `data_info() -> DataInfo { path, notes: u32, bytes: u64 }`, `export_all(dir) -> u32` (files written), `delete_all_data()`.
  - `export_all` writes `<yyyy-mm-dd> <title>.md` for each meeting with a note, and adds ` (2)` when two names match. Characters `/ \ : * ? " < > |` become `-`.
  - `delete_all_data` refuses with `MorselError::AlreadyRecording` while a meeting records, cancels note jobs, empties every table in one transaction, re-inserts the built-in templates, and deletes both keys.
  - Capabilities add each `allow-<command>`, `dialog:allow-open`, and `opener:allow-reveal-item-in-dir`.

- [ ] **Step 1: Write the failing tests** in `commands.rs` `mod tests`: `export_all_writes_one_file_per_note`, `export_all_makes_safe_unique_names`, `delete_all_clears_data_and_keys_and_keeps_built_ins`, `delete_all_refuses_while_recording`, `usage_summary_counts_last_30_days`.
- [ ] **Step 2: Run** `cargo test --manifest-path src-tauri/Cargo.toml commands` — they fail.
- [ ] **Step 3: Write the commands** and register them in `lib.rs` and `build.rs`.
- [ ] **Step 4: Run** the full `cargo test` and `cargo clippy --all-targets` — all pass, no warning.
- [ ] **Step 5: Commit** `feat: expose library, ask and data commands`

### Task 5: All notes screen

**Files:**
- Create: `src/screens/notes/NotesScreen.tsx`, `src/screens/notes/notes.test.tsx`
- Modify: `src/lib/api.ts`, `src/app/routes.tsx`

**Interfaces:**
- Consumes: Task 4 commands.
- Produces: `api` functions with the command names in camelCase, and the types `NotesFilter`, `NotesPage`, `Section`, `Template`, `TemplateInput`, `AskAnswer`, `AskRange`, `UsageTotal`, `DataInfo`. Route `/notes`.
- Board 4.1: title "Notes", a search box "Search notes", a template list ("All templates"), a person list ("Anyone"), a table Meeting / Template / People / Date / Length, and "N of M notes". Rows open `/note/:id` (or `/meeting/:id` while it records). Search waits 250 ms after the last key.

- [ ] **Step 1: Write the failing tests**: `lists_notes_with_count`, `search_waits_then_sends_query`, `template_and_person_filters_send_ids`, `row_opens_the_note`, `empty_state_shows_a_hint`.
- [ ] **Step 2: Run** `pnpm exec vitest run src/screens/notes` — they fail.
- [ ] **Step 3: Write the screen.** Length = `ended_at - started_at` in minutes, "—" when unknown. People = names joined by ", ", "—" when empty.
- [ ] **Step 4: Run** the same command — all pass.
- [ ] **Step 5: Commit** `feat: add the all notes screen with search and filters`

### Task 6: Templates screen

**Files:**
- Create: `src/screens/templates/TemplatesScreen.tsx`, `src/screens/templates/templates.test.tsx`
- Modify: `src/app/routes.tsx`, `src/screens/note/NoteScreen.tsx` (delete button with a confirm dialog)

**Interfaces:**
- Board 4.3: a list with "Used N times" (General shows "Default for new notes"), a "New template" button, a form with Name, When to use, Sections (name and hint, add, remove, move up and down), "Instructions for the AI", "Save template", "Duplicate", "Reset" (built-in only) and "Delete" (custom only). Route `/templates`.
- Produces: `ConfirmDialog` in `src/app/ConfirmDialog.tsx` (title, body, confirm label, `onConfirm`, `onCancel`), used by Tasks 6 and 8.

- [ ] **Step 1: Write the failing tests**: `shows_templates_with_use_counts`, `save_sends_sections_and_hints`, `new_template_starts_empty`, `reset_only_for_built_in`, `delete_asks_first`, and in `note.test.tsx` `delete_note_asks_then_goes_to_notes`.
- [ ] **Step 2: Run** `pnpm exec vitest run src/screens/templates src/screens/note` — they fail.
- [ ] **Step 3: Write the screen and the dialog.**
- [ ] **Step 4: Run** the same command — all pass.
- [ ] **Step 5: Commit** `feat: add the templates screen`

### Task 7: Ask Morsel

**Files:**
- Create: `src/screens/ask/AskScreen.tsx`, `src/screens/ask/ask.test.tsx`
- Modify: `src/screens/meeting/TranscriptPanel.tsx` (enable "Ask about this meeting"), `src/screens/HomeScreen.tsx` (box "Ask about any meeting", `⌘K` focuses it, Enter opens `/ask?q=`)

**Interfaces:**
- Board 4.2: the question bubble, the answer in EB Garamond, "From N meetings" with one link for each source ("<title>, <d Mon>"), a form with "Ask about your meetings", the range list (All meetings, This week, This month) and "Ask". Earlier questions show in the sidebar section of the screen; a click opens `/ask/:id`. Routes `/ask` and `/ask/:id`.
- The meeting panel shows the answer under the box, with no sources list.

- [ ] **Step 1: Write the failing tests**: `ask_sends_question_and_range`, `answer_shows_source_links`, `query_param_asks_at_once`, `earlier_question_opens_saved_answer`, `error_shows_message`, `meeting_panel_asks_about_this_meeting`, `home_cmd_k_focuses_ask_box`.
- [ ] **Step 2: Run** `pnpm exec vitest run src/screens/ask src/screens/meeting src/screens/screens.test.tsx` — they fail.
- [ ] **Step 3: Write the code.** The Ask button is disabled while a request runs and when the box is empty.
- [ ] **Step 4: Run** the same command — all pass.
- [ ] **Step 5: Commit** `feat: add Ask Morsel across meetings and in a meeting`

### Task 8: Settings privacy, usage and Home library

**Files:**
- Create: `src/screens/settings/SettingsLayout.tsx`, `src/screens/settings/SettingsPrivacyScreen.tsx`, `src/screens/settings/settings.test.tsx`
- Modify: `src/screens/SettingsModelsScreen.tsx` (move into the layout, add usage), `src/app/routes.tsx`, `src/app/AppShell.tsx` (notes count), `src/screens/HomeScreen.tsx` (group by day, "View all")

**Interfaces:**
- Board 5.2: a settings sidebar ("Back to Morsel", Models, Transcription, Calendar, Privacy and data, Shortcuts). Calendar and Shortcuts show "Comes with the calendar setup." until Plan 3. Privacy rows: the fixed audio line, "Data folder" with path, count and size and "Show in Finder", "Export all notes as Markdown" with "Export", and "Delete all notes, transcripts and keys" with "Delete all data" and the dialog. Route `/settings/privacy`.
- 5.1 gains "Usage, last 30 days: N tokens" and "$X.XX" only when the cost is known.
- Home (board 2.1): "Earlier notes" grouped as Today, Yesterday, weekday names, then dates; "View all" opens `/notes`. The sidebar shows the note count next to "Notes".

- [ ] **Step 1: Write the failing tests**: `privacy_shows_data_info`, `export_picks_folder_and_reports_count`, `delete_all_asks_then_deletes`, `models_shows_usage_and_hides_unknown_cost`, `home_groups_notes_by_day`, `sidebar_shows_note_count`.
- [ ] **Step 2: Run** `pnpm exec vitest run src/screens src/app` — they fail.
- [ ] **Step 3: Write the code.**
- [ ] **Step 4: Run** `pnpm test` and `pnpm exec tsc --noEmit` — all pass.
- [ ] **Step 5: Commit** `feat: add privacy and data settings, usage, and a library home`

## Final check

```bash
pnpm test && pnpm exec tsc --noEmit && cargo test --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets && pnpm tauri build --debug
```

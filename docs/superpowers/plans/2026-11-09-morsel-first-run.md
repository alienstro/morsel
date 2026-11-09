# Morsel first run and calendar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the first-run screens 1.1 to 1.3, the Composio calendar link, the next-meeting card, the meeting reminder, and the global shortcut.

**Architecture:** The Rust backend owns the Composio REST calls, the Composio key (Keychain), the event cache, the reminder timer and the global shortcut. The frontend shows the onboarding, the Home card, the Calendar and Shortcuts settings pages, and a small reminder window. The deep-link plugin brings the user back after the calendar login.

**Tech Stack:** Tauri 2, tauri-plugin-deep-link, tauri-plugin-global-shortcut, objc2-av-foundation, chrono, reqwest, wiremock (tests), React 19, Vitest.

**Spec:** `docs/superpowers/specs/2026-11-08-morsel-v1-design.md` (sections 11, 12, 13, 16). Look: the canvas boards Onboard-Welcome, Onboard-AI, Onboard-Calendar, Main, Reminder, Settings-Privacy.

## Global Constraints

- The calendar is optional. Morsel works without it.
- The reminder shows one minute before a calendar meeting, with "Start notes" and "Dismiss". The user can turn it off. Default: on.
- No auto-start setting in v1 (spec §11 wins over the canvas switch).
- The Composio key stays in the Keychain (service `app.morsel`), account `composio_api_key`. It never goes to the frontend, a log, or the database.
- Composio API: base `https://backend.composio.dev/api/v3`, header `x-api-key`.
- Morsel reads only the title, the time, the video link and the people of each event. It never changes the calendar.
- Copy from the canvas: "Step 1 of 3", "Welcome to Morsel", "Set up the AI", "Connect your calendar", "Skip for now", "Finish", "Continue", "Back", "What Morsel reads", "<title> in 1 min", "Start notes", "Dismiss".
- The frontend tests mock Tauri through `src/test/tauri.ts`. Each new command goes in `build.rs` COMMANDS, `capabilities/default.json` and `lib.rs`.

## Review Focus

1. A Composio response with an unexpected shape (no `items`, nested `data`) must give an empty list, not a crash. Test: `parse_google_events_accepts_nested_data`.
2. All-day events and cancelled events must not show on the card and must not get a reminder. Test: `parse_google_events_skips_all_day_and_cancelled`.
3. A reminder must show once per event, also when the timer runs many times in the last minute. Test: `due_reminders_returns_each_event_once`.
4. The global shortcut while a meeting records must open that meeting, not start a second one. Test: `shortcut_while_recording_opens_the_live_meeting`.
5. "Delete all data" must also delete the Composio key and the calendar link. Test: `delete_all_data_removes_the_composio_key`.

---

### Task 1: Settings for the first run, the reminder and the shortcut

**Files:**
- Modify: `src-tauri/src/settings.rs`, `src/lib/api.ts`, `src/app/routes.tsx`
- Test: `src-tauri/src/settings.rs` (tests module), `src/app/onboarding-gate.test.tsx`

**Interfaces:**
- Produces: `Settings { onboarded: bool (default false), remind_before_meeting: bool (default true), global_shortcut: String (default "CommandOrControl+Shift+M") }`; TS `Settings.onboarded`, `remindBeforeMeeting`, `globalShortcut`.
- Produces: route `/welcome/:step` (step = `1|2|3`) outside the shell. The shell redirects to `/welcome/1` when `onboarded` is false.

- [ ] Test `default_settings_need_onboarding`: `!s.onboarded && s.remind_before_meeting && s.global_shortcut == "CommandOrControl+Shift+M"`.
- [ ] Test `old_settings_json_loads_with_new_defaults`: a saved JSON without the new fields loads with the defaults.
- [ ] Vitest `redirects_to_welcome_when_not_onboarded` and `shows_home_when_onboarded`.
- [ ] Implement, run `cargo test settings` and `pnpm vitest run src/app`, commit.

### Task 2: Microphone permission and the privacy settings links

**Files:**
- Create: `src-tauri/src/permissions.rs`
- Modify: `src-tauri/Cargo.toml` (objc2-av-foundation), `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`, `src-tauri/build.rs`, `capabilities/default.json`, `src/lib/api.ts`

**Interfaces:**
- Produces: `enum MicAccess { Allowed, Denied, Unknown }` (serde lowercase); `fn mic_access_from_status(raw: isize) -> MicAccess` (3 → Allowed, 1 or 2 → Denied, else Unknown).
- Produces commands: `microphone_access() -> MicAccess`, `request_microphone() -> MicAccess` (async; waits for the answer of the macOS prompt), `open_privacy_settings(pane: "microphone" | "audio_capture")`.
- Pane URLs: `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone` and `...?Privacy_AudioCapture`.

- [ ] Test `mic_access_maps_the_av_status` for 0, 1, 2, 3.
- [ ] Test `open_privacy_settings_rejects_an_unknown_pane`.
- [ ] Implement, run `cargo test permissions`, commit.

### Task 3: Onboarding screens 1.1 and 1.2

**Files:**
- Create: `src/screens/onboarding/OnboardingLayout.tsx`, `src/screens/onboarding/WelcomeStep.tsx`, `src/screens/onboarding/AiStep.tsx`, `src/screens/onboarding/onboarding.css`
- Test: `src/screens/onboarding/onboarding.test.tsx`

**Interfaces:**
- Consumes: Task 1 settings, Task 2 commands, existing `downloadModel`, `onModelDownload`, `setApiKey`, `testNotesKey`, `saveSettings`.
- Produces: `OnboardingLayout({ step, title, lead, children, footer })` with the painting panel and "Step N of 3".

- [ ] Tests: `welcome_continue_needs_the_microphone`, `welcome_asks_for_the_microphone`, `system_audio_opens_system_settings`, `ai_step_saves_settings_then_tests_the_connection`, `ai_step_shows_download_progress`, `ai_step_back_goes_to_step_1`.
- [ ] Implement, run `pnpm vitest run src/screens/onboarding`, commit.

### Task 4: Composio client and event parsers

**Files:**
- Create: `src-tauri/src/calendar/mod.rs`, `src-tauri/src/calendar/composio.rs`, `src-tauri/src/calendar/events.rs`
- Modify: `src-tauri/Cargo.toml` (chrono), `src-tauri/src/secrets.rs` (`COMPOSIO_KEY = "composio_api_key"` in `ACCOUNTS`)

**Interfaces:**
- Produces: `enum Provider { Google, Outlook }` (serde lowercase), toolkit slugs `googlecalendar` and `outlook`, tools `GOOGLECALENDAR_EVENTS_LIST` and `OUTLOOK_GET_CALENDAR_VIEW`.
- Produces: `struct CalendarEvent { id: String, provider: Provider, title: String, start: i64 (ms), end: i64 (ms), link: Option<String>, people: Vec<String> }` (camelCase).
- Produces: `Composio::new(http, base_url, key)`, `create_auth_config(Provider) -> String`, `link(auth_config_id, user_id, callback) -> Link { redirect_url, connected_account_id }`, `account_active(id) -> bool`, `delete_account(id)`, `list_events(provider, account_id, user_id, from_ms, to_ms) -> Vec<CalendarEvent>`.
- Produces: `parse_google_events(&Value) -> Vec<CalendarEvent>`, `parse_outlook_events(&Value) -> Vec<CalendarEvent>`, `meeting_place(link) -> Option<&'static str>` ("Google Meet", "Zoom", "Microsoft Teams").

- [ ] Tests (wiremock): `create_auth_config_sends_managed_auth`, `link_returns_the_redirect_url`, `execute_error_maps_to_calendar_error`.
- [ ] Tests (parsers): `parse_google_events_reads_title_time_link_people`, `parse_google_events_skips_all_day_and_cancelled`, `parse_google_events_accepts_nested_data`, `parse_outlook_events_reads_utc_times`, `meeting_place_names_the_app`.
- [ ] Implement, run `cargo test calendar`, commit.

### Task 5: Calendar service, commands and the deep link

**Files:**
- Create: `src-tauri/src/calendar/service.rs`
- Modify: `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`, `src-tauri/build.rs`, `capabilities/default.json`, `src-tauri/tauri.conf.json` (deep-link scheme `morsel`), `src/lib/api.ts`

**Interfaces:**
- Produces: the settings row `calendar` with `CalendarLink { user_id, google: Option<Connection>, outlook: Option<Connection> }`, `Connection { auth_config_id, account_id, active }`.
- Produces commands: `calendar_status() -> CalendarStatus { hasKey, google: "none"|"pending"|"connected", outlook: ..., today: u32 }`, `connect_calendar(provider)` (opens the login page in the browser), `disconnect_calendar(provider)`, `upcoming_events() -> Vec<CalendarEvent>` (now to the end of the day, from the cache).
- Produces: event `calendar://changed` after a deep link `morsel://calendar/...` and after each refresh. The cache refreshes every 5 minutes.

- [ ] Tests: `connect_saves_a_pending_connection_and_returns_the_url`, `status_marks_a_pending_connection_connected_when_active`, `disconnect_deletes_the_account_and_the_link`, `upcoming_events_skips_past_events`, `delete_all_data_removes_the_composio_key`.
- [ ] Implement, run `cargo test calendar`, commit.

### Task 6: Calendar step 1.3 and Settings Calendar

**Files:**
- Create: `src/screens/calendar/CalendarPanel.tsx`, `src/screens/onboarding/CalendarStep.tsx`, `src/screens/settings/SettingsCalendarScreen.tsx`
- Test: `src/screens/calendar/calendar.test.tsx`

**Interfaces:**
- Consumes: Task 5 commands and event, Task 1 `remindBeforeMeeting`.
- Produces: `CalendarPanel()` (key field, one row for each provider with Connect or Disconnect).

- [ ] Tests: `panel_saves_the_composio_key`, `connect_calls_the_backend`, `connected_row_shows_the_meeting_count`, `finish_marks_onboarding_done`, `skip_marks_onboarding_done`, `reminder_switch_saves_the_setting`.
- [ ] Implement, run `pnpm vitest run src/screens/calendar`, commit.

### Task 7: Next-meeting card on Home and meetings with people

**Files:**
- Modify: `src-tauri/src/commands.rs` (`start_meeting` gains `people: Vec<String>`), `src-tauri/src/session.rs`, `src/screens/HomeScreen.tsx`, `src/lib/api.ts`
- Test: `src/screens/screens.test.tsx`, Rust test in `commands.rs`

**Interfaces:**
- Produces: `api.startMeeting(title, templateId, people = [])`.
- Card copy: `<HH:MM> | <title> | <place> with <people>, in <n> min` ("now" when the start is past).

- [ ] Tests: `start_meeting_saves_the_people`, `home_shows_the_next_meeting_card`, `card_start_sends_title_and_people`, `home_without_calendar_shows_the_plain_start`.
- [ ] Implement, run the two test files, commit.

### Task 8: Meeting reminder window

**Files:**
- Create: `src-tauri/src/calendar/reminder.rs`, `src/reminder/ReminderApp.tsx`, `src-tauri/capabilities/reminder.json`
- Modify: `src/main.tsx` (window label `reminder` renders `ReminderApp`), `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `fn due_reminders(events, now_ms, shown: &mut HashSet<String>) -> Vec<CalendarEvent>` (start in `(now, now + 60_000]`).
- Produces commands: `reminder_event() -> Option<CalendarEvent>`, `start_from_reminder(template_id)` (starts the meeting, closes the window, focuses main, emits `app://open-meeting { id }`), `dismiss_reminder()`.
- The window: label `reminder`, 380 × 104, no decorations, always on top, top right of the main screen, not focused.

- [ ] Tests: `due_reminders_returns_each_event_once`, `due_reminders_skips_when_off`, `reminder_shows_title_and_people`, `start_notes_calls_the_backend`.
- [ ] Implement, run `cargo test reminder` and `pnpm vitest run src/reminder`, commit.

### Task 9: Global shortcut and Settings Shortcuts

**Files:**
- Create: `src-tauri/src/shortcut.rs`, `src/screens/settings/SettingsShortcutsScreen.tsx`
- Modify: `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml`, `src/app/routes.tsx` (listens to `app://open-meeting`)

**Interfaces:**
- Produces: `fn shortcut_action(recording: Option<MeetingId>) -> ShortcutAction { OpenLive(id), StartNew }`; command `set_global_shortcut(shortcut: String)` (checks, registers, saves).

- [ ] Tests: `shortcut_while_recording_opens_the_live_meeting`, `bad_shortcut_is_rejected`, `shortcuts_page_saves_a_new_shortcut`, `open_meeting_event_navigates`.
- [ ] Implement, run the tests, commit.

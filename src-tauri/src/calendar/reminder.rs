//! The reminder one minute before a calendar meeting, in a small window of its own.

use super::CalendarEvent;
use super::service::CalendarService;
use crate::commands::AppState;
use crate::db::templates::GENERAL_ID;
use crate::types::{MeetingId, MorselError, Result};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

pub const WINDOW: &str = "reminder";
pub const OPEN_MEETING: &str = "app://open-meeting";
const LEAD_MS: i64 = 60_000;
const TICK: std::time::Duration = std::time::Duration::from_secs(15);
const WIDTH: f64 = 400.0;
const HEIGHT: f64 = 112.0;

/// The events that start in the next minute and have no reminder yet. An event that moves
/// to a new time gets a new reminder.
pub fn due_reminders(
    enabled: bool,
    events: &[CalendarEvent],
    now_ms: i64,
    shown: &mut HashSet<String>,
) -> Vec<CalendarEvent> {
    if !enabled {
        return Vec::new();
    }
    events
        .iter()
        .filter(|e| e.start > now_ms && e.start <= now_ms + LEAD_MS)
        .filter(|e| shown.insert(format!("{}@{}", e.id, e.start)))
        .cloned()
        .collect()
}

/// The event that the reminder window shows.
#[derive(Default)]
pub struct ReminderState {
    current: Mutex<Option<CalendarEvent>>,
    shown: Mutex<HashSet<String>>,
}

fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    m.lock().map_err(|_| MorselError::Db("lock".into()))
}

/// Checks the calendar every 15 seconds and shows a reminder when a meeting is one minute away.
pub fn start_reminder_loop<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(err) = tick(&app) {
                eprintln!("reminder check failed: {err}");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

fn tick<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    let enabled = app.state::<AppState>().settings()?.remind_before_meeting;
    let events = app.state::<Arc<CalendarService>>().events()?;
    let reminders = app.state::<ReminderState>();
    let now = chrono::Utc::now().timestamp_millis();
    let due = due_reminders(enabled, &events, now, &mut *lock(&reminders.shown)?);
    if let Some(event) = due.into_iter().next() {
        *lock(&reminders.current)? = Some(event);
        open_window(app)?;
    }
    Ok(())
}

fn open_window<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    if let Some(old) = app.get_webview_window(WINDOW) {
        let _ = old.destroy();
    }
    let mut builder = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("index.html".into()))
        .title("Morsel reminder")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible_on_all_workspaces(true);
    if let Ok(Some(screen)) = app.primary_monitor() {
        let size = screen.size().to_logical::<f64>(screen.scale_factor());
        builder = builder.position(size.width - WIDTH - 16.0, 40.0);
    }
    builder
        .build()
        .map(|_| ())
        .map_err(|e| MorselError::Io(e.to_string()))
}

fn close_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.destroy();
    }
}

#[tauri::command]
pub fn reminder_event(state: tauri::State<'_, ReminderState>) -> Result<Option<CalendarEvent>> {
    Ok(lock(&state.current)?.clone())
}

/// Starts the meeting of the reminder, then shows it in the main window.
#[tauri::command]
pub async fn start_from_reminder<R: Runtime>(
    app: AppHandle<R>,
    reminders: tauri::State<'_, ReminderState>,
    state: tauri::State<'_, AppState>,
) -> Result<MeetingId> {
    let event = lock(&reminders.current)?
        .clone()
        .ok_or_else(|| MorselError::NotFound("reminder".into()))?;
    let id = state
        .start_meeting_with(Some(event.title), GENERAL_ID, event.people)
        .await?;
    *lock(&reminders.current)? = None;
    crate::show_main(&app);
    let _ = app.emit_to("main", OPEN_MEETING, serde_json::json!({ "id": id }));
    close_window(&app);
    Ok(id)
}

#[tauri::command]
pub fn dismiss_reminder<R: Runtime>(
    app: AppHandle<R>,
    reminders: tauri::State<'_, ReminderState>,
) -> Result<()> {
    *lock(&reminders.current)? = None;
    close_window(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Provider;

    fn event(id: &str, start: i64) -> CalendarEvent {
        CalendarEvent {
            id: id.into(),
            provider: Provider::Google,
            title: id.into(),
            start,
            end: start + 30 * 60_000,
            link: None,
            people: vec![],
        }
    }

    #[test]
    fn due_reminders_returns_each_event_once() {
        let mut shown = HashSet::new();
        let events = vec![
            event("soon", 1_000 + 50_000),
            event("later", 1_000 + 5 * 60_000),
        ];
        let first = due_reminders(true, &events, 1_000, &mut shown);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, "soon");
        for t in [16_000, 31_000, 46_000] {
            assert!(due_reminders(true, &events, t, &mut shown).is_empty());
        }
    }

    #[test]
    fn due_reminders_skips_events_outside_the_minute() {
        let mut shown = HashSet::new();
        let now = 10_000_000;
        let events = vec![
            event("started", now - 1),
            event("exact", now),
            event("edge", now + 60_000),
            event("far", now + 60_001),
        ];
        let ids: Vec<String> = due_reminders(true, &events, now, &mut shown)
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec!["edge"]);
    }

    #[test]
    fn due_reminders_skips_when_off() {
        let mut shown = HashSet::new();
        let events = vec![event("soon", 30_000)];
        assert!(due_reminders(false, &events, 0, &mut shown).is_empty());
        assert!(shown.is_empty());
    }

    #[test]
    fn a_moved_event_gets_a_new_reminder() {
        let mut shown = HashSet::new();
        assert_eq!(
            due_reminders(true, &[event("e", 30_000)], 0, &mut shown).len(),
            1
        );
        let moved = event("e", 10 * 60_000 + 30_000);
        assert_eq!(
            due_reminders(true, &[moved], 10 * 60_000, &mut shown).len(),
            1
        );
    }
}

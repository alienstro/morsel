//! The global shortcut that starts a note for a meeting that is not in the calendar.

use crate::calendar::reminder::OPEN_MEETING;
use crate::commands::AppState;
use crate::db::templates::GENERAL_ID;
use crate::types::{MeetingId, MorselError, Result};
use std::str::FromStr;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    /// A meeting records now: show it. Morsel never starts a second meeting.
    OpenLive(MeetingId),
    StartNew,
}

pub fn shortcut_action(recording: Option<MeetingId>) -> ShortcutAction {
    match recording {
        Some(id) => ShortcutAction::OpenLive(id),
        None => ShortcutAction::StartNew,
    }
}

/// Parses an accelerator such as "CommandOrControl+Shift+M". A global shortcut needs
/// Command, Control or Option, so that it does not take a plain key from other apps.
pub fn parse_shortcut(raw: &str) -> Result<Shortcut> {
    let shortcut = Shortcut::from_str(raw.trim())
        .map_err(|e| MorselError::Invalid(format!("shortcut \"{raw}\": {e}")))?;
    let strong = Modifiers::SUPER | Modifiers::CONTROL | Modifiers::ALT;
    if !shortcut.mods.intersects(strong) {
        return Err(MorselError::Invalid(
            "A shortcut needs Command, Control or Option.".into(),
        ));
    }
    Ok(shortcut)
}

/// Replaces the registered global shortcut.
pub fn register<R: Runtime>(app: &AppHandle<R>, raw: &str) -> Result<()> {
    let shortcut = parse_shortcut(raw)?;
    let gs = app.global_shortcut();
    gs.unregister_all()
        .map_err(|e| MorselError::Invalid(e.to_string()))?;
    gs.register(shortcut)
        .map_err(|e| MorselError::Invalid(format!("macOS refused the shortcut: {e}")))
}

/// The handler of the plugin: on a key press, open the live meeting or start a new one.
pub fn on_shortcut<R: Runtime>(app: &AppHandle<R>, state: ShortcutState) {
    if state != ShortcutState::Pressed {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let meetings = app.state::<AppState>();
        let id = match shortcut_action(meetings.recording_meeting().await) {
            ShortcutAction::OpenLive(id) => Ok(id),
            ShortcutAction::StartNew => meetings.start_meeting(None, GENERAL_ID).await,
        };
        crate::show_main(&app);
        match id {
            Ok(id) => {
                let _ = app.emit_to("main", OPEN_MEETING, serde_json::json!({ "id": id }));
            }
            Err(err) => eprintln!("the shortcut could not start a meeting: {err}"),
        }
    });
}

/// Checks and registers a new shortcut, then saves it. A refused shortcut keeps the old one.
#[tauri::command]
pub fn set_global_shortcut<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    shortcut: String,
) -> Result<()> {
    let mut settings = state.settings()?;
    if let Err(err) = register(&app, &shortcut) {
        let _ = register(&app, &settings.global_shortcut);
        return Err(err);
    }
    settings.global_shortcut = shortcut.trim().to_owned();
    state.save_settings(&settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_while_recording_opens_the_live_meeting() {
        assert_eq!(shortcut_action(Some(7)), ShortcutAction::OpenLive(7));
        assert_eq!(shortcut_action(None), ShortcutAction::StartNew);
    }

    #[test]
    fn bad_shortcut_is_rejected() {
        assert!(parse_shortcut("CommandOrControl+Shift+M").is_ok());
        assert!(parse_shortcut("Alt+Space").is_ok());
        assert!(parse_shortcut("Shift+M").is_err());
        assert!(parse_shortcut("M").is_err());
        assert!(parse_shortcut("Command+Nope").is_err());
        assert!(parse_shortcut("").is_err());
    }
}

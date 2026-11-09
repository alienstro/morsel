/// The commands that the app exposes. Each one needs an `allow-<name>` permission in a capability.
const COMMANDS: &[&str] = &[
    "start_meeting",
    "pause_meeting",
    "resume_meeting",
    "stop_meeting",
    "save_user_notes",
    "get_meeting",
    "list_meetings",
    "list_templates",
    "get_settings",
    "save_settings",
    "set_api_key",
    "has_api_key",
    "write_notes",
    "cancel_notes",
    "export_note",
    "test_notes_key",
    "model_status",
    "download_model",
    "search_notes",
    "list_people",
    "delete_meeting",
    "save_template",
    "duplicate_template",
    "delete_template",
    "reset_template",
    "ask",
    "list_asks",
    "get_ask",
    "usage_summary",
    "data_info",
    "export_all",
    "delete_all_data",
    "microphone_access",
    "request_microphone",
    "open_privacy_settings",
    "calendar_status",
    "connect_calendar",
    "disconnect_calendar",
    "refresh_calendar",
    "upcoming_events",
    "reminder_event",
    "start_from_reminder",
    "dismiss_reminder",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run the Tauri build script");
}

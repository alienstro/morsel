pub mod ask;
pub mod audio;
pub mod calendar;
pub mod chunker;
pub mod commands;
pub mod db;
pub mod events;
pub mod llm;
pub mod notes;
pub mod permissions;
pub mod secrets;
pub mod session;
pub mod settings;
pub mod transcribe;
pub mod types;

use calendar::service::CalendarService;
use commands::{AppState, RealEngines};
use events::EventSink;
use secrets::{KeychainStore, SecretStore};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Builder, Emitter, Manager, Runtime};
use tauri_plugin_deep_link::DeepLinkExt;

/// Sends backend events to every window.
pub struct TauriSink<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> EventSink for TauriSink<R> {
    fn emit(&self, name: &str, payload: serde_json::Value) {
        if let Err(err) = self.0.emit(name, payload) {
            eprintln!("cannot send event {name}: {err}");
        }
    }
}

/// Adds the Morsel commands to a Tauri builder.
pub fn configure<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![
            commands::start_meeting,
            commands::pause_meeting,
            commands::resume_meeting,
            commands::stop_meeting,
            commands::save_user_notes,
            commands::get_meeting,
            commands::list_meetings,
            commands::list_templates,
            commands::get_settings,
            commands::save_settings,
            commands::set_api_key,
            commands::has_api_key,
            commands::write_notes,
            commands::cancel_notes,
            commands::export_note,
            commands::test_notes_key,
            commands::model_status,
            commands::download_model,
            commands::search_notes,
            commands::list_people,
            commands::delete_meeting,
            commands::save_template,
            commands::duplicate_template,
            commands::delete_template,
            commands::reset_template,
            commands::ask,
            commands::list_asks,
            commands::get_ask,
            commands::usage_summary,
            commands::data_info,
            commands::export_all,
            commands::delete_all_data,
            permissions::microphone_access,
            permissions::request_microphone,
            permissions::open_privacy_settings,
            calendar::service::calendar_status,
            calendar::service::connect_calendar,
            calendar::service::disconnect_calendar,
            calendar::service::refresh_calendar,
            calendar::service::upcoming_events,
        ])
}

/// Opens the database, marks interrupted meetings, and creates the app state.
fn setup<R: Runtime>(app: &mut tauri::App<R>) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let db = Arc::new(Mutex::new(db::open(&data_dir.join("morsel.db"))?));
    let interrupted = session::recover(&db)?;
    if interrupted > 0 {
        eprintln!("marked {interrupted} meeting(s) as interrupted");
    }
    let http = reqwest::Client::new();
    let secrets: Arc<dyn SecretStore> = Arc::new(KeychainStore);
    let model_dir = transcribe::model_download::model_dir(&data_dir);
    let engines = Arc::new(RealEngines::new(
        model_dir.clone(),
        http.clone(),
        secrets.clone(),
    ));
    let sink = Arc::new(TauriSink(app.handle().clone()));
    let calendar = Arc::new(CalendarService::new(
        db.clone(),
        secrets.clone(),
        http.clone(),
        calendar::composio::BASE_URL,
        sink.clone(),
    ));
    app.manage(calendar.clone());
    app.manage(AppState::new(
        db, secrets, http, sink, engines, data_dir, model_dir,
    ));
    start_calendar_poll(calendar.clone());
    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        let from_calendar = event
            .urls()
            .iter()
            .any(|u| u.scheme() == "morsel" && u.host_str() == Some("calendar"));
        if from_calendar {
            show_main(&handle);
            let cal = calendar.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = cal.check_pending().await {
                    eprintln!("cannot check the calendar link: {err}");
                }
                let _ = cal.refresh(chrono::Utc::now().timestamp_millis()).await;
            });
        }
    });
    Ok(())
}

/// Reads the calendars now and then every 5 minutes.
fn start_calendar_poll(calendar: Arc<CalendarService>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(err) = calendar
                .refresh(chrono::Utc::now().timestamp_millis())
                .await
            {
                eprintln!("cannot read the calendars: {err}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(5 * 60)).await;
        }
    });
}

/// Shows and focuses the main window.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.unminimize();
        let _ = main.show();
        let _ = main.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    configure(tauri::Builder::default())
        .setup(setup)
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_builds() {
        let app = super::configure(tauri::test::mock_builder())
            .build(tauri::test::mock_context(tauri::test::noop_assets()));
        assert!(app.is_ok());
    }
}

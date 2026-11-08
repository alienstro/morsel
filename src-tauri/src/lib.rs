pub mod audio;
pub mod chunker;
pub mod commands;
pub mod db;
pub mod events;
pub mod llm;
pub mod notes;
pub mod secrets;
pub mod session;
pub mod settings;
pub mod transcribe;
pub mod types;

use commands::{AppState, RealEngines};
use events::EventSink;
use secrets::{KeychainStore, SecretStore};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Builder, Emitter, Manager, Runtime};

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
    app.manage(AppState::new(db, secrets, http, sink, engines, model_dir));
    Ok(())
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

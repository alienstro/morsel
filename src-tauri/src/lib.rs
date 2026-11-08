pub mod audio;
pub mod chunker;
pub mod db;
pub mod llm;
pub mod secrets;
pub mod settings;
pub mod transcribe;
pub mod types;

use tauri::{Builder, Runtime};

/// Adds the Morsel plugins, state and commands to a Tauri builder.
pub fn configure<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    configure(tauri::Builder::default())
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

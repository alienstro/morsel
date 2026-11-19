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
    "request_system_audio",
    "open_privacy_settings",
    "calendar_status",
    "connect_calendar",
    "disconnect_calendar",
    "refresh_calendar",
    "upcoming_events",
    "reminder_event",
    "start_from_reminder",
    "dismiss_reminder",
    "set_global_shortcut",
    "audio_size",
    "delete_audio",
    "notes_model_status",
    "download_notes_model",
    "search_model_status",
    "download_search_model",
];

fn main() {
    link_compiler_runtime();
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run the Tauri build script");
}

/// The Metal code of llama.cpp uses `@available(macOS 15.0, *)`. With a lower minimum macOS,
/// that check calls `___isPlatformVersionAtLeast`. LTO drops the weak copy in Rust `std`,
/// so the release link fails unless the clang runtime library supplies the function.
fn link_compiler_runtime() {
    // The build script runs on the host, so read the target from Cargo, not from `cfg!`.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = std::process::Command::new("clang")
        .arg("--print-runtime-dir")
        .output()
        .expect("clang must be installed to find its runtime library");
    let dir = String::from_utf8(out.stdout).expect("clang printed a path that is not UTF-8");
    println!("cargo:rustc-link-search=native={}", dir.trim());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}

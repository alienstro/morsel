//! The microphone permission and the links to the macOS privacy settings.

use crate::types::{MorselError, Result};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MicAccess {
    Allowed,
    Denied,
    Unknown,
}

/// Maps an `AVAuthorizationStatus` value: 3 is authorized, 1 (restricted) and 2 (denied) are denied.
pub fn mic_access_from_status(raw: isize) -> MicAccess {
    match raw {
        3 => MicAccess::Allowed,
        1 | 2 => MicAccess::Denied,
        _ => MicAccess::Unknown,
    }
}

/// Returns the System Settings URL of a privacy pane.
pub fn privacy_pane_url(pane: &str) -> Result<&'static str> {
    match pane {
        "microphone" => {
            Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        }
        "audio_capture" => {
            Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_AudioCapture")
        }
        other => Err(MorselError::Invalid(format!(
            "unknown privacy pane: {other}"
        ))),
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{MicAccess, mic_access_from_status};
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};

    pub fn access() -> MicAccess {
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
            return MicAccess::Unknown;
        };
        let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) };
        mic_access_from_status(status.0)
    }

    /// Shows the macOS prompt when the user has not answered yet, and waits for the answer.
    pub async fn request() -> MicAccess {
        if access() != MicAccess::Unknown {
            return access();
        }
        let Some(rx) = ask() else {
            return MicAccess::Unknown;
        };
        match rx.await {
            Ok(true) => MicAccess::Allowed,
            Ok(false) => MicAccess::Denied,
            Err(_) => access(),
        }
    }

    /// Starts the request. The block is not `Send`, so it stays in this sync function.
    fn ask() -> Option<tokio::sync::oneshot::Receiver<bool>> {
        let audio = unsafe { AVMediaTypeAudio }?;
        let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
        let tx = std::sync::Mutex::new(Some(tx));
        let block = RcBlock::new(move |granted: Bool| {
            if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
                let _ = tx.send(granted.as_bool());
            }
        });
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &block) };
        Some(rx)
    }
}

#[cfg(target_os = "macos")]
use mac::{access as current_mic_access, request as ask_for_mic};

#[cfg(not(target_os = "macos"))]
fn current_mic_access() -> MicAccess {
    MicAccess::Unknown
}

#[cfg(not(target_os = "macos"))]
async fn ask_for_mic() -> MicAccess {
    MicAccess::Unknown
}

#[tauri::command]
pub fn microphone_access() -> MicAccess {
    current_mic_access()
}

#[tauri::command]
pub async fn request_microphone() -> MicAccess {
    ask_for_mic().await
}

/// How long the system audio probe records. macOS shows its prompt while the tap runs.
const SYSTEM_AUDIO_PROBE: std::time::Duration = std::time::Duration::from_secs(1);

/// Records system audio for a moment and drops the sound. macOS has no call that asks for
/// this permission, and it lists an app in System Settings only after the app records once.
#[tauri::command]
pub async fn request_system_audio() -> Result<()> {
    tauri::async_runtime::spawn_blocking(probe_system_audio)
        .await
        .map_err(|e| MorselError::Audio(e.to_string()))?
}

fn probe_system_audio() -> Result<()> {
    use crate::audio::{source::AudioSource, system_tap::SystemTapSource};
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let mut tap = SystemTapSource::new();
    tap.start(tx)?;
    std::thread::sleep(SYSTEM_AUDIO_PROBE);
    tap.stop();
    drop(rx);
    Ok(())
}

#[tauri::command]
pub fn open_privacy_settings<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    pane: String,
) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let url = privacy_pane_url(&pane)?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| MorselError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mic_access_maps_the_av_status() {
        assert_eq!(mic_access_from_status(0), MicAccess::Unknown);
        assert_eq!(mic_access_from_status(1), MicAccess::Denied);
        assert_eq!(mic_access_from_status(2), MicAccess::Denied);
        assert_eq!(mic_access_from_status(3), MicAccess::Allowed);
    }

    #[test]
    fn open_privacy_settings_rejects_an_unknown_pane() {
        assert!(privacy_pane_url("camera").is_err());
        assert!(
            privacy_pane_url("microphone")
                .unwrap()
                .ends_with("Privacy_Microphone")
        );
        assert!(
            privacy_pane_url("audio_capture")
                .unwrap()
                .ends_with("Privacy_AudioCapture")
        );
    }
}

//! Manual check: records the system audio for 5 seconds and prints the level.
//! Play a video or a sound, then run: cargo run --example tap_check

use morsel_lib::audio::{level::rms, source::AudioSource, system_tap::SystemTapSource};

#[tokio::main]
async fn main() {
    let mut tap = SystemTapSource::new();
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    if let Err(err) = tap.start(tx) {
        eprintln!("cannot start system audio: {err}");
        std::process::exit(1);
    }
    println!("Recording the system audio for 5 seconds...");
    let end = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut peak = 0.0f32;
    let mut samples = 0usize;
    while let Ok(Some(frame)) = tokio::time::timeout_at(end, rx.recv()).await {
        samples += frame.len();
        peak = peak.max(rms(&frame));
    }
    tap.stop();
    println!("samples: {samples} (expected about 80000), highest level: {peak:.3}");
}

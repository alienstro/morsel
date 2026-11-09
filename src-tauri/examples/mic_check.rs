//! Manual check: records the microphone for 5 seconds and prints the level.
//! Run: cargo run --example mic_check

use morsel_lib::audio::{level::rms, mic::MicSource, source::AudioSource};

#[tokio::main]
async fn main() {
    let mut mic = MicSource::default_device();
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    if let Err(err) = mic.start(tx) {
        eprintln!("cannot start the microphone: {err}");
        std::process::exit(1);
    }
    println!("Speak now. Recording 5 seconds...");
    let end = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut peak = 0.0f32;
    let mut samples = 0usize;
    while let Ok(Some(frame)) = tokio::time::timeout_at(end, rx.recv()).await {
        samples += frame.len();
        peak = peak.max(rms(&frame));
    }
    mic.stop();
    println!("samples: {samples} (expected about 80000), highest level: {peak:.3}");
}

//! Local transcription with Whisper Turbo (int8) through sherpa-onnx.
//! Whisper knows most languages, Tagalog too, and it reads at most 30 s of audio for each
//! call. The chunker never makes a chunk longer than 28 s.

use super::model_download::model_ready;
use super::{Transcriber, language_param};
use crate::audio::PIPELINE_RATE;
use crate::types::{MorselError, Result};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineWhisperModelConfig};
use std::path::Path;

/// One transcription request for the worker thread.
type Job = (Vec<f32>, tokio::sync::oneshot::Sender<String>);

/// Whisper runs on its own thread with the macOS QoS class `utility`. The onnxruntime threads take
/// the class of the thread that loads the model, so the call app gets the CPU first.
pub struct WhisperTranscriber {
    jobs: std::sync::mpsc::Sender<Job>,
}

impl WhisperTranscriber {
    /// Loads the model. `language` is a Whisper code such as "tl", or "auto" to detect it.
    /// sherpa-onnx sets the language when it loads the model, so a new language needs a new load.
    pub fn new(dir: &Path, language: &str) -> Result<Self> {
        if !model_ready(dir) {
            return Err(MorselError::Transcribe(
                "the Whisper model is not downloaded".into(),
            ));
        }
        let config = recognizer_config(dir, language);
        let (jobs, inbox) = std::sync::mpsc::channel::<Job>();
        let (loaded_tx, loaded) = std::sync::mpsc::channel::<bool>();
        std::thread::Builder::new()
            .name("whisper".into())
            .spawn(move || {
                set_utility_qos();
                let Some(recognizer) = OfflineRecognizer::create(&config) else {
                    let _ = loaded_tx.send(false);
                    return;
                };
                let _ = loaded_tx.send(true);
                while let Ok((samples, reply)) = inbox.recv() {
                    let stream = recognizer.create_stream();
                    stream.accept_waveform(PIPELINE_RATE as i32, &samples);
                    recognizer.decode(&stream);
                    let text = stream
                        .get_result()
                        .map(|r| r.text.trim().to_owned())
                        .unwrap_or_default();
                    let _ = reply.send(text);
                }
            })?;
        if !loaded.recv().unwrap_or(false) {
            return Err(MorselError::Transcribe(
                "sherpa-onnx cannot load the Whisper model".into(),
            ));
        }
        Ok(Self { jobs })
    }
}

fn recognizer_config(dir: &Path, language: &str) -> OfflineRecognizerConfig {
    let path = |name: &str| Some(dir.join(name).to_string_lossy().into_owned());
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.whisper = OfflineWhisperModelConfig {
        encoder: path("turbo-encoder.int8.onnx"),
        decoder: path("turbo-decoder.int8.onnx"),
        language: language_param(Some(language)).map(str::to_owned),
        task: Some("transcribe".into()),
        ..Default::default()
    };
    config.model_config.tokens = path("turbo-tokens.txt");
    config.model_config.num_threads = 4;
    config
}

/// Gives the current thread the macOS QoS class `utility`. Other systems keep their default.
fn set_utility_qos() {
    #[cfg(target_os = "macos")]
    // SAFETY: the call changes only the QoS class of the current thread.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

#[async_trait::async_trait]
impl Transcriber for WhisperTranscriber {
    async fn transcribe(&self, samples: &[f32], _language: Option<&str>) -> Result<String> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.jobs
            .send((samples.to_vec(), reply))
            .map_err(|_| MorselError::Transcribe("the Whisper thread stopped".into()))?;
        answer
            .await
            .map_err(|_| MorselError::Transcribe("the Whisper thread stopped".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::model_download;

    #[cfg(target_os = "macos")]
    #[test]
    fn set_utility_qos_sets_the_thread_class() {
        let class = std::thread::spawn(|| {
            set_utility_qos();
            let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
            let mut priority = 0;
            unsafe {
                libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut priority);
            }
            class
        })
        .join()
        .unwrap();
        assert_eq!(class as u32, libc::qos_class_t::QOS_CLASS_UTILITY as u32);
    }

    #[test]
    fn missing_model_dir_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(WhisperTranscriber::new(dir.path(), "tl").is_err());
    }

    /// Needs the model (about 1 GB): `cargo test -- --include-ignored whisper`.
    #[tokio::test]
    #[ignore]
    async fn transcribes_the_fixture_with_the_real_model() {
        let home = std::env::var("HOME").expect("HOME");
        let dir = model_download::model_dir(&model_download::models_root(
            &Path::new(&home).join("Library/Application Support/app.morsel"),
        ));
        model_download::ensure_model(&reqwest::Client::new(), &dir, |_, _| {})
            .await
            .unwrap();
        let t = WhisperTranscriber::new(&dir, "auto").unwrap();
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/hello_16k.wav"
        ))
        .unwrap();
        let data = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
        let samples: Vec<f32> = bytes[data..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| i16::from_le_bytes(b) as f32 / 32_768.0)
            .collect();
        let started = std::time::Instant::now();
        let text = t.transcribe(&samples, None).await.unwrap().to_lowercase();
        eprintln!("whisper: {:?} in {:?}", text, started.elapsed());
        assert!(text.contains("hello") && text.contains("test"), "{text}");
        // The longest chunk is 28 s. Two streams need less than half of real time for each
        // chunk, also with the QoS class `utility`: 28 s of audio in less than 14 s.
        let longest: Vec<f32> = samples.iter().cycle().take(16_000 * 28).copied().collect();
        for _ in 0..2 {
            let started = std::time::Instant::now();
            t.transcribe(&longest, None).await.unwrap();
            let took = started.elapsed();
            eprintln!("whisper: 28 s chunk in {took:?}");
            assert!(took.as_secs_f32() < 14.0, "28 s chunk took {took:?}");
        }
    }
}

//! Local transcription with Whisper Turbo (int8) through sherpa-onnx.
//! Whisper knows most languages, Tagalog too, and it reads at most 30 s of audio for each
//! call. The chunker never makes a chunk longer than 28 s.

use super::model_download::model_ready;
use super::{Transcriber, language_param};
use crate::audio::PIPELINE_RATE;
use crate::types::{MorselError, Result};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineWhisperModelConfig};
use std::path::Path;
use std::sync::Arc;

pub struct WhisperTranscriber {
    recognizer: Arc<OfflineRecognizer>,
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
        let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| {
            MorselError::Transcribe("sherpa-onnx cannot load the Whisper model".into())
        })?;
        Ok(Self {
            recognizer: Arc::new(recognizer),
        })
    }
}

#[async_trait::async_trait]
impl Transcriber for WhisperTranscriber {
    async fn transcribe(&self, samples: &[f32], _language: Option<&str>) -> Result<String> {
        let recognizer = self.recognizer.clone();
        let samples = samples.to_vec();
        tokio::task::spawn_blocking(move || {
            let stream = recognizer.create_stream();
            stream.accept_waveform(PIPELINE_RATE as i32, &samples);
            recognizer.decode(&stream);
            stream
                .get_result()
                .map(|r| r.text.trim().to_owned())
                .unwrap_or_default()
        })
        .await
        .map_err(|e| MorselError::Transcribe(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::model_download;

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
        let thirty: Vec<f32> = samples.iter().cycle().take(16_000 * 30).copied().collect();
        let started = std::time::Instant::now();
        t.transcribe(&thirty, None).await.unwrap();
        let took = started.elapsed();
        eprintln!("whisper: 30 s chunk in {took:?}");
        // Two streams need less than half of real time: 30 s of audio in less than 15 s.
        assert!(took.as_secs_f32() < 15.0, "30 s chunk took {took:?}");
    }
}

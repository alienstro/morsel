//! Local transcription with Parakeet TDT 0.6B v3 (int8) through sherpa-onnx.

use super::Transcriber;
use super::model_download::model_ready;
use crate::audio::PIPELINE_RATE;
use crate::types::{MorselError, Result};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use std::path::Path;
use std::sync::Arc;

pub struct ParakeetTranscriber {
    recognizer: Arc<OfflineRecognizer>,
}

impl ParakeetTranscriber {
    /// Loads the model from `dir`. The model must be downloaded first.
    pub fn new(dir: &Path) -> Result<Self> {
        if !model_ready(dir) {
            return Err(MorselError::Transcribe(
                "the Parakeet model is not downloaded".into(),
            ));
        }
        let path = |name: &str| Some(dir.join(name).to_string_lossy().into_owned());
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: path("encoder.int8.onnx"),
            decoder: path("decoder.int8.onnx"),
            joiner: path("joiner.int8.onnx"),
        };
        config.model_config.tokens = path("tokens.txt");
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.num_threads = 4;
        let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| {
            MorselError::Transcribe("sherpa-onnx cannot load the Parakeet model".into())
        })?;
        Ok(Self {
            recognizer: Arc::new(recognizer),
        })
    }
}

#[async_trait::async_trait]
impl Transcriber for ParakeetTranscriber {
    /// Parakeet v3 detects the language itself, so `language` is not used.
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
fn default_model_dir_for_tests() -> std::path::PathBuf {
    let home = std::env::var("HOME").expect("HOME");
    super::model_download::model_dir(
        &Path::new(&home).join("Library/Application Support/app.morsel"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::Transcriber;

    fn read_fixture() -> Vec<f32> {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/hello_16k.wav"
        ))
        .unwrap();
        let data = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
        bytes[data..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0)
            .collect()
    }

    #[test]
    fn missing_model_dir_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ParakeetTranscriber::new(dir.path()).is_err());
    }

    /// Needs the model: `cargo test -- --include-ignored parakeet`.
    #[tokio::test]
    #[ignore]
    async fn transcribes_fixture_with_words_hello_and_test() {
        let dir = default_model_dir_for_tests();
        crate::transcribe::model_download::ensure_model(&reqwest::Client::new(), &dir, |_, _| {})
            .await
            .unwrap();
        let t = ParakeetTranscriber::new(&dir).unwrap();
        let samples = read_fixture();
        let started = std::time::Instant::now();
        let text = t.transcribe(&samples, None).await.unwrap().to_lowercase();
        eprintln!("parakeet: {:?} in {:?}", text, started.elapsed());
        assert!(text.contains("hello") && text.contains("test"), "{text}");
        let thirty: Vec<f32> = samples.iter().cycle().take(16_000 * 30).copied().collect();
        let started = std::time::Instant::now();
        t.transcribe(&thirty, None).await.unwrap();
        let took = started.elapsed();
        eprintln!("parakeet: 30 s chunk in {took:?}");
        // Two streams need less than half of real time: 30 s of audio in less than 10 s.
        assert!(took.as_secs_f32() < 10.0, "30 s chunk took {took:?}");
    }
}

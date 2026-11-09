//! Speech to text for one chunk: local Whisper Turbo or a cloud endpoint.

pub mod after;
pub mod cloud;
pub mod model_download;
pub mod whisper;

use crate::audio::PIPELINE_RATE;
#[cfg(test)]
use crate::types::MorselError;
use crate::types::Result;
use std::time::Duration;

/// Waits before the second and the third try.
const RETRY_WAITS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(4)];

#[async_trait::async_trait]
pub trait Transcriber: Send + Sync {
    /// Transcribes 16 kHz mono samples. `language` "auto" or `None` lets the model detect it.
    async fn transcribe(&self, samples: &[f32], language: Option<&str>) -> Result<String>;
}

/// Tries 3 times, with waits of 2 s and 4 s. Returns `None` when all tries fail.
pub async fn transcribe_with_retry(
    t: &dyn Transcriber,
    samples: &[f32],
    language: Option<&str>,
) -> Option<String> {
    for attempt in 0..=RETRY_WAITS.len() {
        match t.transcribe(samples, language).await {
            Ok(text) => return Some(text),
            Err(err) => {
                eprintln!("transcription try {} failed: {err}", attempt + 1);
                if let Some(wait) = RETRY_WAITS.get(attempt) {
                    tokio::time::sleep(*wait).await;
                }
            }
        }
    }
    None
}

/// A 16-bit PCM WAV file in memory. Audio never goes to disk.
pub fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&PIPELINE_RATE.to_le_bytes());
    out.extend_from_slice(&(PIPELINE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// The language to send, or `None` for automatic detection.
pub fn language_param(language: Option<&str>) -> Option<&str> {
    language.filter(|l| !l.is_empty() && *l != "auto")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Flaky {
        calls: AtomicUsize,
        fail_first: usize,
    }

    #[async_trait::async_trait]
    impl Transcriber for Flaky {
        async fn transcribe(&self, _samples: &[f32], _language: Option<&str>) -> Result<String> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.fail_first {
                Err(MorselError::Http("down".into()))
            } else {
                Ok("hello".into())
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn retries_twice_then_returns_none() {
        let t = Flaky {
            calls: AtomicUsize::new(0),
            fail_first: 10,
        };
        let started = tokio::time::Instant::now();
        assert_eq!(transcribe_with_retry(&t, &[0.1; 10], None).await, None);
        assert_eq!(t.calls.load(Ordering::SeqCst), 3);
        assert_eq!(started.elapsed().as_secs(), 6);
    }

    #[tokio::test(start_paused = true)]
    async fn second_try_success_returns_text() {
        let t = Flaky {
            calls: AtomicUsize::new(0),
            fail_first: 1,
        };
        assert_eq!(
            transcribe_with_retry(&t, &[0.1; 10], None).await.as_deref(),
            Some("hello")
        );
        assert_eq!(t.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn wav_header_says_16000_hz_mono() {
        let wav = wav_bytes(&[0.0, 0.5, -0.5, 1.0]);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            16_000
        );
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16);
        assert_eq!(wav.len(), 44 + 4 * 2);
        assert_eq!(i16::from_le_bytes([wav[46], wav[47]]), 16_383);
    }
}

//! Cuts one stream into chunks of 10 to 30 seconds, at a silence when it can.

use crate::audio::{PIPELINE_RATE, level::rms};
use crate::types::{AudioChunk, Speaker};

const RATE: usize = PIPELINE_RATE as usize;
/// Silence is measured over frames of 30 ms.
const FRAME: usize = RATE * 30 / 1000;
const SILENCE_RMS: f32 = 0.01;
const MIN_SAMPLES: usize = RATE * 10;
const MAX_SAMPLES: usize = RATE * 30;
const CUT_SILENCE_SAMPLES: usize = RATE * 400 / 1000;

pub struct Chunker {
    speaker: Speaker,
    /// Samples that are not yet a full 30 ms frame.
    partial: Vec<f32>,
    buffer: Vec<f32>,
    buffer_start: usize,
    total: usize,
    silent_run: usize,
    has_voice: bool,
}

impl Chunker {
    pub fn new(speaker: Speaker) -> Self {
        Self {
            speaker,
            partial: Vec::new(),
            buffer: Vec::new(),
            buffer_start: 0,
            total: 0,
            silent_run: 0,
            has_voice: false,
        }
    }

    /// Adds 16 kHz mono samples. Returns the chunks that are complete.
    pub fn push(&mut self, samples: &[f32]) -> Vec<AudioChunk> {
        self.partial.extend_from_slice(samples);
        let mut out = Vec::new();
        let full = self.partial.len() / FRAME * FRAME;
        let frames: Vec<f32> = self.partial.drain(..full).collect();
        for frame in frames.chunks_exact(FRAME) {
            if rms(frame) < SILENCE_RMS {
                self.silent_run += FRAME;
            } else {
                self.silent_run = 0;
                self.has_voice = true;
            }
            self.buffer.extend_from_slice(frame);
            self.total += FRAME;
            let len = self.buffer.len();
            if len >= MAX_SAMPLES || (len >= MIN_SAMPLES && self.silent_run >= CUT_SILENCE_SAMPLES)
            {
                out.extend(self.cut());
            }
        }
        out
    }

    /// Returns the rest of the stream as a last chunk, if it has voice.
    pub fn flush(&mut self) -> Option<AudioChunk> {
        let rest = std::mem::take(&mut self.partial);
        if rms(&rest) >= SILENCE_RMS {
            self.has_voice = true;
        }
        self.total += rest.len();
        self.buffer.extend(rest);
        self.cut()
    }

    fn cut(&mut self) -> Option<AudioChunk> {
        let samples = std::mem::take(&mut self.buffer);
        let start = self.buffer_start;
        self.buffer_start = self.total;
        self.silent_run = 0;
        let has_voice = std::mem::replace(&mut self.has_voice, false);
        if samples.is_empty() || !has_voice {
            return None;
        }
        Some(AudioChunk {
            speaker: self.speaker,
            start_ms: (start * 1000 / RATE) as u64,
            samples,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: usize = 16_000;

    fn speech(seconds: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize)
            .map(|i| (i as f32 * 0.07).sin() * 0.3)
            .collect()
    }

    fn silence(seconds: f32) -> Vec<f32> {
        vec![0.0; (seconds * RATE as f32) as usize]
    }

    fn secs(chunk: &AudioChunk) -> f32 {
        chunk.samples.len() as f32 / RATE as f32
    }

    fn push_all(chunker: &mut Chunker, parts: &[Vec<f32>]) -> Vec<AudioChunk> {
        let mut out = Vec::new();
        for part in parts {
            for piece in part.chunks(1600) {
                out.extend(chunker.push(piece));
            }
        }
        out
    }

    #[test]
    fn cuts_at_first_400ms_silence_after_10s() {
        let mut c = Chunker::new(Speaker::Others);
        let chunks = push_all(&mut c, &[speech(12.0), silence(0.5), speech(5.0)]);
        assert_eq!(chunks.len(), 1);
        let len = secs(&chunks[0]);
        assert!((12.0..=12.5).contains(&len), "first chunk is {len} s");
        assert_eq!(chunks[0].speaker, Speaker::Others);
    }

    #[test]
    fn never_cuts_before_10s() {
        let mut c = Chunker::new(Speaker::You);
        let chunks = push_all(&mut c, &[speech(5.0), silence(1.0), speech(4.0)]);
        assert!(chunks.is_empty());
        let rest = c.flush().unwrap();
        assert!((secs(&rest) - 10.0).abs() < 0.01);
    }

    #[test]
    fn always_cuts_at_30s() {
        let mut c = Chunker::new(Speaker::You);
        let chunks = push_all(&mut c, &[speech(35.0)]);
        assert_eq!(chunks.len(), 1);
        assert!((secs(&chunks[0]) - 30.0).abs() < 0.01);
        assert!((secs(&c.flush().unwrap()) - 5.0).abs() < 0.01);
    }

    #[test]
    fn drops_all_silent_chunk() {
        let mut c = Chunker::new(Speaker::You);
        let chunks = push_all(&mut c, &[silence(12.0)]);
        assert!(chunks.is_empty());
        assert!(c.flush().is_none());
    }

    #[test]
    fn start_ms_of_second_chunk_equals_length_of_first() {
        let mut c = Chunker::new(Speaker::You);
        let chunks = push_all(
            &mut c,
            &[speech(12.0), silence(0.5), speech(12.0), silence(0.5)],
        );
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].start_ms, 0);
        let first_ms = (chunks[0].samples.len() * 1000 / RATE) as u64;
        assert_eq!(chunks[1].start_ms, first_ms);
    }

    #[test]
    fn dropped_silence_still_moves_time_forward() {
        let mut c = Chunker::new(Speaker::You);
        let chunks = push_all(&mut c, &[silence(10.5), speech(10.0), silence(0.5)]);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].start_ms >= 10_000, "start {}", chunks[0].start_ms);
    }

    #[test]
    fn flush_returns_the_rest() {
        let mut c = Chunker::new(Speaker::You);
        assert!(push_all(&mut c, &[speech(3.0)]).is_empty());
        let rest = c.flush().unwrap();
        assert!((secs(&rest) - 3.0).abs() < 0.01);
        assert!(c.flush().is_none());
    }
}

//! Converts device audio (any rate, any channel count) to 16 kHz mono.

use super::PIPELINE_RATE;
use crate::types::{MorselError, Result};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler as _};

const CHUNK_FRAMES: usize = 1024;

/// Mixes all channels to mono, then resamples to 16 kHz with an FFT resampler.
pub struct Resampler {
    channels: usize,
    inner: Option<Fft<f32>>,
    pending: Vec<f32>,
    skip: usize,
    frames_in: usize,
    frames_out: usize,
    input_rate: u32,
}

impl Resampler {
    pub fn new(input_rate: u32, channels: u16) -> Result<Self> {
        if channels == 0 || input_rate == 0 {
            return Err(MorselError::Audio("the device has no channels or no sample rate".into()));
        }
        let inner = if input_rate == PIPELINE_RATE {
            None
        } else {
            Some(
                Fft::<f32>::new(input_rate as usize, PIPELINE_RATE as usize, CHUNK_FRAMES, 1, FixedSync::Input)
                    .map_err(|e| MorselError::Audio(e.to_string()))?,
            )
        };
        let skip = inner.as_ref().map(|r| r.output_delay()).unwrap_or(0);
        Ok(Self { channels: channels as usize, inner, pending: Vec::new(), skip, frames_in: 0, frames_out: 0, input_rate })
    }

    /// Takes interleaved device samples and returns the 16 kHz mono samples that are ready.
    pub fn process(&mut self, interleaved: &[f32]) -> Vec<f32> {
        let mono = self.to_mono(interleaved);
        self.frames_in += mono.len();
        if self.inner.is_none() {
            return mono;
        }
        self.pending.extend(mono);
        let mut out = Vec::new();
        while self.pending.len() >= CHUNK_FRAMES {
            let chunk: Vec<f32> = self.pending.drain(..CHUNK_FRAMES).collect();
            out.extend(self.run(&chunk, None));
        }
        self.emit(out)
    }

    /// Returns the rest of the audio. Call it once, when the stream ends.
    pub fn flush(&mut self) -> Vec<f32> {
        if self.inner.is_none() {
            return Vec::new();
        }
        let rest: Vec<f32> = std::mem::take(&mut self.pending);
        let mut padded = rest.clone();
        padded.resize(CHUNK_FRAMES, 0.0);
        let mut out = self.run(&padded, Some(rest.len()));
        // One more empty chunk pushes the resampler delay out.
        out.extend(self.run(&vec![0.0; CHUNK_FRAMES], Some(0)));
        let expected = (self.frames_in as u64 * PIPELINE_RATE as u64 / self.input_rate as u64) as usize;
        let mut out = self.emit(out);
        out.truncate(expected.saturating_sub(self.frames_out - out.len()));
        out
    }

    fn to_mono(&self, interleaved: &[f32]) -> Vec<f32> {
        if self.channels == 1 {
            return interleaved.to_vec();
        }
        interleaved
            .chunks_exact(self.channels)
            .map(|frame| frame.iter().sum::<f32>() / self.channels as f32)
            .collect()
    }

    fn run(&mut self, chunk: &[f32], partial_len: Option<usize>) -> Vec<f32> {
        let Some(inner) = self.inner.as_mut() else { return Vec::new() };
        let mut out = vec![0.0f32; inner.output_frames_max()];
        let capacity = out.len();
        let input = InterleavedSlice::new(chunk, 1, chunk.len()).expect("input buffer");
        let mut output = InterleavedSlice::new_mut(&mut out, 1, capacity).expect("output buffer");
        let indexing = Indexing { input_offset: 0, output_offset: 0, partial_len, active_channels_mask: None };
        match inner.process_into_buffer(&input, &mut output, Some(&indexing)) {
            Ok((_, written)) => {
                out.truncate(written);
                out
            }
            Err(_) => Vec::new(),
        }
    }

    fn emit(&mut self, mut out: Vec<f32>) -> Vec<f32> {
        let drop = self.skip.min(out.len());
        self.skip -= drop;
        out.drain(..drop);
        self.frames_out += out.len();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, channels: u16, seconds: f32) -> Vec<f32> {
        let frames = (rate as f32 * seconds) as usize;
        let mut out = Vec::with_capacity(frames * channels as usize);
        for i in 0..frames {
            let v = (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5;
            for _ in 0..channels {
                out.push(v);
            }
        }
        out
    }

    #[test]
    fn stereo_48k_one_second_gives_16000_samples() {
        let input = sine(48_000, 2, 1.0);
        let mut r = Resampler::new(48_000, 2).unwrap();
        let mut out = Vec::new();
        for piece in input.chunks(960) {
            out.extend(r.process(piece));
        }
        out.extend(r.flush());
        let n = out.len() as f32;
        assert!((15_840.0..=16_160.0).contains(&n), "got {n} samples");
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.4..0.6).contains(&peak), "peak {peak}");
    }

    #[test]
    fn mono_16k_passes_through() {
        let input = sine(16_000, 1, 0.1);
        let mut r = Resampler::new(16_000, 1).unwrap();
        assert_eq!(r.process(&input), input);
        assert!(r.flush().is_empty());
    }

    #[test]
    fn zero_channels_is_an_error() {
        assert!(Resampler::new(48_000, 0).is_err());
    }
}

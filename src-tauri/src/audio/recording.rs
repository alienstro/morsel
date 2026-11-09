//! The saved audio of a meeting: one 16 kHz mono 16-bit WAV file for each speaker.

use super::PIPELINE_RATE;
use crate::types::{MeetingId, MorselError, Result};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const BYTES_PER_SAMPLE: u64 = 2;
/// The writer updates the WAV header about once a second, so a crash loses little audio.
const FLUSH_SAMPLES: usize = PIPELINE_RATE as usize;

/// The folder of the saved audio of one meeting.
pub fn audio_dir(data_dir: &Path, id: MeetingId) -> PathBuf {
    data_dir.join("audio").join(id.to_string())
}

fn spec() -> hound::WavSpec {
    hound::WavSpec {
        channels: 1,
        sample_rate: PIPELINE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

fn wav_error(err: hound::Error) -> MorselError {
    MorselError::Audio(format!("cannot write the meeting audio: {err}"))
}

/// Writes one stream of 16 kHz mono audio to a WAV file.
pub struct StreamWriter {
    wav: hound::WavWriter<BufWriter<File>>,
    unflushed: usize,
}

impl StreamWriter {
    /// Creates the file and its folders.
    pub fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let wav = hound::WavWriter::create(path, spec()).map_err(wav_error)?;
        Ok(Self { wav, unflushed: 0 })
    }

    pub fn write(&mut self, samples: &[f32]) -> Result<()> {
        for &s in samples {
            let value = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            self.wav.write_sample(value).map_err(wav_error)?;
        }
        self.unflushed += samples.len();
        if self.unflushed >= FLUSH_SAMPLES {
            self.wav.flush().map_err(wav_error)?;
            self.unflushed = 0;
        }
        Ok(())
    }

    /// Writes the final header.
    pub fn finish(self) -> Result<()> {
        self.wav.finalize().map_err(wav_error)
    }
}

/// Reads a WAV file of `StreamWriter` in pieces. It takes the length from the file size, not
/// from the header, so a file that a crash left open is still readable.
pub struct StreamReader {
    file: BufReader<File>,
    data_start: u64,
    samples: u64,
    position: u64,
}

impl StreamReader {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        let mut head = vec![0u8; 4_096.min(size as usize)];
        file.read_exact(&mut head)?;
        let data = head
            .windows(4)
            .position(|w| w == b"data")
            .ok_or_else(|| MorselError::Audio(format!("{} is not a WAV file", path.display())))?;
        let data_start = data as u64 + 8;
        file.seek(SeekFrom::Start(data_start))?;
        Ok(Self {
            file: BufReader::new(file),
            data_start,
            samples: size.saturating_sub(data_start) / BYTES_PER_SAMPLE,
            position: 0,
        })
    }

    pub fn duration_ms(&self) -> u64 {
        self.samples * 1_000 / PIPELINE_RATE as u64
    }

    /// Moves to `ms` from the start of the audio.
    pub fn seek_ms(&mut self, ms: u64) -> Result<()> {
        self.position = (ms * PIPELINE_RATE as u64 / 1_000).min(self.samples);
        self.file.seek(SeekFrom::Start(
            self.data_start + self.position * BYTES_PER_SAMPLE,
        ))?;
        Ok(())
    }

    /// Returns at most `max` samples. Returns no samples at the end of the audio.
    pub fn read(&mut self, max: usize) -> Result<Vec<f32>> {
        let count = (max as u64).min(self.samples - self.position) as usize;
        let mut bytes = vec![0u8; count * BYTES_PER_SAMPLE as usize];
        self.file.read_exact(&mut bytes)?;
        self.position += count as u64;
        Ok(bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| i16::from_le_bytes(b) as f32 / i16::MAX as f32)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone that survives the 16-bit round trip with a small error.
    fn tone(seconds: f32) -> Vec<f32> {
        (0..(seconds * 16_000.0) as usize)
            .map(|i| (i as f32 * 0.05).sin() * 0.5)
            .collect()
    }

    fn write_file(path: &Path, samples: &[f32]) {
        let mut w = StreamWriter::create(path).unwrap();
        for piece in samples.chunks(1_600) {
            w.write(piece).unwrap();
        }
        w.finish().unwrap();
    }

    fn read_all(r: &mut StreamReader) -> Vec<f32> {
        let mut all = Vec::new();
        loop {
            let piece = r.read(16_000).unwrap();
            if piece.is_empty() {
                return all;
            }
            all.extend(piece);
        }
    }

    #[test]
    fn audio_dir_is_one_folder_for_each_meeting() {
        assert_eq!(
            audio_dir(Path::new("/data"), 42),
            Path::new("/data/audio/42")
        );
    }

    #[test]
    fn writes_and_reads_back_the_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio/1/you.wav");
        let samples = tone(2.0);
        write_file(&path, &samples);
        let mut r = StreamReader::open(&path).unwrap();
        assert_eq!(r.duration_ms(), 2_000);
        let back = read_all(&mut r);
        assert_eq!(back.len(), samples.len());
        assert!(
            back.iter()
                .zip(&samples)
                .all(|(a, b)| (a - b).abs() < 0.001)
        );
    }

    #[test]
    fn a_wav_with_a_wrong_header_length_is_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("you.wav");
        write_file(&path, &tone(1.5));
        // A crash leaves the lengths of the header at 0.
        let mut bytes = std::fs::read(&path).unwrap();
        let data = bytes.windows(4).position(|w| w == b"data").unwrap();
        bytes[4..8].copy_from_slice(&[0; 4]);
        bytes[data + 4..data + 8].copy_from_slice(&[0; 4]);
        std::fs::write(&path, bytes).unwrap();
        let mut r = StreamReader::open(&path).unwrap();
        assert_eq!(r.duration_ms(), 1_500);
        assert_eq!(read_all(&mut r).len(), 24_000);
    }

    #[test]
    fn reader_yields_audio_in_pieces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("you.wav");
        write_file(&path, &tone(1.0));
        let mut r = StreamReader::open(&path).unwrap();
        let sizes: Vec<usize> = std::iter::from_fn(|| {
            let piece = r.read(6_000).unwrap();
            (!piece.is_empty()).then_some(piece.len())
        })
        .collect();
        assert_eq!(sizes, [6_000, 6_000, 4_000]);
    }

    #[test]
    fn seek_ms_skips_the_audio_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("you.wav");
        write_file(&path, &tone(2.0));
        let mut r = StreamReader::open(&path).unwrap();
        r.seek_ms(1_250).unwrap();
        assert_eq!(read_all(&mut r).len(), 12_000);
    }
}

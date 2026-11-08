//! Audio capture and processing. Audio stays in memory only.

pub mod level;
pub mod resample;

/// The sample rate of all audio inside the pipeline.
pub const PIPELINE_RATE: u32 = 16_000;

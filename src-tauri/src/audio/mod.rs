//! Audio capture and processing. Audio stays in memory only.

pub mod level;
pub mod mic;
pub mod resample;
pub mod source;
pub mod system_tap;

/// The sample rate of all audio inside the pipeline.
pub const PIPELINE_RATE: u32 = 16_000;

//! The common interface of the two audio sources.

use crate::types::Result;
use tokio::sync::mpsc::Sender;

/// A source of 16 kHz mono audio. A source is used for one meeting only.
pub trait AudioSource: Send {
    /// Starts the capture. Frames go to `tx`. Returns an error when the capture cannot start.
    fn start(&mut self, tx: Sender<Vec<f32>>) -> Result<()>;
    /// Stops the capture and releases the device.
    fn stop(&mut self);
}

//! Event names and the sink that sends events to the frontend.

use serde_json::Value;
use std::sync::Mutex;

pub const TRANSCRIPT: &str = "meeting://transcript";
pub const LEVELS: &str = "meeting://levels";
pub const STATUS: &str = "meeting://status";
pub const NOTES_DONE: &str = "notes://done";
pub const NOTES_FAILED: &str = "notes://failed";
pub const MODEL_DOWNLOAD: &str = "model://download";

pub trait EventSink: Send + Sync {
    fn emit(&self, name: &str, payload: Value);
}

/// Keeps every event in memory. For tests.
#[derive(Default)]
pub struct VecSink {
    events: Mutex<Vec<(String, Value)>>,
}

impl VecSink {
    /// The payloads of the events with this name, in order.
    pub fn named(&self, name: &str) -> Vec<Value> {
        self.events.lock().expect("lock").iter().filter(|(n, _)| n == name).map(|(_, v)| v.clone()).collect()
    }
}

impl EventSink for VecSink {
    fn emit(&self, name: &str, payload: Value) {
        self.events.lock().expect("lock").push((name.to_owned(), payload));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_sink_keeps_events_in_order() {
        let sink = VecSink::default();
        sink.emit(TRANSCRIPT, serde_json::json!({ "text": "a" }));
        sink.emit(STATUS, serde_json::json!({ "status": "recording" }));
        assert_eq!(sink.named(TRANSCRIPT).len(), 1);
        assert_eq!(sink.named(STATUS)[0]["status"], "recording");
    }
}

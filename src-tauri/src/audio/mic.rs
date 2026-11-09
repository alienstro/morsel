//! The microphone: the "You" stream.

use super::resample::Resampler;
use super::source::AudioSource;
use crate::types::{MorselError, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::mpsc::Sender;

/// Receives 16 kHz mono frames from a device callback.
pub type SampleSink = Box<dyn FnMut(Vec<f32>) + Send + 'static>;
/// Keeps a device stream open. Dropping it closes the stream.
pub type StreamGuard = Box<dyn std::any::Any>;

/// Opens the current default input device.
pub trait StreamFactory: Send + 'static {
    fn open(&mut self, on_samples: SampleSink) -> Result<StreamGuard>;
}

/// Tells when the default input device changed since the last call.
pub trait DeviceWatcher: Send + 'static {
    fn default_input_changed(&mut self) -> bool;
}

pub struct MicSource {
    parts: Option<(Box<dyn StreamFactory>, Box<dyn DeviceWatcher>)>,
    poll: Duration,
    stop_tx: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl MicSource {
    /// The real microphone: cpal, default input device, device check each second.
    pub fn default_device() -> Self {
        Self::with_parts(
            Box::new(CpalInputFactory),
            Box::new(CpalInputWatcher::new()),
            Duration::from_secs(1),
        )
    }

    pub fn with_parts(
        factory: Box<dyn StreamFactory>,
        watcher: Box<dyn DeviceWatcher>,
        poll: Duration,
    ) -> Self {
        Self {
            parts: Some((factory, watcher)),
            poll,
            stop_tx: None,
            thread: None,
        }
    }
}

fn sink_for(tx: &Sender<Vec<f32>>) -> SampleSink {
    let tx = tx.clone();
    // The device callback must never block, so a full channel drops the frame.
    Box::new(move |samples| {
        let _ = tx.try_send(samples);
    })
}

impl AudioSource for MicSource {
    fn start(&mut self, tx: Sender<Vec<f32>>) -> Result<()> {
        let (mut factory, mut watcher) = self
            .parts
            .take()
            .ok_or_else(|| MorselError::Audio("the microphone source was already used".into()))?;
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let poll = self.poll;
        let thread = std::thread::spawn(move || {
            let mut guard = match factory.open(sink_for(&tx)) {
                Ok(guard) => {
                    let _ = ready_tx.send(Ok(()));
                    Some(guard)
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err));
                    return;
                }
            };
            loop {
                match stop_rx.recv_timeout(poll) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                if watcher.default_input_changed() || guard.is_none() {
                    drop(guard.take());
                    // A failed reopen is tried again at the next poll.
                    guard = factory.open(sink_for(&tx)).ok();
                }
            }
            drop(guard);
        });
        let ready = ready_rx
            .recv()
            .unwrap_or_else(|_| Err(MorselError::Audio("the microphone thread stopped".into())));
        if ready.is_ok() {
            self.stop_tx = Some(stop_tx);
            self.thread = Some(thread);
        } else {
            let _ = thread.join();
        }
        ready
    }

    fn stop(&mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for MicSource {
    fn drop(&mut self) {
        self.stop();
    }
}

fn map_cpal_error(err: cpal::Error) -> MorselError {
    if matches!(err.kind(), cpal::ErrorKind::PermissionDenied) {
        MorselError::MicPermissionDenied
    } else {
        MorselError::Audio(err.to_string())
    }
}

/// Opens the default input device with cpal and resamples to 16 kHz mono.
pub struct CpalInputFactory;

impl StreamFactory for CpalInputFactory {
    fn open(&mut self, mut on_samples: SampleSink) -> Result<StreamGuard> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or_else(|| MorselError::Audio("no microphone is connected".into()))?;
        let supported = device.default_input_config().map_err(map_cpal_error)?;
        let config = supported.config();
        let mut resampler = Resampler::new(config.sample_rate, config.channels)?;
        let on_error = |err: cpal::Error| eprintln!("microphone stream error: {err}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
                config,
                move |data: &[f32], _| {
                    let out = resampler.process(data);
                    if !out.is_empty() {
                        on_samples(out);
                    }
                },
                on_error,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
                config,
                move |data: &[i16], _| {
                    let floats: Vec<f32> = data.iter().map(|v| *v as f32 / 32_768.0).collect();
                    let out = resampler.process(&floats);
                    if !out.is_empty() {
                        on_samples(out);
                    }
                },
                on_error,
                None,
            ),
            other => {
                return Err(MorselError::Audio(format!(
                    "the microphone format {other:?} is not supported"
                )));
            }
        }
        .map_err(map_cpal_error)?;
        stream.play().map_err(map_cpal_error)?;
        Ok(Box::new(stream))
    }
}

/// Compares the id of the default input device with the last one.
pub struct CpalInputWatcher {
    last: Option<cpal::DeviceId>,
}

impl CpalInputWatcher {
    pub fn new() -> Self {
        Self {
            last: Self::current(),
        }
    }

    fn current() -> Option<cpal::DeviceId> {
        cpal::default_host()
            .default_input_device()
            .and_then(|d| d.id().ok())
    }
}

impl Default for CpalInputWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceWatcher for CpalInputWatcher {
    fn default_input_changed(&mut self) -> bool {
        let now = Self::current();
        if now != self.last {
            self.last = now;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeFactory {
        opens: Arc<AtomicUsize>,
        live: Arc<AtomicUsize>,
    }

    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl StreamFactory for FakeFactory {
        fn open(&mut self, mut on_samples: SampleSink) -> Result<StreamGuard> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            self.live.fetch_add(1, Ordering::SeqCst);
            on_samples(vec![0.1; 160]);
            Ok(Box::new(Guard(self.live.clone())))
        }
    }

    struct FakeWatcher {
        answers: Vec<bool>,
    }

    impl DeviceWatcher for FakeWatcher {
        fn default_input_changed(&mut self) -> bool {
            if self.answers.is_empty() {
                false
            } else {
                self.answers.remove(0)
            }
        }
    }

    fn source(answers: Vec<bool>) -> (MicSource, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let opens = Arc::new(AtomicUsize::new(0));
        let live = Arc::new(AtomicUsize::new(0));
        let factory = FakeFactory {
            opens: opens.clone(),
            live: live.clone(),
        };
        let src = MicSource::with_parts(
            Box::new(factory),
            Box::new(FakeWatcher { answers }),
            Duration::from_millis(5),
        );
        (src, opens, live)
    }

    #[tokio::test]
    async fn restarts_on_default_device_change() {
        let (mut src, opens, _live) = source(vec![true]);
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        src.start(tx).unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(opens.load(Ordering::SeqCst), 2);
        assert_eq!(rx.recv().await.unwrap().len(), 160);
        assert_eq!(rx.recv().await.unwrap().len(), 160);
        src.stop();
    }

    #[tokio::test]
    async fn stop_ends_the_stream() {
        let (mut src, _opens, live) = source(vec![]);
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        src.start(tx).unwrap();
        assert_eq!(live.load(Ordering::SeqCst), 1);
        src.stop();
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn open_error_is_returned_by_start() {
        struct Denied;
        impl StreamFactory for Denied {
            fn open(&mut self, _on: SampleSink) -> Result<StreamGuard> {
                Err(MorselError::MicPermissionDenied)
            }
        }
        let mut src = MicSource::with_parts(
            Box::new(Denied),
            Box::new(FakeWatcher { answers: vec![] }),
            Duration::from_millis(5),
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        assert!(matches!(
            src.start(tx),
            Err(MorselError::MicPermissionDenied)
        ));
    }
}

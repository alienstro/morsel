//! Owns the one loaded local notes model, and enforces the memory rules for an 8 GB Mac.

use crate::chat_model::{Budget, ChatModel};
use crate::llm::{ChatMessage, ChatResult};
use crate::local_llm::{LOCAL_MODEL_NAME, LocalLlm};
use crate::types::{MorselError, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The manager frees the model after this time with no use.
pub const IDLE: Duration = Duration::from_secs(300);
/// The answer limits. With the local budget of 6,000 prompt tokens, they fit the 8,192-token context.
const NOTES_MAX_TOKENS: u32 = 1536;
const ASK_MAX_TOKENS: u32 = 768;
pub const WHISPER_BUSY: &str = "Ask is available when the transcription ends.";

/// A model that writes an answer. This call blocks.
pub trait Generate: Send + Sync + 'static {
    fn complete(
        &self,
        messages: &[ChatMessage],
        max_tokens: u32,
        cancel: &AtomicBool,
    ) -> Result<ChatResult>;
}

impl Generate for LocalLlm {
    fn complete(
        &self,
        messages: &[ChatMessage],
        max_tokens: u32,
        cancel: &AtomicBool,
    ) -> Result<ChatResult> {
        LocalLlm::complete(self, messages, max_tokens, cancel)
    }
}

/// The on-device Whisper model, seen from the notes side.
pub trait WhisperControl: Send + Sync {
    /// True while a recording or a transcription uses Whisper.
    fn in_use(&self) -> bool;
    /// Frees the cached Whisper model when nothing uses it.
    fn free(&self);
}

type Loader<T> = Box<dyn Fn(&Path) -> Result<Arc<T>> + Send + Sync>;
type Slot<T> = Arc<Mutex<Option<(PathBuf, Arc<T>)>>>;

pub struct ModelManager<T> {
    loader: Loader<T>,
    idle: Duration,
    loaded: Slot<T>,
    /// Each use bumps it. An idle timer frees the model only if no use came after it started.
    generation: Arc<AtomicU64>,
    /// One generation at a time: llama.cpp must not make two contexts from one model in parallel,
    /// and two contexts do not fit in 8 GB.
    run_lock: Mutex<()>,
}

impl<T: Send + Sync + 'static> ModelManager<T> {
    pub fn new(loader: Loader<T>, idle: Duration) -> Self {
        Self {
            loader,
            idle,
            loaded: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
            run_lock: Mutex::new(()),
        }
    }

    /// Runs `job` with the model, after any job that runs now. This call blocks.
    pub fn run<R>(&self, path: &Path, job: impl FnOnce(&T) -> Result<R>) -> Result<R> {
        let _turn = self.run_lock.lock().expect("run lock");
        job(self.get(path)?.as_ref())
    }

    /// Returns the loaded model, and loads it first when it is not loaded. This call can block for seconds.
    pub fn get(&self, path: &Path) -> Result<Arc<T>> {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut slot = self.loaded.lock().expect("model lock");
        if let Some((loaded_path, model)) = slot.as_ref()
            && loaded_path == path
        {
            return Ok(model.clone());
        }
        // Free the old model before the new one loads, so that two never use memory at the same time.
        *slot = None;
        let model = (self.loader)(path)?;
        *slot = Some((path.to_owned(), model.clone()));
        Ok(model)
    }

    /// Starts the idle time again. Call it after each use.
    pub fn touch(&self) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let (counter, loaded, idle) = (self.generation.clone(), self.loaded.clone(), self.idle);
        runtime.spawn(async move {
            tokio::time::sleep(idle).await;
            if counter.load(Ordering::SeqCst) == generation {
                *loaded.lock().expect("model lock") = None;
            }
        });
    }

    /// Frees the model now. A call that still runs keeps its own reference until it ends.
    pub fn unload(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self.loaded.lock().expect("model lock") = None;
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded.lock().expect("model lock").is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Notes,
    Ask,
}

/// The local notes model behind the `ChatModel` interface.
pub struct LocalChat<T: Generate> {
    manager: Arc<ModelManager<T>>,
    path: PathBuf,
    whisper: Arc<dyn WhisperControl>,
    purpose: Purpose,
}

impl<T: Generate> LocalChat<T> {
    pub fn new(
        manager: Arc<ModelManager<T>>,
        path: PathBuf,
        whisper: Arc<dyn WhisperControl>,
        purpose: Purpose,
    ) -> Self {
        Self {
            manager,
            path,
            whisper,
            purpose,
        }
    }
}

#[async_trait]
impl<T: Generate> ChatModel for LocalChat<T> {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        cancel: &CancellationToken,
    ) -> Result<ChatResult> {
        if cancel.is_cancelled() {
            return Err(MorselError::Cancelled);
        }
        if self.purpose == Purpose::Ask && self.whisper.in_use() {
            return Err(MorselError::Invalid(WHISPER_BUSY.into()));
        }
        self.whisper.free();
        let max_tokens = match self.purpose {
            Purpose::Notes => NOTES_MAX_TOKENS,
            Purpose::Ask => ASK_MAX_TOKENS,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let (manager, path, messages, job_stop) = (
            self.manager.clone(),
            self.path.clone(),
            messages.to_vec(),
            stop.clone(),
        );
        let mut job = tokio::task::spawn_blocking(move || {
            manager.run(&path, |model| {
                model.complete(&messages, max_tokens, &job_stop)
            })
        });
        let result = tokio::select! {
            r = &mut job => r.map_err(|e| MorselError::Invalid(format!("the local notes model stopped: {e}")))?,
            _ = cancel.cancelled() => {
                // The blocking job sees the flag at its next token and ends.
                stop.store(true, Ordering::Relaxed);
                Err(MorselError::Cancelled)
            }
        };
        self.manager.touch();
        result
    }

    fn name(&self) -> &str {
        LOCAL_MODEL_NAME
    }

    fn budget(&self) -> Budget {
        Budget::LOCAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_model::{Budget, ChatModel};
    use crate::llm::{ChatMessage, ChatResult};
    use crate::types::Result;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    struct Echo;

    impl Generate for Echo {
        fn complete(&self, _m: &[ChatMessage], _max: u32, _c: &AtomicBool) -> Result<ChatResult> {
            Ok(ChatResult {
                text: "answer".into(),
                prompt_tokens: 3,
                completion_tokens: 1,
                cost: None,
            })
        }
    }

    fn manager(loads: Arc<AtomicUsize>) -> Arc<ModelManager<Echo>> {
        Arc::new(ModelManager::new(
            Box::new(move |_p: &Path| {
                loads.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(Echo))
            }),
            Duration::from_secs(300),
        ))
    }

    #[derive(Default)]
    struct FakeWhisper {
        in_use: AtomicBool,
        freed: AtomicUsize,
    }

    impl WhisperControl for FakeWhisper {
        fn in_use(&self) -> bool {
            self.in_use.load(Ordering::SeqCst)
        }
        fn free(&self) {
            self.freed.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn user(text: &str) -> Vec<ChatMessage> {
        vec![ChatMessage {
            role: "user".into(),
            content: text.into(),
        }]
    }

    #[tokio::test(start_paused = true)]
    async fn manager_loads_once_and_frees_after_idle() {
        let loads = Arc::new(AtomicUsize::new(0));
        let m = manager(loads.clone());
        m.get(Path::new("/m.gguf")).unwrap();
        m.get(Path::new("/m.gguf")).unwrap();
        m.touch();
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert!(m.is_loaded());
        tokio::time::sleep(Duration::from_secs(299)).await;
        assert!(m.is_loaded());
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(!m.is_loaded());
        m.get(Path::new("/m.gguf")).unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_use_restarts_the_idle_time() {
        let m = manager(Arc::new(AtomicUsize::new(0)));
        m.get(Path::new("/m.gguf")).unwrap();
        m.touch();
        tokio::time::sleep(Duration::from_secs(200)).await;
        m.touch();
        tokio::time::sleep(Duration::from_secs(200)).await;
        assert!(m.is_loaded());
    }

    #[test]
    fn unload_frees_the_model() {
        let m = manager(Arc::new(AtomicUsize::new(0)));
        m.get(Path::new("/m.gguf")).unwrap();
        m.unload();
        assert!(!m.is_loaded());
    }

    #[tokio::test]
    async fn local_ask_waits_while_whisper_is_loaded() {
        let whisper = Arc::new(FakeWhisper::default());
        whisper.in_use.store(true, Ordering::SeqCst);
        let chat = LocalChat::new(
            manager(Arc::new(AtomicUsize::new(0))),
            "/m.gguf".into(),
            whisper,
            Purpose::Ask,
        );
        let err = chat
            .chat(&user("q"), &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid value: Ask is available when the transcription ends."
        );
    }

    #[tokio::test]
    async fn local_notes_free_whisper_before_the_load() {
        let whisper = Arc::new(FakeWhisper::default());
        let chat = LocalChat::new(
            manager(Arc::new(AtomicUsize::new(0))),
            "/m.gguf".into(),
            whisper.clone(),
            Purpose::Notes,
        );
        let r = chat
            .chat(&user("q"), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(r.text, "answer");
        assert_eq!(whisper.freed.load(Ordering::SeqCst), 1);
        assert_eq!(chat.budget(), Budget::LOCAL);
        assert_eq!(chat.name(), crate::local_llm::LOCAL_MODEL_NAME);
    }

    /// Counts how many calls run at the same time.
    struct Slow {
        now: AtomicUsize,
        most: AtomicUsize,
    }

    impl Generate for Slow {
        fn complete(&self, _m: &[ChatMessage], _max: u32, _c: &AtomicBool) -> Result<ChatResult> {
            let n = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(n, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(50));
            self.now.fetch_sub(1, Ordering::SeqCst);
            Ok(ChatResult {
                text: "ok".into(),
                prompt_tokens: 1,
                completion_tokens: 1,
                cost: None,
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_local_calls_never_run_at_the_same_time() {
        let slow = Arc::new(Slow {
            now: AtomicUsize::new(0),
            most: AtomicUsize::new(0),
        });
        let model = slow.clone();
        let m = Arc::new(ModelManager::new(
            Box::new(move |_p: &Path| Ok(model.clone())),
            Duration::from_secs(300),
        ));
        let chat = |purpose| {
            LocalChat::new(
                m.clone(),
                "/m.gguf".into(),
                Arc::new(FakeWhisper::default()),
                purpose,
            )
        };
        let (notes, ask) = (chat(Purpose::Notes), chat(Purpose::Ask));
        let token = CancellationToken::new();
        let (qa, qb) = (user("a"), user("b"));
        let (a, b) = tokio::join!(notes.chat(&qa, &token), ask.chat(&qb, &token));
        a.unwrap();
        b.unwrap();
        assert_eq!(slow.most.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_cancelled_token_stops_the_local_call() {
        let chat = LocalChat::new(
            manager(Arc::new(AtomicUsize::new(0))),
            "/m.gguf".into(),
            Arc::new(FakeWhisper::default()),
            Purpose::Notes,
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = chat.chat(&user("q"), &cancel).await.unwrap_err();
        assert!(matches!(err, crate::types::MorselError::Cancelled));
    }
}

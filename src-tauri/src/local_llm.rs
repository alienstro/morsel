//! The on-device notes model: MiniCPM-V-4 (Q6_K) through llama.cpp, with Metal.

use crate::llm::{ChatMessage, ChatResult};
use crate::transcribe::model_download::{ModelFile, file_ready};
use crate::types::{MorselError, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

const NOTES_REPO: &str = "https://huggingface.co/openbmb/MiniCPM-V-4-gguf/resolve/main";
const NOTES_DIR: &str = "minicpm-v-4-q6k";
pub const NOTES_FILE: &str = "ggml-model-Q6_K.gguf";
/// The name that the usage table and the saved note show for the local model.
pub const LOCAL_MODEL_NAME: &str = "MiniCPM-V-4 (local)";
/// The context window. It keeps the memory use safe on an 8 GB Mac.
pub const N_CTX: u32 = 8192;
/// The prompt goes to llama.cpp in batches of this size.
const N_BATCH: usize = 512;

/// The one file of the notes model, pinned to its size and SHA-256.
pub fn notes_model_files() -> Vec<ModelFile> {
    vec![ModelFile {
        name: NOTES_FILE.into(),
        url: format!("{NOTES_REPO}/{NOTES_FILE}"),
        sha256: "fc78a557d5bcc3cb21f99667f97fe3e0381fc129d586d1532fffe1926ec53bd1".into(),
        size: 2_960_134_016,
    }]
}

/// The folder of the notes model inside `models_root`.
pub fn notes_model_dir(root: &Path) -> PathBuf {
    root.join(NOTES_DIR)
}

/// True when the notes model file is in `dir` with the right size.
pub fn notes_model_ready(dir: &Path) -> bool {
    notes_model_files().iter().all(|f| file_ready(dir, f))
}

/// Keeps the end of a prompt that is too long, so that the answer still fits in the context.
/// The end holds the instruction and the newest text, so it is the part to keep.
pub fn fit_prompt<T>(mut tokens: Vec<T>, n_ctx: u32, max_tokens: u32) -> Vec<T> {
    let room = n_ctx.saturating_sub(max_tokens) as usize;
    if tokens.len() > room {
        tokens.drain(..tokens.len() - room);
    }
    tokens
}

/// llama.cpp allows one backend for each process.
fn backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
    if let Some(b) = BACKEND.get() {
        return Ok(b);
    }
    llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default().with_logs_enabled(false));
    let b = LlamaBackend::init().map_err(|e| llm_error("start llama.cpp", e))?;
    Ok(BACKEND.get_or_init(|| b))
}

fn llm_error(what: &str, err: impl std::fmt::Display) -> MorselError {
    MorselError::Invalid(format!("the local notes model cannot {what}: {err}"))
}

pub struct LocalLlm {
    model: LlamaModel,
}

impl LocalLlm {
    /// Loads the model with all layers on the GPU (Metal).
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(MorselError::Invalid(
                "download the notes model in Settings first".into(),
            ));
        }
        let params = LlamaModelParams::default().with_n_gpu_layers(1000);
        let model = LlamaModel::load_from_file(backend()?, path, &params)
            .map_err(|e| llm_error("load", e))?;
        Ok(Self { model })
    }

    fn prompt(&self, messages: &[ChatMessage]) -> Result<String> {
        let template = self
            .model
            .chat_template(None)
            .map_err(|e| llm_error("read its chat template", e))?;
        let chat = messages
            .iter()
            .map(|m| LlamaChatMessage::new(m.role.clone(), m.content.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| llm_error("read the messages", e))?;
        self.model
            .apply_chat_template(&template, &chat, true)
            .map_err(|e| llm_error("build the prompt", e))
    }

    /// Writes an answer. This call blocks: run it on a blocking thread.
    /// It stops at the end-of-text token, at `max_tokens`, or when `cancel` becomes true.
    pub fn complete(
        &self,
        messages: &[ChatMessage],
        max_tokens: u32,
        cancel: &AtomicBool,
    ) -> Result<ChatResult> {
        let backend = backend()?;
        let prompt = self.prompt(messages)?;
        let vocab = self.model.vocab();
        let tokens = fit_prompt(
            vocab.tokenize(prompt.as_bytes(), false, true),
            N_CTX,
            max_tokens,
        );
        if tokens.is_empty() {
            return Err(MorselError::Invalid("the prompt is empty".into()));
        }
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(N_CTX))
            .with_n_batch(N_BATCH as u32);
        let mut ctx = self
            .model
            .new_context(backend, params)
            .map_err(|e| llm_error("make a context", e))?;

        let mut batch = LlamaBatch::new(N_BATCH, 1);
        let last = tokens.len() - 1;
        for (start, chunk) in tokens.chunks(N_BATCH).enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(MorselError::Cancelled);
            }
            batch.clear();
            for (i, token) in chunk.iter().enumerate() {
                let pos = start * N_BATCH + i;
                batch
                    .add(*token, pos as i32, &[0], pos == last)
                    .map_err(|e| llm_error("read the prompt", e))?;
            }
            ctx.decode(&mut batch)
                .map_err(|e| llm_error("read the prompt", e))?;
        }

        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::temp(0.3),
            LlamaSampler::dist(1234),
        ]);
        let mut bytes = Vec::new();
        let mut pos = tokens.len() as i32;
        let mut produced: u32 = 0;
        while produced < max_tokens {
            if cancel.load(Ordering::Relaxed) {
                return Err(MorselError::Cancelled);
            }
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);
            if vocab.is_eog(token) {
                break;
            }
            produced += 1;
            if add_piece(&mut bytes, &vocab.token_to_piece(token, false, None)) {
                break;
            }
            batch.clear();
            batch
                .add(token, pos, &[0], true)
                .map_err(|e| llm_error("write", e))?;
            pos += 1;
            ctx.decode(&mut batch).map_err(|e| llm_error("write", e))?;
        }
        Ok(ChatResult {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            prompt_tokens: tokens.len() as u32,
            completion_tokens: produced,
            cost: None,
        })
    }
}

/// Finds a loop at the end of the output: one unit, 3 times in a row.
/// Returns the length to keep, which ends after the first copy of the unit.
pub fn loop_cut(text: &[u8]) -> Option<usize> {
    const MIN_UNIT: usize = 8;
    const MAX_UNIT: usize = 1_000;
    let len = text.len();
    (MIN_UNIT..=MAX_UNIT.min(len / 3)).find_map(|unit| {
        let third = &text[len - unit..];
        let second = &text[len - 2 * unit..len - unit];
        let first = &text[len - 3 * unit..len - 2 * unit];
        let is_loop =
            third == second && second == first && third.iter().any(u8::is_ascii_alphanumeric);
        is_loop.then_some(len - 2 * unit)
    })
}

/// Adds one token's text to the output. Returns true when the output loops:
/// then the output keeps only the text before the loop, and writing must stop.
fn add_piece(out: &mut Vec<u8>, piece: &[u8]) -> bool {
    out.extend_from_slice(piece);
    match loop_cut(out) {
        Some(keep) => {
            out.truncate(keep);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_piece_stops_at_the_third_copy_and_keeps_the_first() {
        let line = b"- [ ] Miguel: send the budget.\n";
        let mut out = b"## Action items\n".to_vec();
        assert!(!add_piece(&mut out, line));
        assert!(!add_piece(&mut out, line));
        assert!(add_piece(&mut out, line));
        assert_eq!(out, b"## Action items\n- [ ] Miguel: send the budget.\n");
    }

    #[test]
    fn loop_cut_keeps_one_copy_of_a_repeated_line() {
        let head = "## Action items\n- [ ] Ana: draft the FAQ.\n";
        let line = "- [ ] Miguel: send the budget by March 22.\n";
        let text = format!("{head}{line}{line}{line}");
        assert_eq!(loop_cut(text.as_bytes()), Some(head.len() + line.len()));
    }

    #[test]
    fn loop_cut_finds_a_repeated_block_of_lines() {
        let block = "- Owner: Miguel\n- Task: Prioritize Maya refund bug\n- Task: Reserve 50,000\n";
        let text = format!("## Action items\n{block}{block}{block}");
        assert_eq!(
            loop_cut(text.as_bytes()),
            Some("## Action items\n".len() + block.len())
        );
    }

    #[test]
    fn loop_cut_finds_a_phrase_repeated_in_one_line() {
        let text = "You: Salamat. Okay. You: Salamat. Okay. You: Salamat. Okay. ";
        assert_eq!(loop_cut(text.as_bytes()), Some(20));
    }

    #[test]
    fn loop_cut_allows_two_copies() {
        let line = "- [ ] Miguel: send the budget by March 22.\n";
        assert_eq!(loop_cut(format!("{line}{line}").as_bytes()), None);
    }

    #[test]
    fn loop_cut_ignores_rules_and_table_lines() {
        let text = "| Item | Owner |\n|--------|--------|--------|--------|\n\n--------------------------------";
        assert_eq!(loop_cut(text.as_bytes()), None);
    }

    #[test]
    fn loop_cut_ignores_a_normal_note() {
        let text = "## Summary\nThe launch moves to March 29.\n\n## Decisions\n- None.\n\n## Action items\n- None.\n";
        assert_eq!(loop_cut(text.as_bytes()), None);
    }

    #[test]
    fn notes_model_files_pin_the_q6k_file() {
        let files = notes_model_files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "ggml-model-Q6_K.gguf");
        assert_eq!(files[0].size, 2_960_134_016);
        assert_eq!(
            files[0].sha256,
            "fc78a557d5bcc3cb21f99667f97fe3e0381fc129d586d1532fffe1926ec53bd1"
        );
        assert!(
            files[0]
                .url
                .starts_with("https://huggingface.co/openbmb/MiniCPM-V-4-gguf/")
        );
    }

    #[test]
    fn fit_prompt_keeps_the_end_of_a_long_prompt() {
        let tokens: Vec<i32> = (0..10_000).collect();
        let fitted = fit_prompt(tokens, 8192, 1024);
        assert_eq!(fitted.len(), 7168);
        assert_eq!(*fitted.last().unwrap(), 9_999);
    }

    /// Needs the model (about 3 GB): `cargo test --lib -- --include-ignored local_llm`.
    #[tokio::test]
    #[ignore]
    async fn local_llm_answers_with_the_real_model() {
        use crate::transcribe::model_download::{ensure_files, models_root};
        let home = std::env::var("HOME").expect("HOME");
        let dir = notes_model_dir(&models_root(
            &Path::new(&home).join("Library/Application Support/app.morsel"),
        ));
        ensure_files(
            &reqwest::Client::new(),
            &dir,
            &notes_model_files(),
            |_, _| {},
        )
        .await
        .unwrap();
        let llm = LocalLlm::load(&dir.join(NOTES_FILE)).unwrap();
        let messages = [ChatMessage {
            role: "user".into(),
            content: "Reply with only the word: pong".into(),
        }];
        let r = llm
            .complete(&messages, 16, &AtomicBool::new(false))
            .unwrap();
        assert!(r.text.to_lowercase().contains("pong"), "{r:?}");
        assert!(r.completion_tokens > 0);
    }

    #[test]
    fn load_without_the_model_asks_for_the_download() {
        // No model is needed: `load` refuses a missing file before any work.
        let err = LocalLlm::load(Path::new("/no/such/model.gguf"))
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "invalid value: download the notes model in Settings first"
        );
    }

    #[test]
    fn fit_prompt_leaves_a_short_prompt_alone() {
        let tokens: Vec<i32> = (0..100).collect();
        assert_eq!(fit_prompt(tokens.clone(), 8192, 1024), tokens);
    }
}

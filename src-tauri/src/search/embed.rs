//! The on-device search model: EmbeddingGemma 2 (Q8_0) through llama.cpp, with Metal.

use crate::local_llm::backend;
use crate::model_manager::ModelManager;
use crate::search::index::Embed;
use crate::transcribe::model_download::ModelFile;
use crate::types::{MorselError, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SEARCH_REPO: &str = "https://huggingface.co/unsloth/embeddinggemma-2-GGUF/resolve/main";
const SEARCH_DIR: &str = "embeddinggemma-2-q8";
pub const SEARCH_FILE: &str = "embeddinggemma-2-Q8_0.gguf";
/// The length of each embedding.
pub const DIMENSIONS: usize = 768;
/// The longest text, in tokens. A part is about 400 tokens. The model is not causal,
/// so the whole text must go to llama.cpp in one batch.
const N_CTX: u32 = 2048;

/// The one file of the search model, pinned to its size and SHA-256.
pub fn search_model_files() -> Vec<ModelFile> {
    vec![ModelFile {
        name: SEARCH_FILE.into(),
        url: format!("{SEARCH_REPO}/{SEARCH_FILE}"),
        sha256: "6f1bd4ac6c5df7444f9cca7ca36cafe6cfa34cd6f49fefb1e0b4be8143aed8bc".into(),
        size: 309_855_520,
    }]
}

/// The folder of the search model inside `models_root`.
pub fn search_model_dir(root: &Path) -> PathBuf {
    root.join(SEARCH_DIR)
}

/// The text to embed for a question.
pub fn query_text(question: &str) -> String {
    format!("task: search result | query: {question}")
}

/// The text to embed for one part of a meeting.
pub fn part_text(title: &str, part: &str) -> String {
    format!("title: {title} | text: {part}")
}

/// The cosine similarity of two normalized vectors: their dot product.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

fn embed_error(what: &str, err: impl std::fmt::Display) -> MorselError {
    MorselError::Invalid(format!("the local search model cannot {what}: {err}"))
}

pub struct LocalEmbedder {
    model: LlamaModel,
}

impl LocalEmbedder {
    /// Loads the model with all layers on the GPU (Metal).
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(MorselError::Invalid(
                "download the search model in Settings first".into(),
            ));
        }
        let params = LlamaModelParams::default().with_n_gpu_layers(1000);
        let model = LlamaModel::load_from_file(backend()?, path, &params)
            .map_err(|e| embed_error("load", e))?;
        Ok(Self { model })
    }

    /// One normalized vector of `DIMENSIONS` values for each text.
    /// This call blocks: run it on a blocking thread.
    pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(N_CTX))
            .with_n_batch(N_CTX)
            .with_n_ubatch(N_CTX)
            .with_embeddings(true);
        let mut ctx = self
            .model
            .new_context(backend()?, params)
            .map_err(|e| embed_error("start", e))?;
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            let mut tokens = self.model.vocab().tokenize(text.as_bytes(), true, false);
            tokens.truncate(N_CTX as usize);
            let mut batch = LlamaBatch::new(N_CTX as usize, 1);
            batch
                .add_sequence(&tokens, 0, false)
                .map_err(|e| embed_error("read the text", e))?;
            ctx.clear_kv_cache();
            ctx.decode(&mut batch)
                .map_err(|e| embed_error("read the text", e))?;
            let vector = ctx
                .embeddings_seq_ith(0)
                .map_err(|e| embed_error("embed", e))?;
            if vector.len() != DIMENSIONS {
                return Err(embed_error(
                    "embed",
                    format!("{} values, not {DIMENSIONS}", vector.len()),
                ));
            }
            out.push(normalize(vector));
        }
        Ok(out)
    }
}

/// The search model behind the model manager, which loads it on first use and frees
/// it after `EMBED_IDLE`.
pub struct ManagedEmbed {
    manager: Arc<ModelManager<LocalEmbedder>>,
    path: PathBuf,
}

impl ManagedEmbed {
    pub fn new(manager: Arc<ModelManager<LocalEmbedder>>, path: PathBuf) -> Self {
        Self { manager, path }
    }
}

impl Embed for ManagedEmbed {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.manager.run(&self.path, |model| model.embed(texts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_model_files_pin_the_q8_file() {
        let files = search_model_files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "embeddinggemma-2-Q8_0.gguf");
        assert_eq!(files[0].size, 309_855_520);
        assert_eq!(
            files[0].sha256,
            "6f1bd4ac6c5df7444f9cca7ca36cafe6cfa34cd6f49fefb1e0b4be8143aed8bc"
        );
        assert_eq!(
            files[0].url,
            format!("{SEARCH_REPO}/embeddinggemma-2-Q8_0.gguf")
        );
    }

    #[test]
    fn labels_match_the_spec() {
        assert_eq!(
            query_text("kailan ang launch?"),
            "task: search result | query: kailan ang launch?"
        );
        assert_eq!(
            part_text("Q1 sync", "The launch moves."),
            "title: Q1 sync | text: The launch moves."
        );
    }

    #[test]
    fn cosine_of_a_vector_with_itself_is_one() {
        let v = [0.6_f32, 0.8];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
        assert!(cosine(&v, &[0.8, -0.6]).abs() < 1e-6);
    }

    #[tokio::test]
    #[ignore = "needs the real search model (310 MB download)"]
    async fn the_same_question_is_closer_to_its_answer_than_to_another_topic() {
        use crate::transcribe::model_download::{ensure_files, models_root};
        let home = std::env::var("HOME").expect("HOME");
        let dir = search_model_dir(&models_root(
            &Path::new(&home).join("Library/Application Support/app.morsel"),
        ));
        ensure_files(
            &reqwest::Client::new(),
            &dir,
            &search_model_files(),
            |_, _| {},
        )
        .await
        .expect("download");
        let embedder = LocalEmbedder::load(&dir.join(SEARCH_FILE)).expect("load");
        let v = embedder
            .embed(&[
                query_text("kailan ang launch?"),
                part_text("Q1 sync", "The launch moves to March 29."),
                part_text("Q1 sync", "Ana writes the FAQ page."),
            ])
            .expect("embed");
        assert_eq!(v.len(), 3);
        assert!(v.iter().all(|e| e.len() == DIMENSIONS));
        let norm: f32 = v[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "{norm}");
        assert!(cosine(&v[0], &v[1]) > cosine(&v[0], &v[2]));
    }
}

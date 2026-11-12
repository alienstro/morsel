//! One interface for the notes model, so that the notes writer and Ask do not know if it is local or cloud.

use crate::ask::ASK_TOKENS;
use crate::llm::{ChatMessage, ChatResult, LlmConfig, chat};
use crate::notes::prompt::{LONG_TRANSCRIPT_TOKENS, PART_TOKENS};
use crate::types::{MorselError, Result};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

/// How much text one model call can take, in estimated tokens (characters ÷ 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// A longer transcript gets the two-step notes method.
    pub long_transcript: usize,
    /// The size of one transcript part in the two-step method.
    pub part: usize,
    /// The meeting text that one Ask answer reads.
    pub ask: usize,
}

impl Budget {
    pub const CLOUD: Budget = Budget {
        long_transcript: LONG_TRANSCRIPT_TOKENS,
        part: PART_TOKENS,
        ask: ASK_TOKENS,
    };
    /// Fits the 8,192-token context of the local model, with room for the answer.
    pub const LOCAL: Budget = Budget {
        long_transcript: 6_000,
        part: 3_000,
        ask: 6_000,
    };
}

#[async_trait]
pub trait ChatModel: Send + Sync {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        cancel: &CancellationToken,
    ) -> Result<ChatResult>;
    /// The model name for the usage table and the saved note.
    fn name(&self) -> &str;
    fn budget(&self) -> Budget;
}

/// The cloud model: an OpenAI-compatible HTTP API.
pub struct CloudChat {
    http: reqwest::Client,
    cfg: LlmConfig,
}

impl CloudChat {
    pub fn new(http: reqwest::Client, cfg: LlmConfig) -> Self {
        Self { http, cfg }
    }
}

#[async_trait]
impl ChatModel for CloudChat {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        cancel: &CancellationToken,
    ) -> Result<ChatResult> {
        tokio::select! {
            r = chat(&self.http, &self.cfg, messages) => r,
            _ = cancel.cancelled() => Err(MorselError::Cancelled),
        }
    }

    fn name(&self) -> &str {
        &self.cfg.model
    }

    fn budget(&self) -> Budget {
        Budget::CLOUD
    }
}

/// A model for tests: it saves each request and answers with a fixed text.
#[cfg(test)]
pub struct FakeChat {
    pub budget: Budget,
    pub answer: String,
    pub calls: std::sync::Mutex<Vec<Vec<ChatMessage>>>,
}

#[cfg(test)]
impl FakeChat {
    pub fn new(budget: Budget, answer: &str) -> Self {
        Self {
            budget,
            answer: answer.into(),
            calls: Default::default(),
        }
    }

    pub fn calls(&self) -> Vec<Vec<ChatMessage>> {
        self.calls.lock().unwrap().clone()
    }
}

#[cfg(test)]
#[async_trait]
impl ChatModel for FakeChat {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        _cancel: &CancellationToken,
    ) -> Result<ChatResult> {
        self.calls.lock().unwrap().push(messages.to_vec());
        Ok(ChatResult {
            text: self.answer.clone(),
            prompt_tokens: 1,
            completion_tokens: 1,
            cost: None,
        })
    }

    fn name(&self) -> &str {
        "fake"
    }

    fn budget(&self) -> Budget {
        self.budget
    }
}

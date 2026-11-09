//! One client for any OpenAI-compatible chat API: OpenRouter, OpenAI, Groq, or a local server.

use crate::types::{MorselError, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const CHAT_TIMEOUT: Duration = Duration::from_secs(120);
const TEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatResult {
    pub text: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost: Option<f64>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChatMessage,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    cost: Option<f64>,
}

/// Joins the base URL and a path, with one slash between them.
pub fn endpoint(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Maps an HTTP error status to a Morsel error.
pub fn status_error(status: reqwest::StatusCode, body: &str) -> MorselError {
    match status.as_u16() {
        401 | 403 => MorselError::InvalidKey,
        429 => MorselError::RateLimit,
        code => MorselError::Http(format!(
            "HTTP {code}: {}",
            body.chars().take(300).collect::<String>()
        )),
    }
}

fn send_error(err: reqwest::Error) -> MorselError {
    MorselError::Http(err.without_url().to_string())
}

pub async fn chat(
    http: &reqwest::Client,
    cfg: &LlmConfig,
    messages: &[ChatMessage],
) -> Result<ChatResult> {
    let response = http
        .post(endpoint(&cfg.base_url, "chat/completions"))
        .bearer_auth(&cfg.api_key)
        .timeout(CHAT_TIMEOUT)
        .json(&serde_json::json!({ "model": cfg.model, "messages": messages }))
        .send()
        .await
        .map_err(send_error)?;
    let status = response.status();
    let body = response.text().await.map_err(send_error)?;
    if !status.is_success() {
        return Err(status_error(status, &body));
    }
    let parsed: ChatResponse =
        serde_json::from_str(&body).map_err(|e| MorselError::Http(format!("bad response: {e}")))?;
    let text = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| MorselError::Http("the response has no choices".into()))?;
    let usage = parsed.usage.unwrap_or_default();
    Ok(ChatResult {
        text,
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        cost: usage.cost,
    })
}

/// Checks the base URL and the key with `GET /models`.
pub async fn test_key(http: &reqwest::Client, cfg: &LlmConfig) -> Result<()> {
    let response = http
        .get(endpoint(&cfg.base_url, "models"))
        .bearer_auth(&cfg.api_key)
        .timeout(TEST_TIMEOUT)
        .send()
        .await
        .map_err(send_error)?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(status_error(status, &body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn cfg(server: &MockServer, suffix: &str) -> LlmConfig {
        LlmConfig {
            base_url: format!("{}/v1{suffix}", server.uri()),
            model: "test/model".into(),
            api_key: "sk-test".into(),
        }
    }

    fn user(text: &str) -> Vec<ChatMessage> {
        vec![ChatMessage::user(text)]
    }

    fn ok_body(cost: Option<f64>) -> serde_json::Value {
        let mut usage = json!({ "prompt_tokens": 12, "completion_tokens": 3 });
        if let Some(c) = cost {
            usage["cost"] = json!(c);
        }
        json!({ "choices": [{ "message": { "role": "assistant", "content": "hello" } }], "usage": usage })
    }

    #[tokio::test]
    async fn chat_sends_bearer_key_and_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(
                json!({ "model": "test/model", "messages": [{ "role": "user", "content": "hi" }] }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body(None)))
            .expect(1)
            .mount(&server)
            .await;
        let result = chat(&reqwest::Client::new(), &cfg(&server, ""), &user("hi"))
            .await
            .unwrap();
        assert_eq!(result.text, "hello");
    }

    #[tokio::test]
    async fn chat_reads_usage_and_optional_cost() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body(Some(0.0021))))
            .mount(&server)
            .await;
        let r = chat(&reqwest::Client::new(), &cfg(&server, ""), &user("hi"))
            .await
            .unwrap();
        assert_eq!(
            (r.prompt_tokens, r.completion_tokens, r.cost),
            (12, 3, Some(0.0021))
        );
    }

    #[tokio::test]
    async fn http_401_maps_to_invalid_key_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let err = chat(&reqwest::Client::new(), &cfg(&server, ""), &user("hi"))
            .await
            .unwrap_err();
        assert!(matches!(err, MorselError::InvalidKey), "{err:?}");
    }

    #[tokio::test]
    async fn http_429_maps_to_rate_limit_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        let err = chat(&reqwest::Client::new(), &cfg(&server, ""), &user("hi"))
            .await
            .unwrap_err();
        assert!(matches!(err, MorselError::RateLimit), "{err:?}");
    }

    #[tokio::test]
    async fn trailing_slash_in_base_url_is_ignored() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_body(None)))
            .expect(1)
            .mount(&server)
            .await;
        chat(&reqwest::Client::new(), &cfg(&server, "/"), &user("hi"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_key_calls_models_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
            .expect(1)
            .mount(&server)
            .await;
        test_key(&reqwest::Client::new(), &cfg(&server, ""))
            .await
            .unwrap();
    }
}

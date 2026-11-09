//! Any OpenAI-compatible `POST /audio/transcriptions` endpoint, such as Groq.

use super::{Transcriber, language_param, wav_bytes};
use crate::llm::{endpoint, status_error};
use crate::types::{MorselError, Result};
use serde::Deserialize;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(60);

pub struct CloudTranscriber {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: String,
}

impl CloudTranscriber {
    pub fn new(http: reqwest::Client, base_url: String, model: String, api_key: String) -> Self {
        Self {
            http,
            base_url,
            model,
            api_key,
        }
    }
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

#[async_trait::async_trait]
impl Transcriber for CloudTranscriber {
    async fn transcribe(&self, samples: &[f32], language: Option<&str>) -> Result<String> {
        let file = reqwest::multipart::Part::bytes(wav_bytes(samples))
            .file_name("chunk.wav")
            .mime_str("audio/wav")
            .map_err(|e| MorselError::Transcribe(e.to_string()))?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", file)
            .text("model", self.model.clone())
            .text("response_format", "json");
        if let Some(lang) = language_param(language) {
            form = form.text("language", lang.to_owned());
        }
        let response = self
            .http
            .post(endpoint(&self.base_url, "audio/transcriptions"))
            .bearer_auth(&self.api_key)
            .timeout(TIMEOUT)
            .multipart(form)
            .send()
            .await
            .map_err(|e| MorselError::Http(e.without_url().to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| MorselError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(status_error(status, &body));
        }
        let parsed: TranscriptionResponse = serde_json::from_str(&body)
            .map_err(|e| MorselError::Transcribe(format!("bad response: {e}")))?;
        Ok(parsed.text.trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::Transcriber;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn server_ok() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/audio/transcriptions"))
            .and(header("authorization", "Bearer gsk-test"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "text": " hello there " })),
            )
            .mount(&server)
            .await;
        server
    }

    fn client(server: &MockServer) -> CloudTranscriber {
        CloudTranscriber::new(
            reqwest::Client::new(),
            format!("{}/openai/v1", server.uri()),
            "whisper-large-v3-turbo".into(),
            "gsk-test".into(),
        )
    }

    async fn sent_body(server: &MockServer) -> String {
        let requests = server.received_requests().await.unwrap();
        String::from_utf8_lossy(&requests[0].body).into_owned()
    }

    #[tokio::test]
    async fn sends_multipart_with_file_and_model() {
        let server = server_ok().await;
        let text = client(&server)
            .transcribe(&[0.1; 1600], Some("en"))
            .await
            .unwrap();
        assert_eq!(text, "hello there");
        let body = sent_body(&server).await;
        assert!(body.contains("name=\"file\"; filename=\"chunk.wav\""));
        assert!(body.contains("name=\"model\"\r\n\r\nwhisper-large-v3-turbo"));
        assert!(body.contains("name=\"language\"\r\n\r\nen"));
    }

    #[tokio::test]
    async fn language_auto_is_not_sent() {
        let server = server_ok().await;
        client(&server)
            .transcribe(&[0.1; 1600], Some("auto"))
            .await
            .unwrap();
        assert!(!sent_body(&server).await.contains("name=\"language\""));
    }

    #[tokio::test]
    async fn http_401_maps_to_invalid_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let err = client(&server)
            .transcribe(&[0.1; 1600], None)
            .await
            .unwrap_err();
        assert!(matches!(err, MorselError::InvalidKey));
    }
}

//! A small client for the Composio REST API (v3). It uses the user's own Composio key.

use super::events::{parse_google_events, parse_outlook_events};
use super::{CalendarEvent, Provider};
use crate::llm::status_error;
use crate::types::{MorselError, Result};
use serde::Deserialize;
use serde_json::{Value, json};

pub const BASE_URL: &str = "https://backend.composio.dev/api/v3";
const MAX_EVENTS: u32 = 50;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Link {
    pub redirect_url: String,
    pub connected_account_id: String,
}

pub struct Composio<'a> {
    http: &'a reqwest::Client,
    base: String,
    key: String,
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl<'a> Composio<'a> {
    pub fn new(http: &'a reqwest::Client, base: &str, key: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            key: key.to_owned(),
        }
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value> {
        let resp = req
            .header("x-api-key", &self.key)
            .send()
            .await
            .map_err(|e| MorselError::Http(e.without_url().to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| MorselError::Http(e.without_url().to_string()))?;
        if !status.is_success() {
            return Err(match status_error(status, &body) {
                MorselError::InvalidKey => MorselError::InvalidComposioKey,
                other => other,
            });
        }
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&body).map_err(|e| MorselError::Http(format!("Composio: {e}")))
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.send(self.http.post(format!("{}{path}", self.base)).json(&body))
            .await
    }

    /// Creates an auth config that uses the Composio-managed OAuth app of the toolkit.
    pub async fn create_auth_config(&self, provider: Provider) -> Result<String> {
        let v = self
            .post(
                "/auth_configs",
                json!({
                    "toolkit": { "slug": provider.toolkit() },
                    "auth_config": { "type": "use_composio_managed_auth", "credentials": {}, "restrict_to_following_tools": [] }
                }),
            )
            .await?;
        v.pointer("/auth_config/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| MorselError::Http("Composio: no auth config id".into()))
    }

    /// Starts a connection. The user opens `redirect_url` to sign in.
    pub async fn link(&self, auth_config_id: &str, user_id: &str, callback: &str) -> Result<Link> {
        let v = self
            .post(
                "/connected_accounts/link",
                json!({ "auth_config_id": auth_config_id, "user_id": user_id, "callback_url": callback }),
            )
            .await?;
        serde_json::from_value(v).map_err(|e| MorselError::Http(format!("Composio: {e}")))
    }

    pub async fn account_active(&self, account_id: &str) -> Result<bool> {
        let v = self
            .send(
                self.http
                    .get(format!("{}/connected_accounts/{account_id}", self.base)),
            )
            .await?;
        Ok(v.get("status").and_then(Value::as_str) == Some("ACTIVE"))
    }

    pub async fn delete_account(&self, account_id: &str) -> Result<()> {
        self.send(
            self.http
                .delete(format!("{}/connected_accounts/{account_id}", self.base)),
        )
        .await
        .map(|_| ())
    }

    async fn execute(
        &self,
        tool: &str,
        account_id: &str,
        user_id: &str,
        args: Value,
    ) -> Result<Value> {
        let v = self
            .post(
                &format!("/tools/execute/{tool}"),
                json!({ "connected_account_id": account_id, "user_id": user_id, "arguments": args }),
            )
            .await?;
        if v.get("successful").and_then(Value::as_bool) == Some(false) {
            let reason = v
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the calendar tool failed");
            return Err(MorselError::Http(format!("Composio: {reason}")));
        }
        Ok(v.get("data").cloned().unwrap_or(Value::Null))
    }

    /// The events between two times (Unix milliseconds).
    pub async fn list_events(
        &self,
        provider: Provider,
        account_id: &str,
        user_id: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<CalendarEvent>> {
        let args = match provider {
            Provider::Google => json!({
                "calendarId": "primary", "timeMin": rfc3339(from_ms), "timeMax": rfc3339(to_ms),
                "singleEvents": true, "orderBy": "startTime", "maxResults": MAX_EVENTS
            }),
            Provider::Outlook => json!({
                "start_datetime": rfc3339(from_ms), "end_datetime": rfc3339(to_ms), "top": MAX_EVENTS
            }),
        };
        let data = self
            .execute(provider.events_tool(), account_id, user_id, args)
            .await?;
        Ok(match provider {
            Provider::Google => parse_google_events(&data),
            Provider::Outlook => parse_outlook_events(&data),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn create_auth_config_sends_managed_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth_configs"))
            .and(header("x-api-key", "ck-test"))
            .and(body_partial_json(json!({
                "toolkit": { "slug": "googlecalendar" },
                "auth_config": { "type": "use_composio_managed_auth" }
            })))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({ "auth_config": { "id": "ac_1" } })),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let c = Composio::new(&http, &server.uri(), "ck-test");
        assert_eq!(
            c.create_auth_config(Provider::Google).await.unwrap(),
            "ac_1"
        );
    }

    #[tokio::test]
    async fn link_returns_the_redirect_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/connected_accounts/link"))
            .and(body_partial_json(json!({
                "auth_config_id": "ac_1", "user_id": "morsel-u", "callback_url": "morsel://calendar/connected"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "link_token": "lt", "redirect_url": "https://connect.composio.dev/link/lt",
                "expires_at": "2026-11-09T12:00:00Z", "connected_account_id": "ca_1"
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let c = Composio::new(&http, &server.uri(), "ck");
        let link = c
            .link("ac_1", "morsel-u", "morsel://calendar/connected")
            .await
            .unwrap();
        assert_eq!(link.redirect_url, "https://connect.composio.dev/link/lt");
        assert_eq!(link.connected_account_id, "ca_1");
    }

    #[tokio::test]
    async fn account_status_and_delete() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/connected_accounts/ca_1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "id": "ca_1", "status": "ACTIVE" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/connected_accounts/ca_2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "id": "ca_2", "status": "INITIATED" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/connected_accounts/ca_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true })))
            .expect(1)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let c = Composio::new(&http, &server.uri(), "ck");
        assert!(c.account_active("ca_1").await.unwrap());
        assert!(!c.account_active("ca_2").await.unwrap());
        c.delete_account("ca_1").await.unwrap();
    }

    #[tokio::test]
    async fn list_events_sends_the_time_range_and_parses_google() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tools/execute/GOOGLECALENDAR_EVENTS_LIST"))
            .and(body_partial_json(json!({
                "connected_account_id": "ca_1", "user_id": "u",
                "arguments": { "calendarId": "primary", "timeMin": "2026-11-09T00:00:00Z", "singleEvents": true }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "successful": true, "error": null,
                "data": { "items": [ { "id": "g", "summary": "Sync", "start": { "dateTime": "2026-11-09T10:00:00Z" } } ] }
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let c = Composio::new(&http, &server.uri(), "ck");
        let from = chrono::DateTime::parse_from_rfc3339("2026-11-09T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        let events = c
            .list_events(Provider::Google, "ca_1", "u", from, from + 86_400_000)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "Sync");
    }

    #[tokio::test]
    async fn execute_error_maps_to_calendar_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tools/execute/OUTLOOK_GET_CALENDAR_VIEW"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "successful": false, "error": "token expired", "data": {}
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/auth_configs"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let c = Composio::new(&http, &server.uri(), "bad");
        let err = c
            .list_events(Provider::Outlook, "ca", "u", 0, 1)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("token expired"), "{err}");
        assert!(matches!(
            c.create_auth_config(Provider::Outlook).await,
            Err(MorselError::InvalidComposioKey)
        ));
    }

    #[tokio::test]
    async fn refused_key_names_the_key_that_composio_needs() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth_configs"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let err = Composio::new(&http, &server.uri(), "bad")
            .create_auth_config(Provider::Google)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "invalid_key");
        assert!(err.to_string().contains("project API key"), "{err}");
        assert!(err.to_string().contains("Project Settings"), "{err}");
    }
}

//! The calendar link: the connections, the event cache and the commands.

use super::composio::Composio;
use super::{CalendarEvent, Provider};
use crate::events::EventSink;
use crate::secrets::{COMPOSIO_KEY, SecretStore};
use crate::types::{MorselError, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

const LINK_ROW: &str = "calendar";
pub const CALLBACK_URL: &str = "morsel://calendar/connected";
pub const CHANGED: &str = "calendar://changed";
/// The cache holds the events from one hour ago to 24 hours ahead.
const PAST_MS: i64 = 60 * 60_000;
const AHEAD_MS: i64 = 24 * 60 * 60_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLink {
    pub auth_config_id: String,
    pub account_id: String,
    pub active: bool,
}

/// Saved in the `settings` table under the key `calendar`. It holds no secret.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalendarLink {
    pub user_id: String,
    pub google: Option<ProviderLink>,
    pub outlook: Option<ProviderLink>,
}

impl CalendarLink {
    fn get(&self, p: Provider) -> Option<&ProviderLink> {
        match p {
            Provider::Google => self.google.as_ref(),
            Provider::Outlook => self.outlook.as_ref(),
        }
    }

    fn slot(&mut self, p: Provider) -> &mut Option<ProviderLink> {
        match p {
            Provider::Google => &mut self.google,
            Provider::Outlook => &mut self.outlook,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkState {
    None,
    Pending,
    Connected,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub state: LinkState,
    /// The number of meetings today, for a connected calendar.
    pub today: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    pub has_key: bool,
    pub google: ProviderStatus,
    pub outlook: ProviderStatus,
}

pub fn load_link(conn: &Connection) -> Result<CalendarLink> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [LINK_ROW],
            |r| r.get(0),
        )
        .optional()?;
    match raw {
        None => Ok(CalendarLink::default()),
        Some(json) => {
            serde_json::from_str(&json).map_err(|e| MorselError::Db(format!("calendar: {e}")))
        }
    }
}

pub fn save_link(conn: &Connection, link: &CalendarLink) -> Result<()> {
    let json = serde_json::to_string(link).map_err(|e| MorselError::Invalid(e.to_string()))?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [LINK_ROW, json.as_str()],
    )?;
    Ok(())
}

/// A stable, random-looking Composio user id for this Mac. It holds no personal data.
fn new_user_id(now_ms: i64) -> String {
    let mut h = Sha256::new();
    h.update(now_ms.to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    h.update(format!("{:p}", &h).as_bytes());
    format!("morsel-{}", &hex::encode(h.finalize())[..16])
}

/// The start and the end of the local day that holds `now_ms`.
fn local_day(now_ms: i64) -> (i64, i64) {
    use chrono::{Local, TimeZone};
    let now = Local
        .timestamp_millis_opt(now_ms)
        .single()
        .unwrap_or_else(Local::now);
    let start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|d| Local.from_local_datetime(&d).earliest())
        .map(|d| d.timestamp_millis())
        .unwrap_or(now_ms);
    (start, start + 24 * 60 * 60_000)
}

pub struct CalendarService {
    db: Arc<Mutex<Connection>>,
    secrets: Arc<dyn SecretStore>,
    http: reqwest::Client,
    base_url: String,
    sink: Arc<dyn EventSink>,
    cache: Mutex<Vec<CalendarEvent>>,
}

impl CalendarService {
    pub fn new(
        db: Arc<Mutex<Connection>>,
        secrets: Arc<dyn SecretStore>,
        http: reqwest::Client,
        base_url: &str,
        sink: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            db,
            secrets,
            http,
            base_url: base_url.to_owned(),
            sink,
            cache: Mutex::new(Vec::new()),
        }
    }

    fn link(&self) -> Result<CalendarLink> {
        let conn = self.db.lock().map_err(|_| MorselError::Db("lock".into()))?;
        load_link(&conn)
    }

    fn store(&self, link: &CalendarLink) -> Result<()> {
        let conn = self.db.lock().map_err(|_| MorselError::Db("lock".into()))?;
        save_link(&conn, link)
    }

    fn key(&self) -> Result<Option<String>> {
        Ok(self
            .secrets
            .get(COMPOSIO_KEY)?
            .filter(|k| !k.trim().is_empty()))
    }

    fn client(&self) -> Result<(Composio<'_>, CalendarLink)> {
        let key = self
            .key()?
            .ok_or_else(|| MorselError::Invalid("Add your Composio key first.".into()))?;
        Ok((
            Composio::new(&self.http, &self.base_url, &key),
            self.link()?,
        ))
    }

    fn changed(&self) {
        self.sink.emit(CHANGED, serde_json::Value::Null);
    }

    /// Starts the sign-in of a calendar and returns the page that the user must open.
    pub async fn connect(&self, provider: Provider, now_ms: i64) -> Result<String> {
        let (api, mut link) = self.client()?;
        if link.user_id.is_empty() {
            link.user_id = new_user_id(now_ms);
        }
        let auth_config_id = match link.get(provider) {
            Some(p) => p.auth_config_id.clone(),
            None => api.create_auth_config(provider).await?,
        };
        let started = api
            .link(&auth_config_id, &link.user_id, CALLBACK_URL)
            .await?;
        if !started.redirect_url.starts_with("https://") {
            return Err(MorselError::Http(
                "Composio sent a sign-in page that is not https.".into(),
            ));
        }
        *link.slot(provider) = Some(ProviderLink {
            auth_config_id,
            account_id: started.connected_account_id,
            active: false,
        });
        self.store(&link)?;
        Ok(started.redirect_url)
    }

    /// Removes the connection. Morsel forgets it also when Composio cannot delete it.
    pub async fn disconnect(&self, provider: Provider) -> Result<()> {
        let (api, mut link) = self.client()?;
        if let Some(p) = link.slot(provider).take()
            && let Err(err) = api.delete_account(&p.account_id).await
        {
            eprintln!("Composio could not delete the {provider:?} connection: {err}");
        }
        self.store(&link)?;
        self.cache
            .lock()
            .map_err(|_| MorselError::Db("lock".into()))?
            .retain(|e| e.provider != provider);
        self.changed();
        Ok(())
    }

    /// Marks a pending connection as connected when Composio says that it is active.
    /// Returns true when a connection changed.
    pub async fn check_pending(&self) -> Result<bool> {
        let Some(key) = self.key()? else {
            return Ok(false);
        };
        let api = Composio::new(&self.http, &self.base_url, &key);
        let mut link = self.link()?;
        let mut changed = false;
        for provider in Provider::ALL {
            if let Some(p) = link.slot(provider).as_mut()
                && !p.active
                && api.account_active(&p.account_id).await?
            {
                p.active = true;
                changed = true;
            }
        }
        if changed {
            self.store(&link)?;
        }
        Ok(changed)
    }

    /// Reads the events of each connected calendar into the cache. A failed calendar keeps
    /// its old events.
    pub async fn refresh(&self, now_ms: i64) -> Result<()> {
        let Some(key) = self.key()? else {
            return Ok(());
        };
        let api = Composio::new(&self.http, &self.base_url, &key);
        let link = self.link()?;
        let old = self
            .cache
            .lock()
            .map_err(|_| MorselError::Db("lock".into()))?
            .clone();
        let mut events = Vec::new();
        for provider in Provider::ALL {
            let Some(p) = link.get(provider).filter(|p| p.active) else {
                continue;
            };
            match api
                .list_events(
                    provider,
                    &p.account_id,
                    &link.user_id,
                    now_ms - PAST_MS,
                    now_ms + AHEAD_MS,
                )
                .await
            {
                Ok(found) => events.extend(found),
                Err(err) => {
                    eprintln!("cannot read the {provider:?} calendar: {err}");
                    events.extend(old.iter().filter(|e| e.provider == provider).cloned());
                }
            }
        }
        events.sort_by_key(|e| e.start);
        *self
            .cache
            .lock()
            .map_err(|_| MorselError::Db("lock".into()))? = events;
        self.changed();
        Ok(())
    }

    /// One poll: activates the finished sign-ins, then reads the events. Errors go to the log.
    pub async fn poll_once(&self, now_ms: i64) {
        if let Err(err) = self.check_pending().await {
            eprintln!("cannot check the calendar link: {err}");
        }
        if let Err(err) = self.refresh(now_ms).await {
            eprintln!("cannot read the calendars: {err}");
        }
    }

    /// The cached events of the calendars that are still connected.
    pub fn events(&self) -> Result<Vec<CalendarEvent>> {
        let link = self.link()?;
        if self.key()?.is_none() {
            return Ok(Vec::new());
        }
        let cache = self
            .cache
            .lock()
            .map_err(|_| MorselError::Db("lock".into()))?;
        Ok(cache
            .iter()
            .filter(|e| link.get(e.provider).is_some_and(|p| p.active))
            .cloned()
            .collect())
    }

    /// The meetings that have not ended, up to the end of today.
    pub fn upcoming(&self, now_ms: i64) -> Result<Vec<CalendarEvent>> {
        let (_, day_end) = local_day(now_ms);
        Ok(self
            .events()?
            .into_iter()
            .filter(|e| e.end > now_ms && e.start < day_end)
            .collect())
    }

    pub async fn status(&self, now_ms: i64) -> Result<CalendarStatus> {
        if self.check_pending().await.unwrap_or(false) {
            self.refresh(now_ms).await?;
        }
        let link = self.link()?;
        let events = self.events()?;
        let (day_start, day_end) = local_day(now_ms);
        let status = |p: Provider| ProviderStatus {
            state: match link.get(p) {
                None => LinkState::None,
                Some(l) if l.active => LinkState::Connected,
                Some(_) => LinkState::Pending,
            },
            today: events
                .iter()
                .filter(|e| e.provider == p && e.start >= day_start && e.start < day_end)
                .count() as u32,
        };
        Ok(CalendarStatus {
            has_key: self.key()?.is_some(),
            google: status(Provider::Google),
            outlook: status(Provider::Outlook),
        })
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[tauri::command]
pub async fn calendar_status(
    state: tauri::State<'_, Arc<CalendarService>>,
) -> Result<CalendarStatus> {
    state.status(now_ms()).await
}

#[tauri::command]
pub async fn connect_calendar<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, Arc<CalendarService>>,
    provider: Provider,
) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let url = state.connect(provider, now_ms()).await?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| MorselError::Io(e.to_string()))
}

#[tauri::command]
pub async fn disconnect_calendar(
    state: tauri::State<'_, Arc<CalendarService>>,
    provider: Provider,
) -> Result<()> {
    state.disconnect(provider).await
}

#[tauri::command]
pub async fn refresh_calendar(state: tauri::State<'_, Arc<CalendarService>>) -> Result<()> {
    state.check_pending().await?;
    state.refresh(now_ms()).await
}

#[tauri::command]
pub fn upcoming_events(
    state: tauri::State<'_, Arc<CalendarService>>,
) -> Result<Vec<CalendarEvent>> {
    state.upcoming(now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::VecSink;
    use crate::secrets::MemoryStore;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const NOW: i64 = 1_793_000_000_000;

    fn service(server: &MockServer) -> (tempfile::TempDir, CalendarService, Arc<MemoryStore>) {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Mutex::new(
            crate::db::open(&dir.path().join("m.db")).unwrap(),
        ));
        let secrets = Arc::new(MemoryStore::default());
        secrets.set(COMPOSIO_KEY, "ck").unwrap();
        let s = CalendarService::new(
            db,
            secrets.clone(),
            reqwest::Client::new(),
            &server.uri(),
            Arc::new(VecSink::default()),
        );
        (dir, s, secrets)
    }

    fn rfc(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms)
            .unwrap()
            .to_rfc3339()
    }

    async fn mock_connect(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/auth_configs"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({ "auth_config": { "id": "ac_g" } })),
            )
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/connected_accounts/link"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "redirect_url": "https://connect.composio.dev/link/x", "connected_account_id": "ca_g"
            })))
            .mount(server)
            .await;
    }

    async fn mock_events(server: &MockServer, events: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path("/connected_accounts/ca_g"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "ACTIVE" })))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/tools/execute/GOOGLECALENDAR_EVENTS_LIST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "successful": true, "data": { "items": events } })),
            )
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn connect_saves_a_pending_connection_and_returns_the_url() {
        let server = MockServer::start().await;
        mock_connect(&server).await;
        let (_d, s, _) = service(&server);
        let url = s.connect(Provider::Google, NOW).await.unwrap();
        assert_eq!(url, "https://connect.composio.dev/link/x");
        let link = s.link().unwrap();
        assert!(link.user_id.starts_with("morsel-"));
        assert_eq!(
            link.google,
            Some(ProviderLink {
                auth_config_id: "ac_g".into(),
                account_id: "ca_g".into(),
                active: false
            })
        );
        assert_eq!(link.outlook, None);
    }

    #[tokio::test]
    async fn connect_refuses_a_sign_in_page_that_is_not_https() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth_configs"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({ "auth_config": { "id": "ac_g" } })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/connected_accounts/link"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "redirect_url": "file:///Applications/Calculator.app", "connected_account_id": "ca_g"
            })))
            .mount(&server)
            .await;
        let (_d, s, _) = service(&server);
        assert!(s.connect(Provider::Google, NOW).await.is_err());
        assert_eq!(s.link().unwrap().google, None);
    }

    #[tokio::test]
    async fn poll_activates_a_pending_connection() {
        let server = MockServer::start().await;
        mock_connect(&server).await;
        mock_events(
            &server,
            json!([{ "id": "e1", "start": { "dateTime": rfc(NOW + 60_000) } }]),
        )
        .await;
        let (_d, s, _) = service(&server);
        s.connect(Provider::Google, NOW).await.unwrap();
        s.poll_once(NOW).await;
        assert!(s.link().unwrap().google.unwrap().active);
        assert_eq!(s.events().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn connect_without_a_key_is_refused() {
        let server = MockServer::start().await;
        let (_d, s, secrets) = service(&server);
        secrets.delete(COMPOSIO_KEY).unwrap();
        let err = s.connect(Provider::Google, NOW).await.unwrap_err();
        assert!(err.to_string().contains("Composio key"), "{err}");
    }

    #[tokio::test]
    async fn status_marks_a_pending_connection_connected_when_active() {
        let server = MockServer::start().await;
        mock_connect(&server).await;
        mock_events(
            &server,
            json!([{ "id": "e1", "summary": "Sync", "start": { "dateTime": rfc(NOW + 5 * 60_000) }, "end": { "dateTime": rfc(NOW + 35 * 60_000) } }]),
        )
        .await;
        let (_d, s, _) = service(&server);
        s.connect(Provider::Google, NOW).await.unwrap();
        let status = s.status(NOW).await.unwrap();
        assert!(status.has_key);
        assert_eq!(status.google.state, LinkState::Connected);
        assert_eq!(status.google.today, 1);
        assert_eq!(status.outlook.state, LinkState::None);
        assert_eq!(s.upcoming(NOW).unwrap()[0].title, "Sync");
    }

    #[tokio::test]
    async fn disconnect_deletes_the_account_and_the_link() {
        let server = MockServer::start().await;
        mock_connect(&server).await;
        mock_events(
            &server,
            json!([{ "id": "e1", "start": { "dateTime": rfc(NOW + 60_000) } }]),
        )
        .await;
        Mock::given(method("DELETE"))
            .and(path("/connected_accounts/ca_g"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let (_d, s, _) = service(&server);
        s.connect(Provider::Google, NOW).await.unwrap();
        s.status(NOW).await.unwrap();
        assert_eq!(s.events().unwrap().len(), 1);
        s.disconnect(Provider::Google).await.unwrap();
        assert_eq!(s.link().unwrap().google, None);
        assert!(s.events().unwrap().is_empty());
    }

    #[tokio::test]
    async fn upcoming_events_skips_past_events() {
        let server = MockServer::start().await;
        mock_connect(&server).await;
        mock_events(
            &server,
            json!([
                { "id": "past", "start": { "dateTime": rfc(NOW - 50 * 60_000) }, "end": { "dateTime": rfc(NOW - 20 * 60_000) } },
                { "id": "live", "start": { "dateTime": rfc(NOW - 5 * 60_000) }, "end": { "dateTime": rfc(NOW + 25 * 60_000) } },
                { "id": "next", "start": { "dateTime": rfc(NOW + 60_000) }, "end": { "dateTime": rfc(NOW + 31 * 60_000) } }
            ]),
        )
        .await;
        let (_d, s, _) = service(&server);
        s.connect(Provider::Google, NOW).await.unwrap();
        s.status(NOW).await.unwrap();
        let ids: Vec<String> = s.upcoming(NOW).unwrap().into_iter().map(|e| e.id).collect();
        assert_eq!(ids, vec!["live", "next"]);
    }

    #[tokio::test]
    async fn events_are_empty_after_the_key_is_deleted() {
        let server = MockServer::start().await;
        mock_connect(&server).await;
        mock_events(
            &server,
            json!([{ "id": "e1", "start": { "dateTime": rfc(NOW + 60_000) } }]),
        )
        .await;
        let (_d, s, secrets) = service(&server);
        s.connect(Provider::Google, NOW).await.unwrap();
        s.status(NOW).await.unwrap();
        secrets.delete(COMPOSIO_KEY).unwrap();
        assert!(s.events().unwrap().is_empty());
    }
}

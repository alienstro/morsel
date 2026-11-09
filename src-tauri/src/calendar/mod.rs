//! The optional calendar link through the user's own Composio account.

pub mod composio;
pub mod events;
pub mod service;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Google,
    Outlook,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Google, Provider::Outlook];

    /// The Composio toolkit slug.
    pub fn toolkit(self) -> &'static str {
        match self {
            Provider::Google => "googlecalendar",
            Provider::Outlook => "outlook",
        }
    }

    /// The Composio tool that lists the events in a time range.
    pub fn events_tool(self) -> &'static str {
        match self {
            Provider::Google => "GOOGLECALENDAR_EVENTS_LIST",
            Provider::Outlook => "OUTLOOK_GET_CALENDAR_VIEW",
        }
    }
}

/// One calendar meeting. Morsel reads only the title, the time, the video link and the people.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub id: String,
    pub provider: Provider,
    pub title: String,
    /// Unix time in milliseconds.
    pub start: i64,
    pub end: i64,
    pub link: Option<String>,
    pub people: Vec<String>,
}

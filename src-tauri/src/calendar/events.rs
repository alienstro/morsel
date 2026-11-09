//! Reads calendar events from Composio tool results. The shapes vary, so each read is defensive.

use super::{CalendarEvent, Provider};
use serde_json::Value;

const UNTITLED: &str = "Untitled meeting";

/// Finds the first array under `key`, at most 4 objects deep. Composio can wrap the provider
/// response in `data`, `response_data` or both.
fn find_array<'a>(v: &'a Value, key: &str, depth: u8) -> Option<&'a Vec<Value>> {
    let obj = v.as_object()?;
    if let Some(Value::Array(items)) = obj.get(key) {
        return Some(items);
    }
    if depth == 0 {
        return None;
    }
    obj.values()
        .find_map(|child| find_array(child, key, depth - 1))
}

fn text<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for key in path {
        cur = cur.get(key)?;
    }
    cur.as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// RFC 3339 with an offset, or a time without a zone (Outlook gives UTC when asked).
fn parse_time(raw: &str) -> Option<i64> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(t.timestamp_millis());
    }
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|t| t.and_utc().timestamp_millis())
}

fn http_link(raw: Option<&str>) -> Option<String> {
    raw.filter(|s| s.starts_with("https://") || s.starts_with("http://"))
        .map(str::to_owned)
}

pub fn parse_google_events(v: &Value) -> Vec<CalendarEvent> {
    let Some(items) = find_array(v, "items", 4) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|e| text(e, &["status"]) != Some("cancelled"))
        .filter_map(|e| {
            let start = parse_time(text(e, &["start", "dateTime"])?)?;
            let end = text(e, &["end", "dateTime"])
                .and_then(parse_time)
                .unwrap_or(start);
            let video = e
                .get("conferenceData")
                .and_then(|c| c.get("entryPoints"))
                .and_then(Value::as_array)
                .and_then(|points| {
                    points
                        .iter()
                        .find(|p| text(p, &["entryPointType"]) == Some("video"))
                })
                .and_then(|p| text(p, &["uri"]));
            let link = http_link(text(e, &["hangoutLink"]))
                .or_else(|| http_link(video))
                .or_else(|| http_link(text(e, &["location"])));
            let people = e
                .get("attendees")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter(|a| a.get("self").and_then(Value::as_bool) != Some(true))
                        .filter(|a| a.get("resource").and_then(Value::as_bool) != Some(true))
                        .filter_map(|a| {
                            text(a, &["displayName"])
                                .or_else(|| text(a, &["email"]))
                                .map(str::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(CalendarEvent {
                id: text(e, &["id"])?.to_owned(),
                provider: Provider::Google,
                title: text(e, &["summary"]).unwrap_or(UNTITLED).to_owned(),
                start,
                end,
                link,
                people,
            })
        })
        .collect()
}

pub fn parse_outlook_events(v: &Value) -> Vec<CalendarEvent> {
    let Some(items) = find_array(v, "value", 4) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|e| e.get("isCancelled").and_then(Value::as_bool) != Some(true))
        .filter(|e| e.get("isAllDay").and_then(Value::as_bool) != Some(true))
        .filter_map(|e| {
            let start = parse_time(text(e, &["start", "dateTime"])?)?;
            let end = text(e, &["end", "dateTime"])
                .and_then(parse_time)
                .unwrap_or(start);
            let link = http_link(text(e, &["onlineMeeting", "joinUrl"]))
                .or_else(|| http_link(text(e, &["onlineMeetingUrl"])));
            let people = e
                .get("attendees")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|a| {
                            text(a, &["emailAddress", "name"])
                                .or_else(|| text(a, &["emailAddress", "address"]))
                                .map(str::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(CalendarEvent {
                id: text(e, &["id"])?.to_owned(),
                provider: Provider::Outlook,
                title: text(e, &["subject"]).unwrap_or(UNTITLED).to_owned(),
                start,
                end,
                link,
                people,
            })
        })
        .collect()
}

/// The name of the video app of a meeting link.
pub fn meeting_place(link: &str) -> Option<&'static str> {
    let host = link.split("://").nth(1)?.split(['/', '?']).next()?;
    if host.ends_with("meet.google.com") {
        Some("Google Meet")
    } else if host.ends_with("zoom.us") {
        Some("Zoom")
    } else if host.ends_with("teams.microsoft.com") || host.ends_with("teams.live.com") {
        Some("Microsoft Teams")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ms(raw: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(raw)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn parse_google_events_reads_title_time_link_people() {
        let v = json!({ "data": { "items": [{
            "id": "g1", "summary": "Weekly product sync", "status": "confirmed",
            "start": { "dateTime": "2026-11-09T10:00:00+08:00" },
            "end": { "dateTime": "2026-11-09T10:30:00+08:00" },
            "hangoutLink": "https://meet.google.com/abc-defg-hij",
            "description": "Secret agenda",
            "attendees": [
                { "email": "me@example.com", "self": true },
                { "email": "sam@example.com", "displayName": "Sam" },
                { "email": "priya@example.com" },
                { "email": "room@example.com", "resource": true }
            ]
        }]}});
        let events = parse_google_events(&v);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.id, "g1");
        assert_eq!(e.title, "Weekly product sync");
        assert_eq!(e.start, ms("2026-11-09T02:00:00Z"));
        assert_eq!(e.end - e.start, 30 * 60_000);
        assert_eq!(
            e.link.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(e.people, vec!["Sam", "priya@example.com"]);
        assert_eq!(e.provider, Provider::Google);
    }

    #[test]
    fn parse_google_events_skips_all_day_and_cancelled() {
        let v = json!({ "items": [
            { "id": "a", "summary": "Holiday", "start": { "date": "2026-11-09" }, "end": { "date": "2026-11-10" } },
            { "id": "b", "summary": "Gone", "status": "cancelled", "start": { "dateTime": "2026-11-09T10:00:00Z" } },
            { "id": "c", "start": { "dateTime": "2026-11-09T11:00:00Z" } }
        ]});
        let events = parse_google_events(&v);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "c");
        assert_eq!(events[0].title, "Untitled meeting");
        assert!(events[0].people.is_empty());
        assert_eq!(events[0].link, None);
    }

    #[test]
    fn parse_google_events_accepts_nested_data() {
        let nested = json!({ "data": { "response_data": { "items": [
            { "id": "n", "summary": "Nested", "start": { "dateTime": "2026-11-09T11:00:00Z" },
              "conferenceData": { "entryPoints": [
                  { "entryPointType": "phone", "uri": "tel:+1" },
                  { "entryPointType": "video", "uri": "https://zoom.us/j/1" }
              ]}}
        ]}}});
        let events = parse_google_events(&nested);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].link.as_deref(), Some("https://zoom.us/j/1"));
        assert!(parse_google_events(&json!({ "data": {} })).is_empty());
        assert!(parse_google_events(&json!(null)).is_empty());
        assert!(parse_google_events(&json!({ "items": "nope" })).is_empty());
    }

    #[test]
    fn parse_outlook_events_reads_utc_times() {
        let v = json!({ "data": { "value": [
            { "id": "o1", "subject": "Acme call",
              "start": { "dateTime": "2026-11-09T02:00:00.0000000", "timeZone": "UTC" },
              "end": { "dateTime": "2026-11-09T02:45:00.0000000", "timeZone": "UTC" },
              "onlineMeeting": { "joinUrl": "https://teams.microsoft.com/l/meetup-join/1" },
              "attendees": [ { "emailAddress": { "name": "Dana", "address": "dana@acme.com" } },
                             { "emailAddress": { "address": "leo@example.com" } } ] },
            { "id": "o2", "subject": "Off", "isAllDay": true, "start": { "dateTime": "2026-11-09T00:00:00.0000000" } },
            { "id": "o3", "subject": "Cancelled", "isCancelled": true, "start": { "dateTime": "2026-11-09T05:00:00.0000000" } }
        ]}});
        let events = parse_outlook_events(&v);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].start, ms("2026-11-09T02:00:00Z"));
        assert_eq!(events[0].end - events[0].start, 45 * 60_000);
        assert_eq!(events[0].people, vec!["Dana", "leo@example.com"]);
        assert_eq!(
            events[0].link.as_deref(),
            Some("https://teams.microsoft.com/l/meetup-join/1")
        );
    }

    #[test]
    fn meeting_place_names_the_app() {
        assert_eq!(
            meeting_place("https://meet.google.com/x"),
            Some("Google Meet")
        );
        assert_eq!(
            meeting_place("https://acme.zoom.us/j/1?pwd=2"),
            Some("Zoom")
        );
        assert_eq!(
            meeting_place("https://teams.microsoft.com/l/1"),
            Some("Microsoft Teams")
        );
        assert_eq!(meeting_place("https://example.com/zoom.us"), None);
        assert_eq!(meeting_place("not a link"), None);
    }
}

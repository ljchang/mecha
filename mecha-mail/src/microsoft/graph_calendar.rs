//! Outlook calendar over Microsoft Graph, extracted from flowmail's
//! `calendar/outlook.rs` with the one fix that matters most:
//!
//! **`calendarView`, not `/events` with a `start/dateTime` filter.** flowmail
//! queries `/events` and filters on start time, which returns recurring
//! *series masters* only — so a weekly standup created in 2020 falls outside
//! any 2026 window and **vanishes from the results entirely**, while an event
//! whose master starts inside the window appears once instead of N times.
//! `calendarView` expands occurrences server-side over the window, which is
//! what "what's on my calendar this week" actually means, and it makes
//! flowmail's hand-rolled RRULE converter unnecessary.
//!
//! Also dropped: the delta/sync-token branch (`/events` never returns a
//! deltaLink, so it was dead code), and `account_id` from the constructor.

use serde_json::{json, Value};

use crate::http::send_with_retry;
use crate::microsoft::graph_mail::urlencode;
use crate::types::MailError;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

#[derive(Debug, Clone, serde::Serialize)]
pub struct Calendar {
    pub id: String,
    pub name: String,
    pub is_primary: bool,
    pub can_edit: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CalendarEvent {
    pub event_id: String,
    pub calendar_id: String,
    pub title: String,
    pub description: Option<String>,
    pub start_time: String,
    pub end_time: String,
    pub location: Option<String>,
    pub status: String,
    pub attendees: Vec<String>,
    pub organizer: Option<String>,
    pub is_all_day: bool,
    pub html_link: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateEventRequest {
    pub title: String,
    pub description: Option<String>,
    pub start_time: String,
    pub end_time: String,
    pub location: Option<String>,
    pub attendees: Vec<String>,
    pub all_day: bool,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateEventRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub location: Option<String>,
    pub attendees: Option<Vec<String>>,
    pub all_day: Option<bool>,
    pub timezone: Option<String>,
}

pub struct OutlookCalendarProvider {
    access_token: String,
    client: reqwest::Client,
}

impl OutlookCalendarProvider {
    pub fn new(access_token: String) -> Self {
        Self {
            access_token,
            client: crate::http::client(),
        }
    }

    async fn get_json(&self, url: &str) -> Result<Value, MailError> {
        let resp = send_with_retry(self.client.get(url).bearer_auth(&self.access_token)).await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(MailError::ApiError {
                status,
                message: super::auth::humanize_aadsts(&body),
            });
        }
        resp.json::<Value>().await.map_err(MailError::from)
    }

    /// Every calendar, including read-only ones. flowmail filtered out
    /// `canEdit == false`, which hid subscribed calendars entirely; the role
    /// is reported instead so the model knows where a write could go.
    pub async fn list_calendars(&self) -> Result<Vec<Calendar>, MailError> {
        let json = self.get_json(&format!("{GRAPH}/me/calendars")).await?;
        Ok(json["value"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|item| Calendar {
                        id: item["id"].as_str().unwrap_or_default().to_string(),
                        name: item["name"].as_str().unwrap_or_default().to_string(),
                        is_primary: item["isDefaultCalendar"].as_bool().unwrap_or(false),
                        can_edit: item["canEdit"].as_bool().unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Events in a window, with recurring series expanded into occurrences.
    pub async fn list_events(
        &self,
        calendar_id: &str,
        start: &str,
        end: &str,
    ) -> Result<Vec<CalendarEvent>, MailError> {
        let base = if calendar_id.is_empty() || calendar_id == "primary" {
            format!("{GRAPH}/me/calendarView")
        } else {
            format!(
                "{GRAPH}/me/calendars/{}/calendarView",
                urlencode(calendar_id)
            )
        };

        let mut url = format!(
            "{base}?startDateTime={}&endDateTime={}&$orderby=start/dateTime&$top=100",
            urlencode(start),
            urlencode(end)
        );
        let mut events = Vec::new();
        loop {
            let json = self.get_json(&url).await?;
            for item in json["value"].as_array().cloned().unwrap_or_default() {
                events.push(parse_event(&item, calendar_id));
            }
            match json["@odata.nextLink"].as_str() {
                Some(next) => url = next.to_string(),
                None => break,
            }
        }
        Ok(events)
    }

    pub async fn create_event(
        &self,
        calendar_id: &str,
        event: &CreateEventRequest,
    ) -> Result<CalendarEvent, MailError> {
        let tz = event.timezone.as_deref().unwrap_or("UTC");
        let mut body = json!({
            "subject": event.title,
            "isAllDay": event.all_day,
            "start": graph_time(&event.start_time, event.all_day, tz),
            "end": graph_time(&event.end_time, event.all_day, tz),
        });
        if let Some(desc) = &event.description {
            body["body"] = json!({"contentType": "text", "content": desc});
        }
        if let Some(loc) = &event.location {
            body["location"] = json!({"displayName": loc});
        }
        if !event.attendees.is_empty() {
            body["attendees"] = json!(event
                .attendees
                .iter()
                .map(|a| json!({"emailAddress": {"address": a}, "type": "required"}))
                .collect::<Vec<_>>());
        }

        let url = if calendar_id.is_empty() || calendar_id == "primary" {
            format!("{GRAPH}/me/events")
        } else {
            format!("{GRAPH}/me/calendars/{}/events", urlencode(calendar_id))
        };
        let resp = send_with_retry(
            self.client
                .post(&url)
                .bearer_auth(&self.access_token)
                .json(&body),
        )
        .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(MailError::ApiError {
                status,
                message: super::auth::humanize_aadsts(&text),
            });
        }
        Ok(parse_event(&resp.json::<Value>().await?, calendar_id))
    }

    pub async fn update_event(
        &self,
        event_id: &str,
        event: &UpdateEventRequest,
    ) -> Result<CalendarEvent, MailError> {
        let all_day = event.all_day.unwrap_or(false);
        let tz = event.timezone.as_deref().unwrap_or("UTC");
        let mut body = json!({});
        if let Some(t) = &event.title {
            body["subject"] = json!(t);
        }
        if let Some(d) = &event.description {
            body["body"] = json!({"contentType": "text", "content": d});
        }
        if let Some(l) = &event.location {
            body["location"] = json!({"displayName": l});
        }
        if let Some(s) = &event.start_time {
            body["start"] = graph_time(s, all_day, tz);
        }
        if let Some(e) = &event.end_time {
            body["end"] = graph_time(e, all_day, tz);
        }
        if let Some(a) = &event.attendees {
            body["attendees"] = json!(a
                .iter()
                .map(|x| json!({"emailAddress": {"address": x}, "type": "required"}))
                .collect::<Vec<_>>());
        }

        let resp = send_with_retry(
            self.client
                .patch(format!("{GRAPH}/me/events/{}", urlencode(event_id)))
                .bearer_auth(&self.access_token)
                .json(&body),
        )
        .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(MailError::ApiError {
                status,
                message: super::auth::humanize_aadsts(&text),
            });
        }
        Ok(parse_event(&resp.json::<Value>().await?, ""))
    }

    pub async fn delete_event(&self, event_id: &str) -> Result<(), MailError> {
        let resp = send_with_retry(
            self.client
                .delete(format!("{GRAPH}/me/events/{}", urlencode(event_id)))
                .bearer_auth(&self.access_token),
        )
        .await?;
        let status = resp.status().as_u16();
        // 404 means it is already gone, which is the outcome asked for.
        if resp.status().is_success() || status == 404 {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        Err(MailError::ApiError {
            status,
            message: super::auth::humanize_aadsts(&text),
        })
    }
}

/// Graph wants `{dateTime, timeZone}`; all-day events want midnight local.
fn graph_time(stamp: &str, all_day: bool, tz: &str) -> Value {
    if all_day {
        let date = &stamp[..stamp.len().min(10)];
        json!({"dateTime": format!("{date}T00:00:00"), "timeZone": tz})
    } else {
        json!({"dateTime": stamp, "timeZone": tz})
    }
}

fn parse_event(item: &Value, calendar_id: &str) -> CalendarEvent {
    let attendees = item["attendees"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a["emailAddress"]["address"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Each end carries its own zone; using the start's for both is wrong
    // across a DST boundary, which is what flowmail did.
    let stamp = |side: &str| -> String {
        let dt = item[side]["dateTime"].as_str().unwrap_or_default();
        match item[side]["timeZone"].as_str() {
            Some("UTC") => format!("{dt}Z"),
            Some(tz) => format!("{dt} {tz}"),
            None => dt.to_string(),
        }
    };

    let status = if item["isCancelled"].as_bool().unwrap_or(false) {
        "cancelled".to_string()
    } else {
        item["showAs"].as_str().unwrap_or("busy").to_string()
    };

    CalendarEvent {
        event_id: item["id"].as_str().unwrap_or_default().to_string(),
        calendar_id: calendar_id.to_string(),
        title: match item["subject"].as_str() {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            _ => "(No title)".to_string(),
        },
        description: item["bodyPreview"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from),
        start_time: stamp("start"),
        end_time: stamp("end"),
        location: item["location"]["displayName"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from),
        status,
        attendees,
        organizer: item["organizer"]["emailAddress"]["address"]
            .as_str()
            .map(String::from),
        is_all_day: item["isAllDay"].as_bool().unwrap_or(false),
        html_link: item["webLink"].as_str().map(String::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_event_parses_with_its_own_zone_per_side() {
        let item = json!({
            "id": "ev1", "subject": "Lab meeting",
            "start": {"dateTime": "2026-08-06T15:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-08-06T16:00:00.0000000", "timeZone": "UTC"},
            "attendees": [{"emailAddress": {"address": "priya@dartmouth.edu"}}],
            "organizer": {"emailAddress": {"address": "luke@dartmouth.edu"}},
            "isAllDay": false, "showAs": "busy",
            "webLink": "https://outlook.office.com/x"
        });
        let e = parse_event(&item, "primary");
        assert_eq!(e.title, "Lab meeting");
        assert!(e.start_time.ends_with('Z'));
        assert_eq!(e.attendees, vec!["priya@dartmouth.edu"]);
        assert_eq!(e.status, "busy");

        let cancelled = json!({"id": "e2", "subject": "", "isCancelled": true});
        let e = parse_event(&cancelled, "");
        assert_eq!(e.status, "cancelled");
        assert_eq!(
            e.title, "(No title)",
            "an empty subject must not render blank"
        );
    }

    #[test]
    fn all_day_events_start_at_local_midnight() {
        let t = graph_time("2026-08-10T09:30:00", true, "America/New_York");
        assert_eq!(t["dateTime"], "2026-08-10T00:00:00");
        let timed = graph_time("2026-08-10T09:30:00", false, "America/New_York");
        assert_eq!(timed["dateTime"], "2026-08-10T09:30:00");
        // Safe on short input, unlike flowmail's fixed slice.
        assert_eq!(graph_time("2026", true, "UTC")["dateTime"], "2026T00:00:00");
    }
}

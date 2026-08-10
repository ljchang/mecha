//! The Google Calendar v3 client, scoped to on-demand use: there is no
//! syncToken/showDeleted incremental machinery, because that serves a local
//! cache and we deliberately do not keep one. Reads ask for
//! `singleEvents=true`, so recurring events arrive as expanded instances —
//! an agent answering "what's on Thursday" wants occurrences, not RRULEs,
//! and `false` would mean re-expanding them here.

use serde_json::{json, Value};

use crate::google::gmail::urlencode;
use crate::http::send_with_retry;
use crate::types::MailError;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Calendar {
    pub id: String,
    pub name: String,
    pub is_primary: bool,
    pub access_role: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CalendarEvent {
    pub event_id: String,
    pub calendar_id: String,
    pub title: String,
    pub description: Option<String>,
    /// RFC 3339 for timed events; `YYYY-MM-DD` for all-day.
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

pub struct CalendarProvider {
    access_token: String,
    client: reqwest::Client,
}

impl CalendarProvider {
    pub fn new(access_token: String) -> Self {
        Self {
            access_token,
            client: crate::http::client(),
        }
    }

    /// Calendars the account can at least see. Filtering to writer/owner
    /// would suit a client that only pushes; we list everything readable and
    /// report the role, so the model knows which calendars a write could
    /// target.
    pub async fn list_calendars(&self) -> Result<Vec<Calendar>, MailError> {
        let resp = send_with_retry(
            self.client
                .get("https://www.googleapis.com/calendar/v3/users/me/calendarList")
                .bearer_auth(&self.access_token),
        )
        .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(MailError::ApiError {
                status,
                message: body,
            });
        }

        let body: Value = resp.json().await?;
        Ok(body["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|item| Calendar {
                id: item["id"].as_str().unwrap_or("").to_string(),
                name: item["summary"].as_str().unwrap_or("").to_string(),
                is_primary: item["primary"].as_bool().unwrap_or(false),
                access_role: item["accessRole"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }

    /// Events in a time window, expanded and start-ordered, paginated to the
    /// end of the window.
    pub async fn list_events(
        &self,
        calendar_id: &str,
        time_min: &str,
        time_max: &str,
    ) -> Result<Vec<CalendarEvent>, MailError> {
        let base_url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events",
            urlencode(calendar_id)
        );

        let mut all_events = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut request = self
                .client
                .get(&base_url)
                .bearer_auth(&self.access_token)
                .query(&[
                    ("singleEvents", "true"),
                    ("orderBy", "startTime"),
                    ("timeMin", time_min),
                    ("timeMax", time_max),
                ]);
            if let Some(ref pt) = page_token {
                request = request.query(&[("pageToken", pt.as_str())]);
            }

            let resp = send_with_retry(request).await?;
            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                return Err(MailError::ApiError {
                    status,
                    message: body,
                });
            }

            let body: Value = resp.json().await?;
            for item in body["items"].as_array().cloned().unwrap_or_default() {
                all_events.push(parse_event(&item, calendar_id));
            }

            page_token = body["nextPageToken"].as_str().map(|s| s.to_string());
            if page_token.is_none() {
                break;
            }
        }

        Ok(all_events)
    }

    pub async fn create_event(
        &self,
        calendar_id: &str,
        event: &CreateEventRequest,
    ) -> Result<CalendarEvent, MailError> {
        let (start_json, end_json) = if event.all_day {
            // All-day events use "date", not "dateTime".
            (
                json!({"date": date_part(&event.start_time)}),
                json!({"date": date_part(&event.end_time)}),
            )
        } else {
            let tz = event.timezone.as_deref().unwrap_or("UTC");
            (
                json!({"dateTime": event.start_time, "timeZone": tz}),
                json!({"dateTime": event.end_time, "timeZone": tz}),
            )
        };

        let mut body = json!({
            "summary": event.title,
            "start": start_json,
            "end": end_json,
        });
        if let Some(ref desc) = event.description {
            body["description"] = json!(desc);
        }
        if let Some(ref loc) = event.location {
            body["location"] = json!(loc);
        }
        if !event.attendees.is_empty() {
            body["attendees"] = json!(event
                .attendees
                .iter()
                .map(|a| json!({"email": a}))
                .collect::<Vec<_>>());
        }

        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events",
            urlencode(calendar_id)
        );
        // Google's default is sendUpdates=none, so attendees were silently
        // NOT invited despite the tool schema saying they would be — Graph
        // always mails attendees, and the surface has one behaviour. Set
        // only when there are attendees: the parameter is about them.
        let mut request = self.client.post(&url).bearer_auth(&self.access_token);
        if !event.attendees.is_empty() {
            request = request.query(&[("sendUpdates", "all")]);
        }
        let resp = send_with_retry(request.json(&body)).await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(MailError::ApiError {
                status,
                message: body,
            });
        }

        let result: Value = resp.json().await?;
        Ok(parse_event(&result, calendar_id))
    }

    pub async fn update_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        event: &UpdateEventRequest,
    ) -> Result<CalendarEvent, MailError> {
        let mut body = json!({});
        if let Some(ref title) = event.title {
            body["summary"] = json!(title);
        }
        if let Some(ref desc) = event.description {
            body["description"] = json!(desc);
        }
        if let Some(ref loc) = event.location {
            body["location"] = json!(loc);
        }
        if let Some(ref attendees) = event.attendees {
            body["attendees"] = json!(attendees
                .iter()
                .map(|a| json!({"email": a}))
                .collect::<Vec<_>>());
        }

        let all_day = event.all_day.unwrap_or(false);
        let tz = event.timezone.as_deref().unwrap_or("UTC");
        if let Some(ref start) = event.start_time {
            body["start"] = if all_day {
                json!({"date": date_part(start)})
            } else {
                json!({"dateTime": start, "timeZone": tz})
            };
        }
        if let Some(ref end) = event.end_time {
            body["end"] = if all_day {
                json!({"date": date_part(end)})
            } else {
                json!({"dateTime": end, "timeZone": tz})
            };
        }

        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events/{}",
            urlencode(calendar_id),
            urlencode(event_id)
        );
        let resp = send_with_retry(
            self.client
                .patch(&url)
                .bearer_auth(&self.access_token)
                .json(&body),
        )
        .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(MailError::ApiError {
                status,
                message: body,
            });
        }

        let result: Value = resp.json().await?;
        Ok(parse_event(&result, calendar_id))
    }

    /// Busy intervals from `freeBusy.query` — `(start, end)` stamp pairs,
    /// no event details, which is the privacy property the caller wants.
    /// Primary calendar only for now: secondary calendars that should count
    /// against availability are a policy question the caller owns, and the
    /// endpoint answers per-calendar when that day comes.
    pub async fn freebusy(
        &self,
        time_min: &str,
        time_max: &str,
    ) -> Result<Vec<(String, String)>, MailError> {
        let body = json!({
            "timeMin": time_min,
            "timeMax": time_max,
            "items": [{"id": "primary"}],
        });
        let resp = send_with_retry(
            self.client
                .post("https://www.googleapis.com/calendar/v3/freeBusy")
                .bearer_auth(&self.access_token)
                .json(&body),
        )
        .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(MailError::ApiError {
                status,
                message: body,
            });
        }
        parse_google_freebusy(&resp.json::<Value>().await?)
    }

    pub async fn delete_event(&self, calendar_id: &str, event_id: &str) -> Result<(), MailError> {
        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{}/events/{}",
            urlencode(calendar_id),
            urlencode(event_id)
        );
        // sendUpdates=all for the same reason as create: the tool schema
        // says attendees are notified of the cancellation, and Google's
        // default is that they are not. Harmless on an event with none.
        let resp = send_with_retry(
            self.client
                .delete(&url)
                .query(&[("sendUpdates", "all")])
                .bearer_auth(&self.access_token),
        )
        .await?;

        let status = resp.status().as_u16();
        // 204 = deleted; 404/410 = already gone, which is the outcome asked for.
        if status == 204 || status == 404 || status == 410 || resp.status().is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        Err(MailError::ApiError {
            status,
            message: body,
        })
    }
}

/// Pull the primary calendar's busy list out of a freeBusy response. The
/// response nests per-calendar errors rather than failing the request, so an
/// errored calendar must surface as an error here — swallowing it would
/// report a calendar that could not be read as a calendar that is free.
fn parse_google_freebusy(body: &Value) -> Result<Vec<(String, String)>, MailError> {
    let calendar = &body["calendars"]["primary"];
    if let Some(errors) = calendar["errors"].as_array().filter(|e| !e.is_empty()) {
        let reasons: Vec<&str> = errors.iter().filter_map(|e| e["reason"].as_str()).collect();
        return Err(MailError::ApiError {
            status: 200,
            message: format!("freeBusy reported errors: {}", reasons.join(", ")),
        });
    }
    Ok(calendar["busy"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    Some((
                        b["start"].as_str()?.to_string(),
                        b["end"].as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The first 10 chars of an RFC 3339 stamp — its `YYYY-MM-DD`. Safe on short
/// input, where a fixed `[..10]` slice would panic.
fn date_part(stamp: &str) -> &str {
    &stamp[..stamp.len().min(10)]
}

fn parse_event(item: &Value, calendar_id: &str) -> CalendarEvent {
    let is_all_day = item["start"]["date"].as_str().is_some();
    let start_time = item["start"]["dateTime"]
        .as_str()
        .or(item["start"]["date"].as_str())
        .unwrap_or("")
        .to_string();
    let end_time = item["end"]["dateTime"]
        .as_str()
        .or(item["end"]["date"].as_str())
        .unwrap_or("")
        .to_string();

    let attendees = item["attendees"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a["email"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    CalendarEvent {
        event_id: item["id"].as_str().unwrap_or("").to_string(),
        calendar_id: calendar_id.to_string(),
        title: item["summary"].as_str().unwrap_or("(No title)").to_string(),
        description: item["description"].as_str().map(|s| s.to_string()),
        start_time,
        end_time,
        location: item["location"].as_str().map(|s| s.to_string()),
        status: item["status"].as_str().unwrap_or("confirmed").to_string(),
        attendees,
        organizer: item["organizer"]["email"].as_str().map(|s| s.to_string()),
        is_all_day,
        html_link: item["htmlLink"].as_str().map(|s| s.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_event_parses_with_timed_and_all_day_starts() {
        let timed = json!({
            "id": "e1", "summary": "Lab meeting", "status": "confirmed",
            "start": {"dateTime": "2026-08-06T15:00:00Z"},
            "end": {"dateTime": "2026-08-06T16:00:00Z"},
            "attendees": [{"email": "priya@example.edu"}, {"email": "luke@example.edu"}],
            "organizer": {"email": "luke@example.edu"}
        });
        let e = parse_event(&timed, "primary");
        assert_eq!(e.title, "Lab meeting");
        assert!(!e.is_all_day);
        assert_eq!(e.attendees.len(), 2);

        let all_day = json!({
            "id": "e2", "summary": "Retreat",
            "start": {"date": "2026-08-10"}, "end": {"date": "2026-08-11"}
        });
        let e = parse_event(&all_day, "primary");
        assert!(e.is_all_day);
        assert_eq!(e.start_time, "2026-08-10");
    }

    #[test]
    fn freebusy_yields_interval_pairs() {
        let body = json!({
            "calendars": {"primary": {"busy": [
                {"start": "2026-08-10T14:00:00Z", "end": "2026-08-10T15:00:00Z"},
                {"start": "2026-08-11T09:00:00Z", "end": "2026-08-11T09:30:00Z"}
            ]}}
        });
        let busy = parse_google_freebusy(&body).unwrap();
        assert_eq!(busy.len(), 2);
        assert_eq!(busy[0].0, "2026-08-10T14:00:00Z");

        let empty = json!({"calendars": {"primary": {"busy": []}}});
        assert!(parse_google_freebusy(&empty).unwrap().is_empty());
    }

    /// freeBusy nests failures instead of failing the request; an errored
    /// calendar must not read as a free one.
    #[test]
    fn freebusy_calendar_errors_are_errors_not_free_time() {
        let body = json!({
            "calendars": {"primary": {"errors": [{"domain": "global", "reason": "notFound"}]}}
        });
        let err = parse_google_freebusy(&body).unwrap_err();
        assert!(err.to_string().contains("notFound"), "{err}");
    }

    #[test]
    fn date_part_is_safe_on_short_input() {
        assert_eq!(date_part("2026-08-10T09:00:00Z"), "2026-08-10");
        assert_eq!(date_part("2026"), "2026");
        assert_eq!(date_part(""), "");
    }
}

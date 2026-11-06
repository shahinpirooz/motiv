//! Motiv: calendar events and meeting participants from Microsoft 365
//! (Graph) and Google Calendar.
//!
//! - Upcoming events (cached a few minutes) for meeting awareness, so a
//!   detected call is named after its calendar event.
//! - Each recorded meeting is matched to the calendar event that overlaps it
//!   most (or one the user picks); the event's organizer and attendees are
//!   stored with the meeting and given to chat, action items and summaries.
//! - For Teams meetings organised by the user, Graph attendance reports say
//!   who actually joined and for how long.

use crate::accounts::{self, oauth::Provider};
use crate::state::AppState;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDateTime, TimeZone, Utc};
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Runtime};

const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const GRAPH: &str = "https://graph.microsoft.com/v1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Person {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attendee {
    pub name: String,
    pub email: String,
    /// accepted / declined / tentative / none / organizer
    pub response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalendarEvent {
    pub provider: Provider,
    pub id: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub organizer: Option<Person>,
    pub attendees: Vec<Attendee>,
    pub join_url: Option<String>,
    /// "teams" | "zoom" | "meet" when the event has a call link.
    pub platform: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttendanceRecord {
    pub name: String,
    pub email: Option<String>,
    pub role: Option<String>,
    pub seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingLink {
    pub meeting_id: String,
    pub event: CalendarEvent,
    pub attendance: Option<Vec<AttendanceRecord>>,
    pub source: String,
}

// ---------------------------------------------------------------------------
// Parsing (pure, unit tested)
// ---------------------------------------------------------------------------

pub fn platform_of(url: &str) -> Option<&'static str> {
    let u = url.to_lowercase();
    if u.contains("teams.microsoft.com") || u.contains("teams.live.com") {
        Some("teams")
    } else if u.contains("zoom.us/") || u.contains("zoomgov.com/") {
        Some("zoom")
    } else if u.contains("meet.google.com/") {
        Some("meet")
    } else {
        None
    }
}

/// First Teams/Zoom/Meet link found in free text (location, body, description).
pub fn find_call_link(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == '"' || c == '\'' || c == '(' || c == ')')
        .filter(|w| w.starts_with("https://"))
        .map(|w| w.trim_end_matches(|c: char| c == '.' || c == ',' || c == ';'))
        .find(|w| platform_of(w).is_some())
        .map(str::to_string)
}

fn s<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_str()
}

fn graph_time(v: &Value) -> Option<DateTime<Utc>> {
    // With Prefer: outlook.timezone="UTC": {"dateTime":"2026-11-06T17:00:00.0000000","timeZone":"UTC"}
    let dt = v.get("dateTime")?.as_str()?;
    let trimmed = dt.split('.').next()?;
    NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S").ok().map(|n| Utc.from_utc_datetime(&n))
}

pub fn parse_graph_events(body: &Value) -> Vec<CalendarEvent> {
    let Some(items) = body.get("value").and_then(|v| v.as_array()) else { return Vec::new() };
    items
        .iter()
        .filter(|e| !e.get("isCancelled").and_then(|v| v.as_bool()).unwrap_or(false))
        .filter_map(|e| {
            let start = graph_time(e.get("start")?)?;
            let end = graph_time(e.get("end")?)?;
            let organizer = e.get("organizer").and_then(|o| o.get("emailAddress")).map(|a| Person {
                name: s(a, &["name"]).unwrap_or_default().to_string(),
                email: s(a, &["address"]).unwrap_or_default().to_lowercase(),
            });
            let attendees = e
                .get("attendees")
                .and_then(|a| a.as_array())
                .map(|list| {
                    list.iter()
                        .filter(|a| s(a, &["type"]) != Some("resource"))
                        .map(|a| Attendee {
                            name: s(a, &["emailAddress", "name"]).unwrap_or_default().to_string(),
                            email: s(a, &["emailAddress", "address"]).unwrap_or_default().to_lowercase(),
                            response: s(a, &["status", "response"]).unwrap_or("none").to_lowercase().replace("tentativelyaccepted", "tentative").replace("notresponded", "none"),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let join_url = s(e, &["onlineMeeting", "joinUrl"])
                .map(str::to_string)
                .or_else(|| s(e, &["location", "displayName"]).and_then(find_call_link))
                .or_else(|| s(e, &["bodyPreview"]).and_then(find_call_link));
            let platform = join_url.as_deref().and_then(platform_of).map(str::to_string).or_else(|| {
                (s(e, &["onlineMeetingProvider"]) == Some("teamsForBusiness")).then(|| "teams".to_string())
            });
            Some(CalendarEvent {
                provider: Provider::Microsoft,
                id: s(e, &["id"])?.to_string(),
                title: s(e, &["subject"]).filter(|t| !t.trim().is_empty()).unwrap_or("(No title)").to_string(),
                start,
                end,
                all_day: e.get("isAllDay").and_then(|v| v.as_bool()).unwrap_or(false),
                organizer,
                attendees,
                join_url,
                platform,
            })
        })
        .collect()
}

fn google_time(v: &Value) -> Option<(DateTime<Utc>, bool)> {
    if let Some(dt) = v.get("dateTime").and_then(|x| x.as_str()) {
        return DateTime::parse_from_rfc3339(dt).ok().map(|d| (d.with_timezone(&Utc), false));
    }
    let d = v.get("date")?.as_str()?;
    let n = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()?.and_hms_opt(0, 0, 0)?;
    Some((Utc.from_utc_datetime(&n), true))
}

pub fn parse_google_events(body: &Value) -> Vec<CalendarEvent> {
    let Some(items) = body.get("items").and_then(|v| v.as_array()) else { return Vec::new() };
    items
        .iter()
        .filter(|e| s(e, &["status"]) != Some("cancelled"))
        .filter_map(|e| {
            let (start, all_day) = google_time(e.get("start")?)?;
            let (end, _) = google_time(e.get("end")?)?;
            let attendees = e
                .get("attendees")
                .and_then(|a| a.as_array())
                .map(|list| {
                    list.iter()
                        .filter(|a| !a.get("resource").and_then(|r| r.as_bool()).unwrap_or(false))
                        .map(|a| Attendee {
                            name: s(a, &["displayName"]).unwrap_or_default().to_string(),
                            email: s(a, &["email"]).unwrap_or_default().to_lowercase(),
                            response: match s(a, &["responseStatus"]).unwrap_or("needsAction") {
                                "needsAction" => "none".into(),
                                other => other.to_lowercase(),
                            },
                        })
                        .collect()
                })
                .unwrap_or_default();
            let video = e
                .get("conferenceData")
                .and_then(|c| c.get("entryPoints"))
                .and_then(|p| p.as_array())
                .and_then(|p| p.iter().find(|x| s(x, &["entryPointType"]) == Some("video")))
                .and_then(|x| s(x, &["uri"]))
                .map(str::to_string);
            let join_url = video
                .or_else(|| s(e, &["hangoutLink"]).map(str::to_string))
                .or_else(|| s(e, &["location"]).and_then(find_call_link))
                .or_else(|| s(e, &["description"]).and_then(find_call_link));
            Some(CalendarEvent {
                provider: Provider::Google,
                id: s(e, &["id"])?.to_string(),
                title: s(e, &["summary"]).filter(|t| !t.trim().is_empty()).unwrap_or("(No title)").to_string(),
                start,
                end,
                all_day,
                organizer: e.get("organizer").map(|o| Person {
                    name: s(o, &["displayName"]).unwrap_or_default().to_string(),
                    email: s(o, &["email"]).unwrap_or_default().to_lowercase(),
                }),
                attendees,
                platform: join_url.as_deref().and_then(platform_of).map(str::to_string),
                join_url,
            })
        })
        .collect()
}

pub fn parse_attendance(body: &Value) -> Vec<AttendanceRecord> {
    body.get("attendanceRecords")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .map(|r| AttendanceRecord {
                    name: s(r, &["identity", "displayName"]).unwrap_or("Unknown").to_string(),
                    email: s(r, &["emailAddress"]).filter(|e| !e.is_empty()).map(|e| e.to_lowercase()),
                    role: s(r, &["role"]).map(str::to_string),
                    seconds: r.get("totalAttendanceInSeconds").and_then(|v| v.as_i64()).unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The event that best matches a recording from `start` to `end`: timed
/// events overlapping it, scored by overlap, preferring events with a call
/// link (and the detected platform, if any).
pub fn best_match<'a>(events: &'a [CalendarEvent], start: DateTime<Utc>, end: DateTime<Utc>, platform: Option<&str>) -> Option<&'a CalendarEvent> {
    let slack = ChronoDuration::minutes(10);
    events
        .iter()
        .filter(|e| !e.all_day && e.start - slack <= end && start <= e.end + slack)
        .map(|e| {
            let overlap = (e.end.min(end) - e.start.max(start)).num_seconds().max(0);
            let mut score = overlap as f64 + 60.0;
            if e.platform.is_some() {
                score *= 1.5;
            }
            if platform.is_some() && e.platform.as_deref() == platform {
                score *= 2.0;
            }
            score -= (e.start - start).num_seconds().abs() as f64 / 60.0;
            (e, score)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(e, _)| e)
}

/// "Participants" line for LLM prompts.
pub fn participants_note(link: &MeetingLink) -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    let mut push = |n: &str, e: &str| {
        let label = if !n.trim().is_empty() { n.trim().to_string() } else { e.split('@').next().unwrap_or(e).to_string() };
        if !label.is_empty() && !names.iter().any(|x| x.eq_ignore_ascii_case(&label)) {
            names.push(label);
        }
    };
    if let Some(att) = &link.attendance {
        for r in att {
            push(&r.name, r.email.as_deref().unwrap_or(""));
        }
    } else {
        if let Some(o) = &link.event.organizer {
            push(&o.name, &o.email);
        }
        for a in link.event.attendees.iter().filter(|a| a.response != "declined") {
            push(&a.name, &a.email);
        }
    }
    (!names.is_empty()).then(|| {
        format!(
            "Calendar event: \"{}\". {}: {}. Use these names (and spellings) for speakers and owners where the transcript supports it.",
            link.event.title,
            if link.attendance.is_some() { "Attended" } else { "Invited" },
            names.join(", ")
        )
    })
}

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

static CACHE: LazyLock<Mutex<Option<(Instant, DateTime<Utc>, DateTime<Utc>, Vec<CalendarEvent>)>>> = LazyLock::new(|| Mutex::new(None));

pub fn invalidate_cache() {
    *CACHE.lock().unwrap() = None;
}

async fn get_json(url: &str, token: &str, prefer_utc: bool) -> Result<Value, String> {
    let mut req = reqwest::Client::new().get(url).bearer_auth(token).timeout(Duration::from_secs(30));
    if prefer_utc {
        req = req.header("Prefer", "outlook.timezone=\"UTC\"");
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = s(&body, &["error", "message"]).unwrap_or_default();
        return Err(format!("{} {}", status.as_u16(), msg).trim().to_string());
    }
    Ok(body)
}

fn iso(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

async fn fetch_graph(token: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<CalendarEvent>, String> {
    let mut url = format!(
        "{}/me/calendarView?startDateTime={}&endDateTime={}&$top=100&$orderby=start/dateTime&$select=id,subject,start,end,isAllDay,isCancelled,organizer,attendees,onlineMeeting,onlineMeetingProvider,location,bodyPreview",
        GRAPH,
        iso(from),
        iso(to)
    );
    let mut out = Vec::new();
    for _ in 0..10 {
        let body = get_json(&url, token, true).await?;
        out.extend(parse_graph_events(&body));
        match s(&body, &["@odata.nextLink"]) {
            Some(next) => url = next.to_string(),
            None => break,
        }
    }
    Ok(out)
}

async fn fetch_google(token: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<CalendarEvent>, String> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    for _ in 0..10 {
        let mut url = url::Url::parse("https://www.googleapis.com/calendar/v3/calendars/primary/events").unwrap();
        url.query_pairs_mut()
            .append_pair("timeMin", &iso(from))
            .append_pair("timeMax", &iso(to))
            .append_pair("singleEvents", "true")
            .append_pair("orderBy", "startTime")
            .append_pair("maxResults", "250");
        if let Some(p) = &page {
            url.query_pairs_mut().append_pair("pageToken", p);
        }
        let body = get_json(url.as_str(), token, false).await?;
        out.extend(parse_google_events(&body));
        page = s(&body, &["nextPageToken"]).map(str::to_string);
        if page.is_none() {
            break;
        }
    }
    Ok(out)
}

/// Events from every connected calendar between `from` and `to`, sorted by start.
pub async fn events_between<R: Runtime>(app: &AppHandle<R>, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<CalendarEvent>, String> {
    let mut all = Vec::new();
    let mut errors = Vec::new();
    for p in [Provider::Microsoft, Provider::Google] {
        if !accounts::is_connected(app, p) {
            continue;
        }
        let r = match accounts::access_token(app, p).await {
            Ok(token) => match p {
                Provider::Microsoft => fetch_graph(&token, from, to).await,
                Provider::Google => fetch_google(&token, from, to).await,
            },
            Err(e) => Err(e),
        };
        match r {
            Ok(ev) => all.extend(ev),
            Err(e) => {
                warn!("calendar: {} fetch failed: {}", p.key(), e);
                errors.push(format!("{}: {}", p.label(), e));
            }
        }
    }
    if all.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    all.sort_by_key(|e| e.start);
    Ok(all)
}

/// Today's and tomorrow's events (cached), used by meeting awareness.
pub async fn upcoming<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<CalendarEvent>, String> {
    let now = Utc::now();
    if let Some((at, from, to, ev)) = CACHE.lock().unwrap().clone() {
        if at.elapsed() < CACHE_TTL && from <= now - ChronoDuration::hours(2) && now + ChronoDuration::hours(12) <= to {
            return Ok(ev);
        }
    }
    let (from, to) = (now - ChronoDuration::hours(6), now + ChronoDuration::hours(36));
    let ev = events_between(app, from, to).await?;
    *CACHE.lock().unwrap() = Some((Instant::now(), from, to, ev.clone()));
    Ok(ev)
}

/// The calendar event happening now that best fits a detected call.
pub async fn current_event<R: Runtime>(app: &AppHandle<R>, platform: Option<&str>) -> Option<CalendarEvent> {
    let ev = upcoming(app).await.ok()?;
    let now = Utc::now();
    best_match(&ev, now, now, platform).cloned()
}

async fn teams_attendance(token: &str, join_url: &str) -> Result<Vec<AttendanceRecord>, String> {
    let filter = format!("JoinWebUrl eq '{}'", join_url.replace('\'', "''"));
    let mut url = url::Url::parse(&format!("{}/me/onlineMeetings", GRAPH)).unwrap();
    url.query_pairs_mut().append_pair("$filter", &filter);
    let body = get_json(url.as_str(), token, false).await?;
    let id = body
        .get("value")
        .and_then(|v| v.as_array())
        .and_then(|v| v.first())
        .and_then(|m| s(m, &["id"]))
        .ok_or("Attendance is only available for Teams meetings you organised.")?
        .to_string();
    let reports = get_json(&format!("{}/me/onlineMeetings/{}/attendanceReports", GRAPH, id), token, false).await?;
    let report_id = reports
        .get("value")
        .and_then(|v| v.as_array())
        .and_then(|v| v.iter().max_by_key(|r| s(r, &["meetingEndDateTime"]).unwrap_or_default().to_string()))
        .and_then(|r| s(r, &["id"]))
        .ok_or("No attendance report yet. Teams creates it a few minutes after the meeting ends.")?
        .to_string();
    let detail = get_json(
        &format!("{}/me/onlineMeetings/{}/attendanceReports/{}?$expand=attendanceRecords", GRAPH, id, report_id),
        token,
        false,
    )
    .await?;
    Ok(parse_attendance(&detail))
}

// ---------------------------------------------------------------------------
// Meeting links
// ---------------------------------------------------------------------------

pub async fn get_link(pool: &SqlitePool, meeting_id: &str) -> Option<MeetingLink> {
    let row: (String, Option<String>, String) =
        sqlx::query_as("SELECT event, attendance, source FROM meeting_calendar_links WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await
            .ok()??;
    Some(MeetingLink {
        meeting_id: meeting_id.to_string(),
        event: serde_json::from_str(&row.0).ok()?,
        attendance: row.1.and_then(|a| serde_json::from_str(&a).ok()),
        source: row.2,
    })
}

async fn save_link(pool: &SqlitePool, link: &MeetingLink) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO meeting_calendar_links (meeting_id, provider, event_id, event, attendance, source, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(meeting_id) DO UPDATE SET provider = excluded.provider, event_id = excluded.event_id,
           event = excluded.event, attendance = excluded.attendance, source = excluded.source, updated_at = excluded.updated_at",
    )
    .bind(&link.meeting_id)
    .bind(link.event.provider.key())
    .bind(&link.event.id)
    .bind(serde_json::to_string(&link.event).map_err(|e| e.to_string())?)
    .bind(link.attendance.as_ref().map(|a| serde_json::to_string(a).unwrap_or_default()))
    .bind(&link.source)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to save calendar link: {}", e))?;
    Ok(())
}

/// When the recording ran: it is saved when it ends, and the transcript says how long it was.
async fn recording_span(pool: &SqlitePool, meeting_id: &str) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let (created,): (String,) = sqlx::query_as("SELECT created_at FROM meetings WHERE id = ?").bind(meeting_id).fetch_optional(pool).await.ok()??;
    let end = DateTime::parse_from_rfc3339(&created).ok()?.with_timezone(&Utc);
    let (dur,): (Option<f64>,) = sqlx::query_as("SELECT MAX(audio_end_time) FROM transcripts WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_one(pool)
        .await
        .ok()?;
    let secs = dur.unwrap_or(0.0).max(60.0) as i64;
    Some((end - ChronoDuration::seconds(secs), end))
}

/// Participants line for a meeting's LLM prompts, if it's linked to an event.
pub async fn participants_for_prompt(pool: &SqlitePool, meeting_id: &str) -> Option<String> {
    get_link(pool, meeting_id).await.as_ref().and_then(participants_note)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn calendar_upcoming<R: Runtime>(app: AppHandle<R>) -> Result<Vec<CalendarEvent>, String> {
    let now = Utc::now();
    Ok(upcoming(&app).await?.into_iter().filter(|e| e.end >= now).collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct MeetingCalendar {
    pub link: Option<MeetingLink>,
    /// Events around the recording, to pick a different one.
    pub candidates: Vec<CalendarEvent>,
    pub connected: bool,
}

/// The meeting's calendar event (matching one automatically the first time)
/// and nearby events to choose from.
#[tauri::command]
pub async fn calendar_for_meeting<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>, meeting_id: String) -> Result<MeetingCalendar, String> {
    let pool = state.db_manager.pool().clone();
    let connected = [Provider::Microsoft, Provider::Google].iter().any(|p| accounts::is_connected(&app, *p));
    let mut link = get_link(&pool, &meeting_id).await;
    let mut candidates = Vec::new();
    if connected {
        if let Some((start, end)) = recording_span(&pool, &meeting_id).await {
            let day_start = start - ChronoDuration::hours(4);
            let day_end = end + ChronoDuration::hours(4);
            candidates = events_between(&app, day_start, day_end).await.unwrap_or_default();
            candidates.retain(|e| !e.all_day);
            if link.is_none() {
                if let Some(ev) = best_match(&candidates, start, end, None) {
                    let l = MeetingLink { meeting_id: meeting_id.clone(), event: ev.clone(), attendance: None, source: "auto".into() };
                    save_link(&pool, &l).await?;
                    link = Some(l);
                }
            }
        }
    }
    Ok(MeetingCalendar { link, candidates, connected })
}

#[tauri::command]
pub async fn calendar_set_meeting_event(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    event: Option<CalendarEvent>,
) -> Result<Option<MeetingLink>, String> {
    let pool = state.db_manager.pool();
    match event {
        None => {
            sqlx::query("DELETE FROM meeting_calendar_links WHERE meeting_id = ?").bind(&meeting_id).execute(pool).await.map_err(|e| e.to_string())?;
            Ok(None)
        }
        Some(ev) => {
            let l = MeetingLink { meeting_id, event: ev, attendance: None, source: "manual".into() };
            save_link(pool, &l).await?;
            Ok(Some(l))
        }
    }
}

/// Fetches who actually attended (Teams meetings you organised).
#[tauri::command]
pub async fn calendar_refresh_attendance<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>, meeting_id: String) -> Result<MeetingLink, String> {
    let pool = state.db_manager.pool();
    let mut link = get_link(pool, &meeting_id).await.ok_or("Link this meeting to a calendar event first.")?;
    if link.event.provider != Provider::Microsoft || link.event.platform.as_deref() != Some("teams") {
        return Err("Attendance reports are available for Teams meetings in Microsoft 365.".into());
    }
    if !accounts::load_config(&app).microsoft.teams_attendance {
        return Err("Turn on Teams attendance in Settings > Accounts and connect again.".into());
    }
    let join = link.event.join_url.clone().ok_or("This event has no Teams link.")?;
    let token = accounts::access_token(&app, Provider::Microsoft).await?;
    let records = teams_attendance(&token, &join).await.map_err(|e| {
        if e.starts_with("403") { "Microsoft 365 denied access to attendance reports. An admin needs to consent to OnlineMeetingArtifact.Read.All.".into() } else { e }
    })?;
    link.attendance = Some(records);
    save_link(pool, &link).await?;
    Ok(link)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn detects_platforms_and_links() {
        assert_eq!(platform_of("https://teams.microsoft.com/l/meetup-join/19%3a"), Some("teams"));
        assert_eq!(platform_of("https://acme.zoom.us/j/123?pwd=x"), Some("zoom"));
        assert_eq!(platform_of("https://meet.google.com/abc-defg-hij"), Some("meet"));
        assert_eq!(platform_of("https://example.com"), None);
        assert_eq!(find_call_link("Join: <https://us02web.zoom.us/j/8812?pwd=Q>. Thanks"), Some("https://us02web.zoom.us/j/8812?pwd=Q".into()));
        assert_eq!(find_call_link("Room 4"), None);
    }

    #[test]
    fn parses_graph() {
        let body = json!({"value": [
            {"id":"e1","subject":"Weekly Planning","isAllDay":false,"isCancelled":false,
             "start":{"dateTime":"2026-11-06T17:00:00.0000000","timeZone":"UTC"},
             "end":{"dateTime":"2026-11-06T17:30:00.0000000","timeZone":"UTC"},
             "organizer":{"emailAddress":{"name":"Shahin Pirooz","address":"Shahin@WhiteDog.com"}},
             "attendees":[
               {"type":"required","emailAddress":{"name":"Priya N","address":"priya@whitedog.com"},"status":{"response":"accepted"}},
               {"type":"resource","emailAddress":{"name":"Room 4","address":"room4@whitedog.com"},"status":{"response":"accepted"}},
               {"type":"optional","emailAddress":{"name":"Sam","address":"sam@x.com"},"status":{"response":"tentativelyAccepted"}}],
             "onlineMeeting":{"joinUrl":"https://teams.microsoft.com/l/meetup-join/abc"}},
            {"id":"e2","subject":"Lunch","isCancelled":true,"start":{"dateTime":"2026-11-06T19:00:00","timeZone":"UTC"},"end":{"dateTime":"2026-11-06T20:00:00","timeZone":"UTC"}},
            {"id":"e3","subject":"","start":{"dateTime":"2026-11-07T01:00:00.0000000","timeZone":"UTC"},"end":{"dateTime":"2026-11-07T02:00:00.0000000","timeZone":"UTC"},
             "location":{"displayName":"https://acme.zoom.us/j/1"}}
        ]});
        let ev = parse_graph_events(&body);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].title, "Weekly Planning");
        assert_eq!(ev[0].start, t("2026-11-06T17:00:00Z"));
        assert_eq!(ev[0].platform.as_deref(), Some("teams"));
        assert_eq!(ev[0].organizer.as_ref().unwrap().email, "shahin@whitedog.com");
        assert_eq!(ev[0].attendees.len(), 2);
        assert_eq!(ev[0].attendees[1].response, "tentative");
        assert_eq!(ev[1].title, "(No title)");
        assert_eq!(ev[1].platform.as_deref(), Some("zoom"));
    }

    #[test]
    fn parses_google() {
        let body = json!({"items": [
            {"id":"g1","summary":"Roadmap","status":"confirmed",
             "start":{"dateTime":"2026-11-06T09:00:00-08:00"},"end":{"dateTime":"2026-11-06T10:00:00-08:00"},
             "organizer":{"email":"lead@x.com","displayName":"Lead"},
             "attendees":[{"email":"me@x.com","self":true,"responseStatus":"accepted"},{"email":"bob@x.com","responseStatus":"needsAction"},{"email":"room@resource.calendar.google.com","resource":true}],
             "conferenceData":{"entryPoints":[{"entryPointType":"phone","uri":"tel:+1"},{"entryPointType":"video","uri":"https://meet.google.com/abc-defg-hij"}]}},
            {"id":"g2","summary":"Holiday","start":{"date":"2026-11-11"},"end":{"date":"2026-11-12"}}
        ]});
        let ev = parse_google_events(&body);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].start, t("2026-11-06T17:00:00Z"));
        assert_eq!(ev[0].platform.as_deref(), Some("meet"));
        assert_eq!(ev[0].attendees.len(), 2);
        assert_eq!(ev[0].attendees[1].response, "none");
        assert!(ev[1].all_day);
    }

    #[test]
    fn parses_attendance() {
        let body = json!({"attendanceRecords":[
            {"emailAddress":"Priya@WhiteDog.com","totalAttendanceInSeconds":1740,"role":"Presenter","identity":{"displayName":"Priya N"}},
            {"emailAddress":"","totalAttendanceInSeconds":300,"role":"Attendee","identity":{"displayName":"Guest"}}]});
        let r = parse_attendance(&body);
        assert_eq!(r[0], AttendanceRecord { name: "Priya N".into(), email: Some("priya@whitedog.com".into()), role: Some("Presenter".into()), seconds: 1740 });
        assert_eq!(r[1].email, None);
    }

    fn ev(id: &str, s: &str, e: &str, platform: Option<&str>) -> CalendarEvent {
        CalendarEvent {
            provider: Provider::Microsoft, id: id.into(), title: id.into(), start: t(s), end: t(e), all_day: false,
            organizer: None, attendees: vec![], join_url: None, platform: platform.map(str::to_string),
        }
    }

    #[test]
    fn matches_recording_to_event() {
        let events = vec![
            ev("standup", "2026-11-06T16:00:00Z", "2026-11-06T16:15:00Z", Some("teams")),
            ev("focus", "2026-11-06T16:00:00Z", "2026-11-06T18:00:00Z", None),
            ev("planning", "2026-11-06T17:00:00Z", "2026-11-06T17:30:00Z", Some("teams")),
            ev("zoomcall", "2026-11-06T17:00:00Z", "2026-11-06T17:30:00Z", Some("zoom")),
        ];
        // Recording 17:02-17:28: the call events beat the long focus block.
        let m = best_match(&events, t("2026-11-06T17:02:00Z"), t("2026-11-06T17:28:00Z"), None).unwrap();
        assert!(m.id == "planning" || m.id == "zoomcall");
        let m = best_match(&events, t("2026-11-06T17:02:00Z"), t("2026-11-06T17:28:00Z"), Some("zoom")).unwrap();
        assert_eq!(m.id, "zoomcall");
        // A call detected right now at 16:05 -> standup.
        let now = t("2026-11-06T16:05:00Z");
        assert_eq!(best_match(&events, now, now, Some("teams")).unwrap().id, "standup");
        assert!(best_match(&events, t("2026-11-06T22:00:00Z"), t("2026-11-06T22:30:00Z"), None).is_none());
    }

    #[test]
    fn participants_note_prefers_attendance() {
        let mut e = ev("Planning", "2026-11-06T17:00:00Z", "2026-11-06T17:30:00Z", Some("teams"));
        e.organizer = Some(Person { name: "Shahin Pirooz".into(), email: "s@w.com".into() });
        e.attendees = vec![
            Attendee { name: "Priya N".into(), email: "p@w.com".into(), response: "accepted".into() },
            Attendee { name: "".into(), email: "bob@w.com".into(), response: "none".into() },
            Attendee { name: "Declined Dan".into(), email: "d@w.com".into(), response: "declined".into() },
        ];
        let mut link = MeetingLink { meeting_id: "m".into(), event: e, attendance: None, source: "auto".into() };
        let n = participants_note(&link).unwrap();
        assert!(n.contains("Invited: Shahin Pirooz, Priya N, bob."));
        assert!(!n.contains("Dan"));
        link.attendance = Some(vec![AttendanceRecord { name: "Priya N".into(), email: None, role: None, seconds: 60 }]);
        assert!(participants_note(&link).unwrap().contains("Attended: Priya N."));
    }
}

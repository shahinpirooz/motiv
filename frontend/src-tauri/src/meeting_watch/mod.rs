//! Motiv: meeting awareness.
//!
//! Polls a few cheap system signals every few seconds (running processes,
//! visible window titles, which apps use the microphone), detects Zoom,
//! Microsoft Teams and Google Meet calls, and then either asks whether to
//! record (a small always-on-top popup), starts recording automatically, or
//! does nothing, depending on settings. When the call ends it asks to stop,
//! stops automatically, or does nothing.
//!
//! Detection rules live in `detect.rs` (pure, unit tested). Recording is
//! started and stopped through the same paths as the tray menu, so the
//! normal save and post-processing flow runs.

pub mod detect;
#[cfg(target_os = "macos")]
mod platform_macos;
#[cfg(target_os = "windows")]
mod platform_windows;

use detect::{detect, Detected, EnabledApps, Event, Signals, Tracker};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use sysinfo::{ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

const PROMPT_LABEL: &str = "meeting-prompt";
const POLL: Duration = Duration::from_secs(3);
/// Polls in a row before a meeting counts as started (about 6 s).
const START_AFTER: u32 = 2;
/// Polls in a row without the meeting before it counts as ended (about 21 s).
const END_AFTER: u32 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Show the popup and let the user decide.
    Ask,
    /// Do it without asking.
    Auto,
    /// Do nothing.
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub on_start: Action,
    pub on_end: Action,
    pub apps: EnabledApps,
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: true, on_start: Action::Ask, on_end: Action::Ask, apps: EnabledApps::default() }
    }
}

#[derive(Default)]
struct Shared {
    settings: Option<Settings>,
    /// What the popup is currently asking about.
    prompt: Option<(PromptKind, Detected)>,
    /// The meeting currently in progress, if any.
    active: Option<Detected>,
    /// The user said "not now" for this meeting; don't ask again until it ends.
    declined: bool,
}

static STATE: LazyLock<Mutex<Shared>> = LazyLock::new(|| Mutex::new(Shared::default()));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptKind {
    Start,
    Stop,
}

/// Payload of the `meeting-detected` / `meeting-ended` events.
#[derive(Debug, Clone, Serialize)]
pub struct MeetingEvent {
    pub app: detect::MeetingApp,
    pub app_label: &'static str,
    pub title: Option<String>,
}

impl From<&Detected> for MeetingEvent {
    fn from(d: &Detected) -> Self {
        Self { app: d.app, app_label: d.app.label(), title: d.title.clone() }
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

fn settings_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("meeting_watch.json"))
}

fn load_settings<R: Runtime>(app: &AppHandle<R>) -> Settings {
    if let Some(s) = STATE.lock().unwrap().settings.clone() {
        return s;
    }
    let s: Settings = settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    STATE.lock().unwrap().settings = Some(s.clone());
    s
}

#[tauri::command]
pub fn meeting_watch_get_settings<R: Runtime>(app: AppHandle<R>) -> Settings {
    load_settings(&app)
}

#[tauri::command]
pub fn meeting_watch_save_settings<R: Runtime>(app: AppHandle<R>, settings: Settings) -> Result<(), String> {
    let path = settings_path(&app).ok_or("No config folder")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    STATE.lock().unwrap().settings = Some(settings);
    Ok(())
}

/// The meeting in progress right now, if any (for the UI).
#[tauri::command]
pub fn meeting_watch_current() -> Option<MeetingEvent> {
    STATE.lock().unwrap().active.as_ref().map(MeetingEvent::from)
}

// ---------------------------------------------------------------------------
// Signals
// ---------------------------------------------------------------------------

fn collect(sys: &mut System, want_windows: bool) -> Signals {
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let names: HashMap<u32, String> = sys
        .processes()
        .iter()
        .map(|(pid, p)| (pid.as_u32(), p.name().to_string_lossy().into_owned()))
        .collect();
    let mut s = Signals { processes: names.values().cloned().collect(), ..Default::default() };

    #[cfg(target_os = "macos")]
    {
        if want_windows {
            s.windows = platform_macos::windows();
        }
        s.mic_users = platform_macos::mic_users();
    }
    #[cfg(target_os = "windows")]
    {
        if want_windows {
            s.windows = platform_windows::windows(&names);
        }
        s.mic_users = platform_windows::mic_users();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = (want_windows, &names);

    if let Some(users) = &s.mic_users {
        s.mic_active = Some(!users.is_empty());
    }
    s
}

// ---------------------------------------------------------------------------
// Popup
// ---------------------------------------------------------------------------

fn query_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{:02X}", b),
        })
        .collect()
}

fn show_prompt<R: Runtime>(app: &AppHandle<R>, kind: PromptKind, d: &Detected) {
    close_prompt(app);
    STATE.lock().unwrap().prompt = Some((kind, d.clone()));
    let url = format!(
        "meeting-prompt.html?kind={}&app={}&title={}",
        match kind { PromptKind::Start => "start", PromptKind::Stop => "stop" },
        query_escape(d.app.short_label()),
        query_escape(d.title.as_deref().unwrap_or("")),
    );
    // The card is drawn by the page (rounded corners + shadow), so the window
    // itself is transparent, undecorated and a little larger than the card.
    let (w, h) = (404.0, 140.0);
    let mut builder = WebviewWindowBuilder::new(app, PROMPT_LABEL, WebviewUrl::App(url.into()))
        .title("Motiv")
        .inner_size(w, h)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .transparent(true)
        .shadow(false)
        .visible_on_all_workspaces(true);
    if let Ok(Some(m)) = app.primary_monitor() {
        let scale = m.scale_factor();
        let size = m.size().to_logical::<f64>(scale);
        let pos = m.position().to_logical::<f64>(scale);
        builder = builder.position(pos.x + size.width - w - 12.0, pos.y + 36.0);
    }
    if let Err(e) = builder.build() {
        warn!("meeting watch: could not open popup: {}", e);
    }
}

fn close_prompt<R: Runtime>(app: &AppHandle<R>) {
    STATE.lock().unwrap().prompt = None;
    if let Some(w) = app.get_webview_window(PROMPT_LABEL) {
        let _ = w.close();
    }
}

fn recording_title(d: &Detected) -> String {
    let when = chrono::Local::now().format("%Y-%m-%d %H:%M");
    match &d.title {
        Some(t) => format!("{} ({})", t, when),
        None => format!("{} meeting {}", d.app.short_label(), when),
    }
}

/// Called by the popup's buttons: "record", "stop", "dismiss" or "never".
#[tauri::command]
pub async fn meeting_prompt_respond<R: Runtime>(app: AppHandle<R>, action: String) -> Result<(), String> {
    let prompt = STATE.lock().unwrap().prompt.take();
    close_prompt(&app);
    let Some((kind, d)) = prompt else { return Ok(()) };
    match (kind, action.as_str()) {
        (PromptKind::Start, "record") => {
            if !crate::audio::recording_commands::is_recording().await {
                crate::tray::request_start_recording(&app, Some(recording_title(&d)));
            }
        }
        (PromptKind::Stop, "stop") => {
            if crate::audio::recording_commands::is_recording().await {
                crate::tray::request_stop_recording(&app);
            }
        }
        (PromptKind::Start, "dismiss") => STATE.lock().unwrap().declined = true,
        (_, "never") => {
            let mut s = load_settings(&app);
            match kind {
                PromptKind::Start => s.on_start = Action::Off,
                PromptKind::Stop => s.on_end = Action::Off,
            }
            meeting_watch_save_settings(app.clone(), s)?;
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Loop
// ---------------------------------------------------------------------------

async fn on_event<R: Runtime>(app: &AppHandle<R>, ev: Event, settings: &Settings) {
    let recording = crate::audio::recording_commands::is_recording().await;
    match ev {
        Event::Started(mut d) => {
            // Prefer the calendar event's name when a matching event is on now.
            let platform = match d.app {
                detect::MeetingApp::Zoom => "zoom",
                detect::MeetingApp::Teams => "teams",
                detect::MeetingApp::Meet => "meet",
            };
            if let Some(event) = crate::calendar::current_event(app, Some(platform)).await {
                d.title = Some(event.title);
            }
            info!("meeting watch: {} call started", d.app.label());
            {
                let mut st = STATE.lock().unwrap();
                st.active = Some(d.clone());
                st.declined = false;
            }
            let _ = app.emit("meeting-detected", MeetingEvent::from(&d));
            if recording {
                return;
            }
            match settings.on_start {
                Action::Auto => crate::tray::request_start_recording(app, Some(recording_title(&d))),
                Action::Ask => show_prompt(app, PromptKind::Start, &d),
                Action::Off => {}
            }
        }
        Event::Ended(d) => {
            info!("meeting watch: {} call ended", d.app.label());
            {
                let mut st = STATE.lock().unwrap();
                st.active = None;
                st.declined = false;
            }
            let _ = app.emit("meeting-ended", MeetingEvent::from(&d));
            // A start prompt for a call that has ended is stale.
            if matches!(STATE.lock().unwrap().prompt, Some((PromptKind::Start, _))) {
                close_prompt(app);
            }
            if !recording {
                return;
            }
            match settings.on_end {
                Action::Auto => crate::tray::request_stop_recording(app),
                Action::Ask => show_prompt(app, PromptKind::Stop, &d),
                Action::Off => {}
            }
        }
    }
}

/// Starts the background watcher. Call once from app setup.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let mut sys = Some(System::new());
        let mut tracker = Tracker::new(START_AFTER, END_AFTER);
        loop {
            tokio::time::sleep(POLL).await;
            let settings = load_settings(&app);
            if !settings.enabled {
                if let Some(d) = tracker.active().cloned() {
                    tracker = Tracker::new(START_AFTER, END_AFTER);
                    STATE.lock().unwrap().active = None;
                    let _ = app.emit("meeting-ended", MeetingEvent::from(&d));
                }
                continue;
            }
            let mut s = sys.take().unwrap_or_else(System::new);
            let result = tauri::async_runtime::spawn_blocking(move || {
                let signals = collect(&mut s, true);
                (s, signals)
            })
            .await;
            let signals = match result {
                Ok((s, signals)) => {
                    sys = Some(s);
                    signals
                }
                Err(e) => {
                    warn!("meeting watch: poll failed: {}", e);
                    continue;
                }
            };
            if let Some(ev) = tracker.update(detect(&signals, settings.apps)) {
                on_event(&app, ev, &settings).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_and_partial_json() {
        let s: Settings = serde_json::from_str(r#"{"on_start":"auto"}"#).unwrap();
        assert!(s.enabled);
        assert_eq!(s.on_start, Action::Auto);
        assert_eq!(s.on_end, Action::Ask);
        assert!(s.apps.meet);
    }

    #[test]
    fn escapes_query_values() {
        assert_eq!(query_escape("Q4 Review & Plan"), "Q4%20Review%20%26%20Plan");
        assert_eq!(query_escape("café"), "caf%C3%A9");
    }

    #[test]
    fn recording_titles() {
        let t = recording_title(&Detected { app: detect::MeetingApp::Zoom, title: None });
        assert!(t.starts_with("Zoom meeting "));
        let t = recording_title(&Detected { app: detect::MeetingApp::Teams, title: None });
        assert!(t.starts_with("Teams meeting "));
        let t = recording_title(&Detected { app: detect::MeetingApp::Meet, title: Some("Roadmap".into()) });
        assert!(t.starts_with("Roadmap ("));
    }
}

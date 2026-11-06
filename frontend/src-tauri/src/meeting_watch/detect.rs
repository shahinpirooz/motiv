//! Pure meeting detection: turns a snapshot of system signals into "which
//! meeting is in progress", and a small state machine that debounces starts
//! and ends. No OS calls here, so it is fully unit tested.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeetingApp {
    Zoom,
    Teams,
    Meet,
}

impl MeetingApp {
    /// Short name for prompts and titles ("Teams meeting detected").
    pub fn short_label(self) -> &'static str {
        match self {
            MeetingApp::Zoom => "Zoom",
            MeetingApp::Teams => "Teams",
            MeetingApp::Meet => "Google Meet",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MeetingApp::Zoom => "Zoom",
            MeetingApp::Teams => "Microsoft Teams",
            MeetingApp::Meet => "Google Meet",
        }
    }
}

/// A visible top-level window.
#[derive(Debug, Clone, Default)]
pub struct WindowInfo {
    /// Owning application or process name (e.g. "zoom.us", "Google Chrome", "ms-teams.exe").
    pub owner: String,
    pub title: String,
}

/// Everything the detector looks at in one poll.
#[derive(Debug, Clone, Default)]
pub struct Signals {
    /// Running process names.
    pub processes: Vec<String>,
    /// Visible windows with titles (empty if the OS denies title access).
    pub windows: Vec<WindowInfo>,
    /// Apps currently capturing the microphone (names, bundle ids or exe
    /// paths). `None` when the OS can't say which app.
    pub mic_users: Option<Vec<String>>,
    /// Whether any app is using the default microphone. `None` if unknown.
    pub mic_active: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detected {
    pub app: MeetingApp,
    /// Meeting name taken from a window title, when there is one.
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct EnabledApps {
    pub zoom: bool,
    pub teams: bool,
    pub meet: bool,
}

impl Default for EnabledApps {
    fn default() -> Self {
        Self { zoom: true, teams: true, meet: true }
    }
}

const BROWSERS: &[&str] = &[
    "chrome", "msedge", "microsoft edge", "safari", "firefox", "arc", "brave", "opera", "vivaldi",
    "chromium", "com.google.chrome", "com.microsoft.edgemac", "com.apple.safari", "org.mozilla.firefox",
    "company.thebrowser.browser",
];

fn lc(s: &str) -> String {
    s.to_lowercase()
}

fn is_browser(name: &str) -> bool {
    let n = lc(name);
    BROWSERS.iter().any(|b| n.contains(b))
}

fn is_zoom(name: &str) -> bool {
    let n = lc(name);
    n.contains("zoom") && !n.contains("zoomit")
}

fn is_teams(name: &str) -> bool {
    let n = lc(name);
    n.contains("teams") || n.contains("com.microsoft.teams")
}

fn mic_used_by(s: &Signals, pred: impl Fn(&str) -> bool) -> Option<bool> {
    s.mic_users.as_ref().map(|users| users.iter().any(|u| pred(u)))
}

/// Mic evidence for an app: attributed use if the OS reports it, otherwise
/// "any app is using the mic", otherwise unknown (treated as true so the
/// window/process evidence decides on its own).
fn mic_evidence(s: &Signals, pred: impl Fn(&str) -> bool) -> bool {
    match mic_used_by(s, pred) {
        Some(v) => v,
        None => s.mic_active.unwrap_or(true),
    }
}

/// Strips app suffixes and separators from a window title.
fn clean_title(raw: &str, strip: &[&str]) -> Option<String> {
    let mut t = raw.trim().to_string();
    for s in strip {
        if let Some(i) = lc(&t).find(&lc(s)) {
            t.truncate(i);
        }
    }
    let t = t.trim().trim_end_matches(|c| c == '|' || c == '-' || c == '–' || c == '—').trim().to_string();
    (!t.is_empty()).then_some(t)
}

fn detect_zoom(s: &Signals) -> Option<Detected> {
    // CptHost is Zoom's screen/meeting host process; it only runs during a meeting.
    let in_meeting_process = s.processes.iter().any(|p| lc(p).starts_with("cpthost"));
    let meeting_window = s.windows.iter().find(|w| {
        is_zoom(&w.owner) && {
            let t = lc(&w.title);
            t.starts_with("zoom meeting") || t.starts_with("zoom webinar") || t.contains("meeting controls")
        }
    });
    let zoom_mic = mic_used_by(s, is_zoom) == Some(true);
    let browser_zoom = s.windows.iter().find(|w| is_browser(&w.owner) && lc(&w.title).contains("zoom") && lc(&w.title).contains("meeting"));

    if in_meeting_process || meeting_window.is_some() || zoom_mic {
        return Some(Detected { app: MeetingApp::Zoom, title: None });
    }
    if let Some(_w) = browser_zoom {
        if mic_evidence(s, is_browser) {
            return Some(Detected { app: MeetingApp::Zoom, title: None });
        }
    }
    None
}

fn detect_teams(s: &Signals) -> Option<Detected> {
    let teams_running = s.processes.iter().any(|p| is_teams(p));
    let teams_mic = mic_used_by(s, is_teams);

    // A Teams call/meeting window (desktop app) or Teams web in a browser.
    let call_window = s.windows.iter().find(|w| {
        let t = lc(&w.title);
        let looks_like_call = t.contains("meeting") || t.contains("call with") || t.contains("| call") || t.starts_with("call ");
        (is_teams(&w.owner) && looks_like_call)
            || (is_browser(&w.owner) && t.contains("microsoft teams") && looks_like_call)
    });

    let title = call_window.and_then(|w| clean_title(&w.title, &["| Microsoft Teams", "Microsoft Teams"]));
    let title = title.filter(|t| {
        let l = lc(t);
        !(l == "meeting" || l == "meetings" || l == "calendar" || l.starts_with("meeting compact"))
    });

    match (teams_mic, call_window) {
        // The OS says Teams has the mic: that's a call.
        (Some(true), _) if teams_running || call_window.is_some() => Some(Detected { app: MeetingApp::Teams, title }),
        // No per-app mic info: need a call window plus the mic in use.
        (None, Some(w)) if mic_evidence(s, |_| true) => {
            let _ = w;
            Some(Detected { app: MeetingApp::Teams, title })
        }
        // Teams web in a browser that has the mic.
        (Some(false), Some(w)) if is_browser(&w.owner) && mic_used_by(s, is_browser) == Some(true) => {
            Some(Detected { app: MeetingApp::Teams, title })
        }
        _ => None,
    }
}

fn detect_meet(s: &Signals) -> Option<Detected> {
    // In a call the tab title is "Meet - abc-defg-hij" or "Meet – <event name>".
    let w = s.windows.iter().find(|w| {
        if !is_browser(&w.owner) {
            return false;
        }
        let t = w.title.trim();
        t.starts_with("Meet - ") || t.starts_with("Meet – ") || t.starts_with("Meet — ") || lc(t).contains("meet.google.com/")
    })?;
    if !mic_evidence(s, is_browser) {
        return None;
    }
    let raw = w.title.trim();
    let title = raw
        .splitn(2, |c| c == '-' || c == '–' || c == '—')
        .nth(1)
        .and_then(|rest| clean_title(rest, &[" - Google Chrome", " - Microsoft Edge", " — Mozilla Firefox", " - Brave", " - Audio playing", " - Camera or microphone recording"]))
        .filter(|t| !is_meet_code(t));
    Some(Detected { app: MeetingApp::Meet, title })
}

/// True for bare Meet codes like "abc-defg-hij" (not a useful meeting name).
fn is_meet_code(t: &str) -> bool {
    let parts: Vec<&str> = t.split('-').collect();
    parts.len() == 3
        && [3usize, 4, 3].iter().zip(&parts).all(|(n, p)| p.len() == *n && p.chars().all(|c| c.is_ascii_lowercase()))
}

/// Returns the meeting in progress, if any. Order matters only when two
/// apps match at once; native apps win over browser tabs.
pub fn detect(s: &Signals, enabled: EnabledApps) -> Option<Detected> {
    (enabled.zoom.then(|| detect_zoom(s)).flatten())
        .or_else(|| enabled.teams.then(|| detect_teams(s)).flatten())
        .or_else(|| enabled.meet.then(|| detect_meet(s)).flatten())
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Started(Detected),
    Ended(Detected),
}

/// Debounces detection: a meeting must be seen `start_after` polls in a row
/// to start, and missing `end_after` polls in a row to end. Short gaps (a
/// window losing focus, a mic toggle) don't end the meeting.
#[derive(Debug)]
pub struct Tracker {
    start_after: u32,
    end_after: u32,
    seen: u32,
    missing: u32,
    pending: Option<Detected>,
    active: Option<Detected>,
}

impl Tracker {
    pub fn new(start_after: u32, end_after: u32) -> Self {
        Self { start_after: start_after.max(1), end_after: end_after.max(1), seen: 0, missing: 0, pending: None, active: None }
    }

    pub fn active(&self) -> Option<&Detected> {
        self.active.as_ref()
    }

    pub fn update(&mut self, now: Option<Detected>) -> Option<Event> {
        match (&mut self.active, now) {
            (Some(active), Some(d)) => {
                self.missing = 0;
                // Pick up a better title if one appears mid-meeting.
                if active.title.is_none() && d.app == active.app && d.title.is_some() {
                    active.title = d.title;
                }
                None
            }
            (Some(_), None) => {
                self.missing += 1;
                if self.missing >= self.end_after {
                    self.missing = 0;
                    self.seen = 0;
                    self.pending = None;
                    self.active.take().map(Event::Ended)
                } else {
                    None
                }
            }
            (None, Some(d)) => {
                if self.pending.as_ref().map(|p| p.app) == Some(d.app) {
                    self.seen += 1;
                    if d.title.is_some() {
                        self.pending = Some(d);
                    }
                } else {
                    self.pending = Some(d);
                    self.seen = 1;
                }
                if self.seen >= self.start_after {
                    self.seen = 0;
                    self.active = self.pending.take();
                    self.active.clone().map(Event::Started)
                } else {
                    None
                }
            }
            (None, None) => {
                self.seen = 0;
                self.pending = None;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(owner: &str, title: &str) -> WindowInfo {
        WindowInfo { owner: owner.into(), title: title.into() }
    }
    fn all() -> EnabledApps {
        EnabledApps::default()
    }

    #[test]
    fn zoom_via_cpthost() {
        let s = Signals { processes: vec!["zoom.us".into(), "CptHost".into()], ..Default::default() };
        assert_eq!(detect(&s, all()).unwrap().app, MeetingApp::Zoom);
        let idle = Signals { processes: vec!["zoom.us".into()], mic_users: Some(vec![]), ..Default::default() };
        assert_eq!(detect(&idle, all()), None);
    }

    #[test]
    fn zoom_via_window_or_mic() {
        let s = Signals { windows: vec![win("zoom.us", "Zoom Meeting")], ..Default::default() };
        assert_eq!(detect(&s, all()).unwrap().app, MeetingApp::Zoom);
        let s = Signals { mic_users: Some(vec!["C:\\Users\\a\\AppData\\Roaming\\Zoom\\bin\\Zoom.exe".into()]), ..Default::default() };
        assert_eq!(detect(&s, all()).unwrap().app, MeetingApp::Zoom);
    }

    #[test]
    fn teams_with_attributed_mic_and_title() {
        let s = Signals {
            processes: vec!["MSTeams".into()],
            windows: vec![win("Microsoft Teams", "Weekly Planning | Microsoft Teams")],
            mic_users: Some(vec!["com.microsoft.teams2".into()]),
            ..Default::default()
        };
        // Title only counts when it looks like a call; this one doesn't, but the mic does.
        assert_eq!(detect(&s, all()), Some(Detected { app: MeetingApp::Teams, title: None }));
        let s = Signals {
            processes: vec!["MSTeams".into()],
            windows: vec![win("Microsoft Teams", "Meeting with Priya | Microsoft Teams")],
            mic_users: Some(vec!["com.microsoft.teams2".into()]),
            ..Default::default()
        };
        assert_eq!(detect(&s, all()).unwrap().title.as_deref(), Some("Meeting with Priya"));
    }

    #[test]
    fn teams_running_without_call_is_ignored() {
        let s = Signals {
            processes: vec!["ms-teams.exe".into()],
            windows: vec![win("ms-teams.exe", "Chat | Microsoft Teams")],
            mic_users: Some(vec![]),
            mic_active: Some(false),
        };
        assert_eq!(detect(&s, all()), None);
    }

    #[test]
    fn teams_without_mic_attribution_needs_mic_active() {
        let mut s = Signals {
            windows: vec![win("Microsoft Teams", "Call with Sam | Microsoft Teams")],
            mic_active: Some(false),
            ..Default::default()
        };
        assert_eq!(detect(&s, all()), None);
        s.mic_active = Some(true);
        assert_eq!(detect(&s, all()).unwrap().title.as_deref(), Some("Call with Sam"));
    }

    #[test]
    fn meet_in_browser() {
        let s = Signals {
            windows: vec![win("Google Chrome", "Meet - abc-defg-hij - Google Chrome")],
            mic_users: Some(vec!["com.google.Chrome".into()]),
            ..Default::default()
        };
        assert_eq!(detect(&s, all()), Some(Detected { app: MeetingApp::Meet, title: None }));
        let s = Signals {
            windows: vec![win("msedge.exe", "Meet – Q4 Roadmap Review - Microsoft Edge")],
            mic_active: Some(true),
            ..Default::default()
        };
        assert_eq!(detect(&s, all()).unwrap().title.as_deref(), Some("Q4 Roadmap Review"));
    }

    #[test]
    fn meet_landing_page_or_no_mic_is_ignored() {
        let s = Signals { windows: vec![win("Google Chrome", "Google Meet")], mic_active: Some(true), ..Default::default() };
        assert_eq!(detect(&s, all()), None);
        let s = Signals {
            windows: vec![win("Google Chrome", "Meet - abc-defg-hij")],
            mic_users: Some(vec!["zoom.us".into()]),
            ..Default::default()
        };
        // Mic is held by Zoom, not the browser: that's a Zoom call.
        assert_eq!(detect(&s, all()).unwrap().app, MeetingApp::Zoom);
    }

    #[test]
    fn disabled_apps_are_skipped() {
        let s = Signals { processes: vec!["CptHost".into()], ..Default::default() };
        assert_eq!(detect(&s, EnabledApps { zoom: false, teams: true, meet: true }), None);
    }

    #[test]
    fn non_meeting_apps_with_meet_in_title_are_ignored() {
        let s = Signals { windows: vec![win("Notes", "Meet - agenda")], mic_active: Some(true), ..Default::default() };
        assert_eq!(detect(&s, all()), None);
    }

    #[test]
    fn tracker_debounces_start_and_end() {
        let z = || Some(Detected { app: MeetingApp::Zoom, title: None });
        let mut t = Tracker::new(2, 3);
        assert_eq!(t.update(z()), None);
        assert_eq!(t.update(None), None); // blip resets
        assert_eq!(t.update(z()), None);
        assert!(matches!(t.update(z()), Some(Event::Started(_))));
        assert_eq!(t.update(None), None);
        assert_eq!(t.update(None), None);
        assert_eq!(t.update(z()), None); // came back within grace
        assert_eq!(t.update(None), None);
        assert_eq!(t.update(None), None);
        assert!(matches!(t.update(None), Some(Event::Ended(_))));
        assert!(t.active().is_none());
    }

    #[test]
    fn tracker_picks_up_title_later() {
        let mut t = Tracker::new(1, 1);
        t.update(Some(Detected { app: MeetingApp::Meet, title: None }));
        t.update(Some(Detected { app: MeetingApp::Meet, title: Some("Roadmap".into()) }));
        assert_eq!(t.active().unwrap().title.as_deref(), Some("Roadmap"));
    }
}

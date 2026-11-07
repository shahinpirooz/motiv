//! Motiv: update check against the latest GitHub release of the repo.
//!
//! Motiv's builds aren't signed for Tauri's auto-updater, so this only tells
//! the user a newer version exists and links to its release page.

use serde::{Deserialize, Serialize};
use std::time::Duration;

const LATEST_RELEASE: &str = "https://api.github.com/repos/shahinpirooz/motiv/releases/latest";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LatestRelease {
    pub version: String,
    pub url: String,
    pub notes: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// "v1.2.3" / "1.2.3" -> (1, 2, 3). Ignores anything after a '-' or '+'.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let core = s.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0));
    Some(v)
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

pub fn parse_release(body: &str) -> Option<LatestRelease> {
    let r: GitHubRelease = serde_json::from_str(body).ok()?;
    if r.draft || r.prerelease {
        return None;
    }
    Some(LatestRelease {
        version: r.tag_name.trim_start_matches(['v', 'V']).to_string(),
        url: r.html_url,
        notes: r.body.filter(|b| !b.trim().is_empty()),
        published_at: r.published_at,
    })
}

/// The latest published release, or None if there isn't one yet.
#[tauri::command]
pub async fn motiv_latest_release() -> Result<Option<LatestRelease>, String> {
    let resp = reqwest::Client::new()
        .get(LATEST_RELEASE)
        .header("User-Agent", "Motiv-update-check")
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Could not reach GitHub: {}", e))?;
    if resp.status().as_u16() == 404 {
        return Ok(None); // no releases published
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub returned {}", resp.status()));
    }
    let body = resp.text().await.map_err(|e| e.to_string())?;
    Ok(parse_release(&body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.7.0"), Some((0, 7, 0)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("nightly"), None);
        assert!(is_newer("v0.8.0", "0.7.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("v0.7.0", "0.7.0"));
        assert!(!is_newer("v0.6.0", "0.7.0"));
        assert!(!is_newer("garbage", "0.7.0"));
    }

    #[test]
    fn releases() {
        let r = parse_release(r#"{"tag_name":"v0.8.0","html_url":"https://github.com/x/releases/tag/v0.8.0","body":"- Faster","published_at":"2026-12-01T00:00:00Z","draft":false,"prerelease":false}"#).unwrap();
        assert_eq!((r.version.as_str(), r.notes.as_deref()), ("0.8.0", Some("- Faster")));
        assert!(parse_release(r#"{"tag_name":"v0.9.0","html_url":"u","prerelease":true}"#).is_none());
        assert!(parse_release("not json").is_none());
    }
}

//! Fork addition: opens the system email client for the "Email" summary button.
//!
//! * `rich_export_open_eml`: saves an unsent .eml draft (built by the frontend,
//!   HTML + plain text) to the temp folder and opens it with the default mail
//!   app. Classic Outlook for Windows and Thunderbird open it as a formatted,
//!   editable draft.
//! * `rich_export_open_mailto`: opens a `mailto:` link (new message with the
//!   subject filled in; the formatted body is pasted from the clipboard).
//!
//! On Windows both go through `rundll32 url.dll,FileProtocolHandler` rather
//! than `cmd /C start`: cmd treats `&` in a mailto query as a command
//! separator and would cut the link (and could run the rest as a command).

use std::path::{Path, PathBuf};
use std::process::Command;

const DRAFT_DIR: &str = "motiv-email-drafts";
const MAX_MAILTO_LEN: usize = 8000;
const MAX_EML_BYTES: usize = 20 * 1024 * 1024;

/// Keeps only a plain file name ending in .eml, with no path parts or
/// characters Windows/macOS reject.
pub fn sanitize_file_name(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut clean: String = base
        .chars()
        .map(|c| if c.is_control() || "\\/:*?\"<>|".contains(c) { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if clean.to_lowercase().ends_with(".eml") {
        clean.truncate(clean.len() - 4);
    }
    let clean = clean.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let clean: String = clean.chars().take(80).collect();
    format!("{}.eml", if clean.is_empty() { "Meeting summary" } else { &clean })
}

pub fn validate_mailto(url: &str) -> Result<(), String> {
    if !url.to_ascii_lowercase().starts_with("mailto:") {
        return Err("Only mailto: links can be opened".into());
    }
    if url.len() > MAX_MAILTO_LEN {
        return Err("The email link is too long".into());
    }
    if url.chars().any(|c| c.is_control() || c == ' ' || c == '"') {
        return Err("The email link must be URL-encoded".into());
    }
    Ok(())
}

/// Program and arguments that open `target` (a file path or URL) with the
/// user's default handler.
pub fn launcher(target: &str) -> (String, Vec<String>) {
    if cfg!(target_os = "windows") {
        (
            "rundll32".into(),
            vec!["url.dll,FileProtocolHandler".into(), target.into()],
        )
    } else if cfg!(target_os = "macos") {
        ("open".into(), vec![target.into()])
    } else {
        ("xdg-open".into(), vec![target.into()])
    }
}

fn open_with_default_app(target: &str) -> Result<(), String> {
    let (program, args) = launcher(target);
    Command::new(&program)
        .args(&args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open the email app ({program}): {e}"))
}

pub fn write_draft(dir: &Path, eml: &str, file_name: &str) -> Result<PathBuf, String> {
    if eml.len() > MAX_EML_BYTES {
        return Err("The email is too large".into());
    }
    if !eml.contains("X-Unsent: 1") {
        return Err("Not an email draft".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not create the drafts folder: {e}"))?;
    let path = dir.join(sanitize_file_name(file_name));
    std::fs::write(&path, eml.as_bytes()).map_err(|e| format!("Could not save the draft: {e}"))?;
    Ok(path)
}

#[tauri::command]
pub async fn rich_export_open_eml(eml: String, file_name: String) -> Result<String, String> {
    let dir = std::env::temp_dir().join(DRAFT_DIR);
    let path = write_draft(&dir, &eml, &file_name)?;
    let target = path.to_string_lossy().to_string();
    open_with_default_app(&target)?;
    Ok(target)
}

#[tauri::command]
pub async fn rich_export_open_mailto(url: String) -> Result<(), String> {
    validate_mailto(&url)?;
    open_with_default_app(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(sanitize_file_name("Weekly sync.eml"), "Weekly sync.eml");
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd.eml");
        assert_eq!(sanitize_file_name("a/b\\c:d*e?.eml"), "b c d e.eml");
        assert_eq!(sanitize_file_name(""), "Meeting summary.eml");
        assert_eq!(sanitize_file_name("..."), "Meeting summary.eml");
        assert_eq!(sanitize_file_name(&"x".repeat(200)).len(), 84);
    }

    #[test]
    fn mailto_validation() {
        assert!(validate_mailto("mailto:?subject=Hi%20there&body=x").is_ok());
        assert!(validate_mailto("MAILTO:a@b.com").is_ok());
        assert!(validate_mailto("https://evil.example").is_err());
        assert!(validate_mailto("mailto:?subject=a b").is_err());
        assert!(validate_mailto("mailto:\"&calc").is_err());
        assert!(validate_mailto(&format!("mailto:?body={}", "a".repeat(9000))).is_err());
    }

    #[test]
    fn launcher_never_uses_a_shell() {
        let (program, args) = launcher("mailto:?subject=a&body=b");
        assert_ne!(program, "cmd");
        assert_ne!(program, "sh");
        assert_eq!(args.last().unwrap(), "mailto:?subject=a&body=b");
    }

    #[test]
    fn drafts_are_written() {
        let dir = std::env::temp_dir().join(format!("meetily-test-{}", std::process::id()));
        let eml = "MIME-Version: 1.0\r\nX-Unsent: 1\r\n\r\nbody";
        let path = write_draft(&dir, eml, "My: meeting").unwrap();
        assert_eq!(path.file_name().unwrap(), "My meeting.eml");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), eml);
        assert!(write_draft(&dir, "not a draft", "x").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}

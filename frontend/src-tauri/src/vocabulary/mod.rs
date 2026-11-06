//! Motiv: names and terms with their correct spelling.
//!
//! Speech recognition often misspells names ("Shaheen Pirus"). The list is
//! added to every LLM prompt (summaries, chat, action items, translation) so
//! the model writes them correctly even when the transcript doesn't, and is
//! given to Whisper as its initial prompt, which biases recognition toward
//! these spellings. (Parakeet has no prompt input.)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};
use tauri::{AppHandle, Manager, Runtime};

static TERMS: LazyLock<RwLock<Vec<String>>> = LazyLock::new(|| RwLock::new(Vec::new()));

/// Whisper's prompt window is ~224 tokens; stay well under it.
const WHISPER_PROMPT_MAX_CHARS: usize = 600;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Vocabulary {
    pub terms: Vec<String>,
}

/// Trims, drops empties, removes duplicates (case-insensitive), strips
/// characters that would break prompts.
pub fn normalize(terms: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in terms {
        let t: String = t.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
        if t.is_empty() || out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            continue;
        }
        out.push(t);
    }
    out
}

pub fn set_terms(terms: Vec<String>) {
    *TERMS.write().unwrap() = normalize(&terms);
}

pub fn terms() -> Vec<String> {
    TERMS.read().unwrap().clone()
}

/// The sentence appended to LLM system prompts, if there are terms.
pub fn llm_note(terms: &[String]) -> Option<String> {
    (!terms.is_empty()).then(|| {
        format!(
            "Correct spellings of names and terms that may appear in this meeting (speech recognition may have \
             misspelled or split them; when a word in the source sounds like one of these, write it exactly like this): {}.",
            terms.join("; ")
        )
    })
}

/// System prompt with the vocabulary note appended.
pub fn with_vocabulary(system_prompt: &str) -> String {
    match llm_note(&terms()) {
        Some(note) => format!("{}\n\n{}", system_prompt.trim_end(), note),
        None => system_prompt.to_string(),
    }
}

/// Initial prompt for Whisper: the terms as a short comma-separated list.
pub fn whisper_prompt(terms: &[String]) -> Option<String> {
    let mut s = String::new();
    for t in terms {
        let next = if s.is_empty() { t.clone() } else { format!(", {}", t) };
        if s.len() + next.len() > WHISPER_PROMPT_MAX_CHARS {
            break;
        }
        s.push_str(&next);
    }
    (!s.is_empty()).then(|| format!("{}.", s))
}

fn path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("vocabulary.json"))
}

/// Loads the saved list into memory. Call once at startup.
pub fn load<R: Runtime>(app: &AppHandle<R>) {
    let v: Vocabulary = path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    set_terms(v.terms);
}

#[tauri::command]
pub fn vocabulary_get() -> Vocabulary {
    Vocabulary { terms: terms() }
}

#[tauri::command]
pub fn vocabulary_save<R: Runtime>(app: AppHandle<R>, terms: Vec<String>) -> Result<Vocabulary, String> {
    let v = Vocabulary { terms: normalize(&terms) };
    let p = path(&app).ok_or("No config folder")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    set_terms(v.terms.clone());
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize(&s(&["  Shahin Pirooz ", "", "whitedog", "WhiteDog", "a\u{0}b"])), s(&["Shahin Pirooz", "whitedog", "ab"]));
    }

    #[test]
    fn llm_note_lists_terms() {
        assert_eq!(llm_note(&[]), None);
        let n = llm_note(&s(&["Shahin Pirooz", "WhiteDog"])).unwrap();
        assert!(n.contains("Shahin Pirooz; WhiteDog."));
    }

    #[test]
    fn whisper_prompt_is_capped() {
        assert_eq!(whisper_prompt(&s(&["Shahin Pirooz", "WhiteDog"])).as_deref(), Some("Shahin Pirooz, WhiteDog."));
        let many: Vec<String> = (0..200).map(|i| format!("Term{}", i)).collect();
        assert!(whisper_prompt(&many).unwrap().len() <= WHISPER_PROMPT_MAX_CHARS + 1);
        assert_eq!(whisper_prompt(&[]), None);
    }
}

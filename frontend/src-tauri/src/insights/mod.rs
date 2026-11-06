//! Motiv: action items, decisions, highlights and open questions.
//!
//! Asks the configured summary model for a JSON document describing the
//! meeting, validates it, and stores it in `meeting_insights`. Long meetings
//! that don't fit the model's context are processed in parts and merged.
//! Action items can be ticked off; the done state is kept in the stored JSON.

use crate::chat::{context_budget, load_meeting, load_provider_settings, transcript_lines};
use crate::state::AppState;
use crate::summary::processor::rough_token_count;
use chrono::Utc;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Runtime};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ActionItem {
    pub task: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub text: String,
    #[serde(default)]
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Insights {
    #[serde(default)]
    pub action_items: Vec<ActionItem>,
    #[serde(default)]
    pub decisions: Vec<Point>,
    #[serde(default)]
    pub highlights: Vec<Point>,
    #[serde(default)]
    pub open_questions: Vec<Point>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredInsights {
    pub meeting_id: String,
    pub insights: Insights,
    pub model: String,
    pub updated_at: String,
}

pub const SYSTEM_PROMPT: &str = r#"You extract structured notes from a meeting transcript.
Each transcript line starts with a timestamp in square brackets.
Reply with ONLY a JSON object, no prose and no code fences, in exactly this shape:
{"action_items":[{"task":"...","owner":"name or null","due":"due date as said, or null","timestamp":"12:34"}],
 "decisions":[{"text":"...","timestamp":"12:34"}],
 "highlights":[{"text":"...","timestamp":"12:34"}],
 "open_questions":[{"text":"...","timestamp":"12:34"}]}
Rules:
- action_items: concrete tasks someone agreed or was asked to do. Start each task with a verb. Owner only if a person is named or clearly implied, else null.
- decisions: things the group agreed or decided.
- highlights: the 3 to 7 most important points or facts discussed.
- open_questions: questions raised but not answered.
- timestamp: the bracketed time of the line the item comes from, without brackets.
- Use only what is in the transcript. Use [] for anything with no items. Keep each item to one sentence."#;

// ---------------------------------------------------------------------------
// Pure helpers (unit tested)
// ---------------------------------------------------------------------------

/// Extracts the JSON object from a model reply (tolerates code fences and
/// text around it) and parses it. Blank items are dropped and "null"/"none"
/// strings become None.
pub fn parse_insights(reply: &str) -> Result<Insights, String> {
    let start = reply.find('{').ok_or("The model did not return JSON.")?;
    let end = reply.rfind('}').ok_or("The model did not return JSON.")?;
    if end < start {
        return Err("The model did not return JSON.".into());
    }
    let mut v: Insights = serde_json::from_str(&reply[start..=end])
        .map_err(|e| format!("The model returned invalid JSON: {}", e))?;

    fn clean(o: Option<String>) -> Option<String> {
        o.map(|s| s.trim().trim_matches(|c| c == '[' || c == ']').trim().to_string())
            .filter(|s| !s.is_empty() && !matches!(s.to_lowercase().as_str(), "null" | "none" | "n/a" | "unknown"))
    }
    v.action_items.retain(|a| !a.task.trim().is_empty());
    for a in &mut v.action_items {
        a.task = a.task.trim().to_string();
        a.owner = clean(a.owner.take());
        a.due = clean(a.due.take());
        a.timestamp = clean(a.timestamp.take());
        a.done = false;
    }
    for list in [&mut v.decisions, &mut v.highlights, &mut v.open_questions] {
        list.retain(|p| !p.text.trim().is_empty());
        for p in list.iter_mut() {
            p.text = p.text.trim().to_string();
            p.timestamp = clean(p.timestamp.take());
        }
    }
    Ok(v)
}

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Merges results from several transcript parts, dropping exact duplicates
/// (ignoring case and punctuation). Highlights are capped at 10.
pub fn merge(parts: Vec<Insights>) -> Insights {
    let mut out = Insights::default();
    for p in parts {
        for a in p.action_items {
            if !out.action_items.iter().any(|x| norm(&x.task) == norm(&a.task)) {
                out.action_items.push(a);
            }
        }
        for (dst, src) in [
            (&mut out.decisions, p.decisions),
            (&mut out.highlights, p.highlights),
            (&mut out.open_questions, p.open_questions),
        ] {
            for item in src {
                if !dst.iter().any(|x| norm(&x.text) == norm(&item.text)) {
                    dst.push(item);
                }
            }
        }
    }
    out.highlights.truncate(10);
    out
}

/// Splits transcript lines into parts that each fit `budget_tokens`.
pub fn split_lines(lines: &[String], budget_tokens: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut used = 0;
    for line in lines {
        let cost = rough_token_count(line) + 1;
        if used + cost > budget_tokens && !current.is_empty() {
            parts.push(current.join("\n"));
            current.clear();
            used = 0;
        }
        current.push(line);
        used += cost;
    }
    if !current.is_empty() {
        parts.push(current.join("\n"));
    }
    parts
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

async fn load(pool: &SqlitePool, meeting_id: &str) -> Result<Option<StoredInsights>, String> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT insights, model, updated_at FROM meeting_insights WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("Failed to load insights: {}", e))?;
    match row {
        None => Ok(None),
        Some((json, model, updated_at)) => {
            let insights = serde_json::from_str(&json).map_err(|e| format!("Stored insights are corrupt: {}", e))?;
            Ok(Some(StoredInsights { meeting_id: meeting_id.to_string(), insights, model, updated_at }))
        }
    }
}

async fn store(pool: &SqlitePool, meeting_id: &str, insights: &Insights, model: &str) -> Result<StoredInsights, String> {
    let now = Utc::now().to_rfc3339();
    let json = serde_json::to_string(insights).map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO meeting_insights (meeting_id, insights, model, created_at, updated_at) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(meeting_id) DO UPDATE SET insights = excluded.insights, model = excluded.model, updated_at = excluded.updated_at",
    )
    .bind(meeting_id)
    .bind(&json)
    .bind(model)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to save insights: {}", e))?;
    Ok(StoredInsights { meeting_id: meeting_id.to_string(), insights: insights.clone(), model: model.to_string(), updated_at: now })
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn api_get_meeting_insights(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Option<StoredInsights>, String> {
    load(state.db_manager.pool(), &meeting_id).await
}

/// Generates (or regenerates) insights for a meeting and stores them.
/// Done marks on action items whose text is unchanged are kept.
#[tauri::command]
pub async fn api_generate_meeting_insights<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    model: String,
    model_name: String,
) -> Result<StoredInsights, String> {
    let pool = state.db_manager.pool().clone();
    let (title, segments) = load_meeting(&pool, &meeting_id).await?;
    let lines = transcript_lines(&segments);
    if lines.is_empty() {
        return Err("This meeting has no transcript yet.".into());
    }

    let settings = load_provider_settings(&pool, &model).await?;
    let budget = context_budget(&settings, &model_name).await;
    let parts = split_lines(&lines, budget);
    let participants = crate::calendar::participants_for_prompt(&pool, &meeting_id).await;
    info!("insights: {} transcript lines in {} part(s) for {}", lines.len(), parts.len(), model_name);

    let mut results = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let user = format!(
            "Meeting: {}{}{}\n\n<transcript>\n{}\n</transcript>",
            title,
            participants.as_deref().map(|n| format!("\n{}", n)).unwrap_or_default(),
            if parts.len() > 1 { format!(" (part {} of {})", i + 1, parts.len()) } else { String::new() },
            part
        );
        let reply = settings.complete(&app, &model_name, SYSTEM_PROMPT, &user).await?;
        match parse_insights(&reply) {
            Ok(v) => results.push(v),
            Err(e) if parts.len() > 1 => warn!("insights: part {} skipped: {}", i + 1, e),
            Err(e) => return Err(e),
        }
    }
    if results.is_empty() {
        return Err("The model did not return usable JSON for any part of the meeting.".into());
    }
    let mut merged = merge(results);

    if let Some(prev) = load(&pool, &meeting_id).await? {
        for a in &mut merged.action_items {
            a.done = prev
                .insights
                .action_items
                .iter()
                .any(|p| p.done && norm(&p.task) == norm(&a.task));
        }
    }
    store(&pool, &meeting_id, &merged, &model_name).await
}

/// Marks one action item (by position) done or not done.
#[tauri::command]
pub async fn api_set_action_item_done(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    index: usize,
    done: bool,
) -> Result<StoredInsights, String> {
    let pool = state.db_manager.pool();
    let mut stored = load(pool, &meeting_id).await?.ok_or("No action items for this meeting.")?;
    let item = stored
        .insights
        .action_items
        .get_mut(index)
        .ok_or("Action item not found.")?;
    item.done = done;
    store(pool, &meeting_id, &stored.insights, &stored.model).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_json_and_cleans_fields() {
        let reply = "Here you go:\n```json\n{\"action_items\":[{\"task\":\" Send the deck \",\"owner\":\"null\",\"due\":\"Friday\",\"timestamp\":\"[12:34]\"},{\"task\":\"  \"}],\"decisions\":[{\"text\":\"Ship v2\",\"timestamp\":null}],\"highlights\":[],\"open_questions\":[{\"text\":\"\"}]}\n```";
        let v = parse_insights(reply).unwrap();
        assert_eq!(v.action_items.len(), 1);
        let a = &v.action_items[0];
        assert_eq!(a.task, "Send the deck");
        assert_eq!(a.owner, None);
        assert_eq!(a.due.as_deref(), Some("Friday"));
        assert_eq!(a.timestamp.as_deref(), Some("12:34"));
        assert_eq!(v.decisions, vec![Point { text: "Ship v2".into(), timestamp: None }]);
        assert!(v.open_questions.is_empty());
    }

    #[test]
    fn missing_lists_default_to_empty() {
        let v = parse_insights("{\"highlights\":[{\"text\":\"Budget approved\"}]}").unwrap();
        assert!(v.action_items.is_empty());
        assert_eq!(v.highlights.len(), 1);
    }

    #[test]
    fn rejects_non_json() {
        assert!(parse_insights("Sorry, I can't help.").is_err());
        assert!(parse_insights("{not json}").is_err());
    }

    #[test]
    fn merge_dedupes_across_parts() {
        let a = Insights {
            action_items: vec![ActionItem { task: "Send the deck.".into(), ..Default::default() }],
            decisions: vec![Point { text: "Ship v2".into(), timestamp: None }],
            ..Default::default()
        };
        let b = Insights {
            action_items: vec![
                ActionItem { task: "send the deck".into(), ..Default::default() },
                ActionItem { task: "Book room".into(), ..Default::default() },
            ],
            decisions: vec![Point { text: "Ship V2!".into(), timestamp: None }],
            ..Default::default()
        };
        let m = merge(vec![a, b]);
        assert_eq!(m.action_items.len(), 2);
        assert_eq!(m.decisions.len(), 1);
    }

    #[test]
    fn splits_lines_under_budget() {
        let lines: Vec<String> = (0..100).map(|i| format!("[0:{:02}] some words here {}", i, i)).collect();
        let parts = split_lines(&lines, 100);
        assert!(parts.len() > 1);
        assert_eq!(parts.join("\n"), lines.join("\n"));
        assert!(parts.iter().all(|p| rough_token_count(p) <= 110));
    }
}

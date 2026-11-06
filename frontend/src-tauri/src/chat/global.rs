//! Motiv: chat across all meetings.
//!
//! Finds the transcript passages most related to the question with the FTS5
//! index (any of the question's keywords), adds a few lines of context around
//! each hit, groups them by meeting, and always includes a short catalog of
//! recent meetings so date questions ("what did we discuss on Tuesday?") work.
//! When keyword hits are thin, the most recent meetings fill the remaining
//! context. Conversations are stored in `assistant_chat_messages`.

use super::{context_budget, keywords, load_provider_settings, resolve_model, transcript_lines, Segment, MAX_HISTORY_TURNS};
use crate::state::AppState;
use crate::summary::processor::rough_token_count;
use chrono::Utc;
use log::info;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::{BTreeSet, HashMap};
use tauri::{AppHandle, Runtime};

/// Lines of transcript kept before and after each search hit.
const CONTEXT_LINES: usize = 3;
/// Search hits considered.
const MAX_HITS: i64 = 60;
/// Meetings listed in the catalog.
const CATALOG_SIZE: i64 = 40;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Source {
    pub meeting_id: String,
    pub title: String,
    pub date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub sources: Vec<Source>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct MeetingInfo {
    pub id: String,
    pub title: String,
    pub date: String,
}

pub const SYSTEM_PROMPT: &str = "You are Motiv, an assistant that answers questions about the user's recorded meetings. \
You are given a catalog of recent meetings and excerpts from meeting transcripts; each excerpt line starts with a timestamp in square brackets. \
Rules: \
1. Use only the catalog and excerpts. If they don't contain the answer, say so and suggest what to search for or which meeting to open. \
2. When you state a fact, name the meeting it comes from and the timestamp, like (Weekly Planning, 12:34). \
3. Be concise. Use short markdown lists for several items. \
4. Do not invent names, numbers, dates or decisions. \
5. Excerpts are partial; say so when that may limit the answer.";

// ---------------------------------------------------------------------------
// Pure helpers (unit tested)
// ---------------------------------------------------------------------------

/// FTS5 query matching ANY of the question's keywords (prefix match).
pub fn any_terms_query(question: &str) -> Option<String> {
    let mut terms: Vec<String> = keywords(question).into_iter().collect();
    terms.sort();
    if terms.is_empty() {
        return None;
    }
    Some(terms.iter().map(|t| format!("\"{}\"*", t.replace('"', ""))).collect::<Vec<_>>().join(" OR "))
}

/// Line indexes to keep for a meeting: each hit plus `CONTEXT_LINES` around it.
pub fn windows_around(hits: &[usize], len: usize) -> Vec<usize> {
    let mut keep = BTreeSet::new();
    for &h in hits {
        let a = h.saturating_sub(CONTEXT_LINES);
        let b = (h + CONTEXT_LINES + 1).min(len);
        keep.extend(a..b);
    }
    keep.into_iter().collect()
}

/// Renders kept lines with "..." between gaps.
pub fn render_lines(lines: &[String], keep: &[usize]) -> String {
    let mut out = String::new();
    let mut prev: Option<usize> = None;
    for &i in keep {
        if let Some(p) = prev {
            out.push_str(if i == p + 1 { "\n" } else { "\n...\n" });
        } else if i > 0 {
            out.push_str("...\n");
        }
        out.push_str(&lines[i]);
        prev = Some(i);
    }
    if prev.map_or(false, |p| p + 1 < lines.len()) {
        out.push_str("\n...");
    }
    out
}

pub fn catalog(meetings: &[MeetingInfo]) -> String {
    meetings.iter().map(|m| format!("- {} | {}", m.date, m.title)).collect::<Vec<_>>().join("\n")
}

pub fn build_user_prompt(catalog: &str, excerpts: &str, history: &[AssistantMessage], question: &str) -> String {
    let mut p = format!("Today is {}.\n\n<recent_meetings>\n{}\n</recent_meetings>\n\n<excerpts>\n{}\n</excerpts>\n",
        chrono::Local::now().format("%A %Y-%m-%d"), catalog, excerpts);
    let recent: Vec<&AssistantMessage> = history.iter().rev().take(MAX_HISTORY_TURNS).collect::<Vec<_>>().into_iter().rev().collect();
    if !recent.is_empty() {
        p.push_str("\n<conversation_so_far>\n");
        for m in recent {
            let who = if m.role == "assistant" { "Assistant" } else { "User" };
            p.push_str(&format!("{}: {}\n", who, m.content.trim()));
        }
        p.push_str("</conversation_so_far>\n");
    }
    p.push_str(&format!("\nQuestion: {}\n", question.trim()));
    p
}

/// Sources the answer actually mentions by title; all included meetings if none match.
pub fn cited_sources(answer: &str, included: &[Source]) -> Vec<Source> {
    let a = answer.to_lowercase();
    let cited: Vec<Source> = included.iter().filter(|s| a.contains(&s.title.to_lowercase())).cloned().collect();
    if cited.is_empty() { included.to_vec() } else { cited }
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

fn date_of(created_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|d| d.with_timezone(&chrono::Local).format("%a %Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| created_at.chars().take(16).collect())
}

async fn recent_meetings(pool: &SqlitePool, limit: i64) -> Result<Vec<MeetingInfo>, String> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, title, created_at FROM meetings ORDER BY created_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to list meetings: {}", e))?;
    Ok(rows.into_iter().map(|(id, title, c)| MeetingInfo { id, title, date: date_of(&c) }).collect())
}

async fn meeting_segments(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<(String, Segment)>, String> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<f64>)>(
        "SELECT id, transcript, timestamp, audio_start_time FROM transcripts
         WHERE meeting_id = ? ORDER BY COALESCE(audio_start_time, 0), timestamp, rowid",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to load transcript: {}", e))?;
    Ok(rows
        .into_iter()
        .filter(|(_, t, _, _)| !t.trim().is_empty())
        .map(|(id, text, timestamp, audio_start_time)| (id, Segment { text, timestamp, audio_start_time }))
        .collect())
}

/// Builds the excerpt block and the list of meetings it draws on.
async fn gather(pool: &SqlitePool, question: &str, recent: &[MeetingInfo], budget: usize) -> Result<(String, Vec<Source>), String> {
    // 1. Search hits, best first, grouped by meeting in rank order.
    let mut order: Vec<String> = Vec::new();
    let mut hit_ids: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(q) = any_terms_query(question) {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT meeting_id, transcript_id FROM transcripts_fts WHERE transcripts_fts MATCH ? ORDER BY bm25(transcripts_fts) LIMIT ?",
        )
        .bind(&q)
        .bind(MAX_HITS)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for (m, t) in rows {
            if !hit_ids.contains_key(&m) {
                order.push(m.clone());
            }
            hit_ids.entry(m).or_default().push(t);
        }
    }
    // 2. Then recent meetings without hits, to fill remaining room.
    for m in recent.iter().take(5) {
        if !hit_ids.contains_key(&m.id) {
            order.push(m.id.clone());
        }
    }

    let info: HashMap<String, MeetingInfo> = {
        let ids: Vec<&String> = order.iter().collect();
        let mut map = HashMap::new();
        for id in ids {
            if let Some(m) = recent.iter().find(|m| &m.id == id) {
                map.insert(id.clone(), m.clone());
            } else if let Ok(Some((title, c))) = sqlx::query_as::<_, (String, String)>("SELECT title, created_at FROM meetings WHERE id = ?")
                .bind(id).fetch_optional(pool).await
            {
                map.insert(id.clone(), MeetingInfo { id: id.clone(), title, date: date_of(&c) });
            }
        }
        map
    };

    let mut out = String::new();
    let mut used = 0usize;
    let mut sources = Vec::new();
    for id in order {
        let Some(meta) = info.get(&id) else { continue };
        let segs = meeting_segments(pool, &id).await?;
        if segs.is_empty() {
            continue;
        }
        let lines = transcript_lines(&segs.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>());
        let body = match hit_ids.get(&id) {
            Some(hits) => {
                let idx: Vec<usize> = hits.iter().filter_map(|h| segs.iter().position(|(sid, _)| sid == h)).collect();
                render_lines(&lines, &windows_around(&idx, lines.len()))
            }
            // No hit: the start of a recent meeting.
            None => render_lines(&lines, &(0..lines.len().min(40)).collect::<Vec<_>>()),
        };
        let block = format!("## {} | {}\n{}\n\n", meta.title, meta.date, body);
        let cost = rough_token_count(&block);
        if used + cost > budget {
            if sources.is_empty() {
                // At least part of the best meeting, cut to fit.
                let keep: String = block.chars().take((budget as f64 / 0.35) as usize).collect();
                out.push_str(&keep);
                sources.push(Source { meeting_id: id, title: meta.title.clone(), date: meta.date.clone() });
            }
            break;
        }
        used += cost;
        out.push_str(&block);
        sources.push(Source { meeting_id: id, title: meta.title.clone(), date: meta.date.clone() });
    }
    Ok((out.trim_end().to_string(), sources))
}

pub async fn load_history(pool: &SqlitePool) -> Result<Vec<AssistantMessage>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, String, String, String)>(
        "SELECT id, role, content, sources, created_at FROM assistant_chat_messages ORDER BY created_at, rowid",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, role, content, sources, created_at)| AssistantMessage {
            id,
            role,
            content,
            sources: serde_json::from_str(&sources).unwrap_or_default(),
            created_at,
        })
        .collect())
}

async fn save(pool: &SqlitePool, role: &str, content: &str, sources: &[Source]) -> Result<AssistantMessage, String> {
    let m = AssistantMessage {
        id: format!("asst-{}", uuid::Uuid::new_v4()),
        role: role.into(),
        content: content.into(),
        sources: sources.to_vec(),
        created_at: Utc::now().to_rfc3339(),
    };
    sqlx::query("INSERT INTO assistant_chat_messages (id, role, content, sources, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&m.id)
        .bind(&m.role)
        .bind(&m.content)
        .bind(serde_json::to_string(&m.sources).unwrap_or_else(|_| "[]".into()))
        .bind(&m.created_at)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to save chat: {}", e))?;
    Ok(m)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn api_get_assistant_chat(state: tauri::State<'_, AppState>) -> Result<Vec<AssistantMessage>, String> {
    load_history(state.db_manager.pool()).await.map_err(|e| format!("Failed to load chat: {}", e))
}

#[tauri::command]
pub async fn api_clear_assistant_chat(state: tauri::State<'_, AppState>) -> Result<(), String> {
    sqlx::query("DELETE FROM assistant_chat_messages")
        .execute(state.db_manager.pool())
        .await
        .map_err(|e| format!("Failed to clear chat: {}", e))?;
    Ok(())
}

/// Asks a question across all meetings. Returns the saved question and answer.
#[tauri::command]
pub async fn api_chat_all_meetings<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    question: String,
    model: Option<String>,
    model_name: Option<String>,
) -> Result<Vec<AssistantMessage>, String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("Question is empty".into());
    }
    let pool = state.db_manager.pool().clone();
    let (model, model_name) = resolve_model(&pool, model, model_name).await?;
    let settings = load_provider_settings(&pool, &model).await?;
    let history = load_history(&pool).await.map_err(|e| format!("Failed to load chat: {}", e))?;
    let recent = recent_meetings(&pool, CATALOG_SIZE).await?;
    if recent.is_empty() {
        return Err("There are no meetings yet.".into());
    }
    let cat = catalog(&recent);
    let history_tokens: usize = history.iter().rev().take(MAX_HISTORY_TURNS).map(|m| rough_token_count(&m.content)).sum();
    let budget = context_budget(&settings, &model_name)
        .await
        .saturating_sub(rough_token_count(&cat) + history_tokens)
        .max(500);
    let (excerpts, included) = gather(&pool, &question, &recent, budget).await?;
    let user_prompt = build_user_prompt(&cat, &excerpts, &history, &question);
    info!(
        "assistant chat: {} meeting(s) in context, ~{} prompt tokens, model {}",
        included.len(),
        rough_token_count(&user_prompt),
        model_name
    );
    let answer = settings.complete(&app, &model_name, SYSTEM_PROMPT, &user_prompt).await?;
    if answer.trim().is_empty() {
        return Err("The model returned an empty answer.".into());
    }
    let sources = cited_sources(&answer, &included);
    let q = save(&pool, "user", &question, &[]).await?;
    let a = save(&pool, "assistant", &answer, &sources).await?;
    Ok(vec![q, a])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_terms() {
        assert_eq!(any_terms_query("What did we decide about the Kubernetes budget?").unwrap(), "\"budget\"* OR \"decide\"* OR \"kubernetes\"*");
        assert_eq!(any_terms_query("what is it?"), None);
    }

    #[test]
    fn windows_and_render() {
        let lines: Vec<String> = (0..20).map(|i| format!("l{}", i)).collect();
        let keep = windows_around(&[1, 10], lines.len());
        assert_eq!(keep, vec![0, 1, 2, 3, 4, 7, 8, 9, 10, 11, 12, 13]);
        let r = render_lines(&lines, &keep);
        assert!(r.starts_with("l0\nl1"));
        assert!(r.contains("l4\n...\nl7"));
        assert!(r.ends_with("l13\n..."));
    }

    #[test]
    fn cites_mentioned_meetings_only() {
        let inc = vec![
            Source { meeting_id: "a".into(), title: "Weekly Planning".into(), date: "x".into() },
            Source { meeting_id: "b".into(), title: "Retro".into(), date: "y".into() },
        ];
        assert_eq!(cited_sources("Per (weekly planning, 12:30) ...", &inc).len(), 1);
        assert_eq!(cited_sources("Nothing named", &inc).len(), 2);
    }

    #[tokio::test]
    async fn gathers_hits_with_context_and_recent_meetings() {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for (id, title, at) in [("m1", "Planning", "2026-11-01T10:00:00Z"), ("m2", "Retro", "2026-11-05T10:00:00Z")] {
            sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
                .bind(id).bind(title).bind(at).bind(at).execute(&pool).await.unwrap();
        }
        for i in 0..30 {
            let text = if i == 15 { "the kubernetes budget was approved".to_string() } else { format!("planning filler {}", i) };
            sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time) VALUES (?, 'm1', ?, '10:00', ?)")
                .bind(format!("p{}", i)).bind(text).bind(i as f64 * 10.0).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time) VALUES ('r0', 'm2', 'retro notes', '10:00', 0)")
            .execute(&pool).await.unwrap();

        let recent = recent_meetings(&pool, 10).await.unwrap();
        assert_eq!(recent[0].title, "Retro");
        let (ex, src) = gather(&pool, "What happened with the Kubernetes budget?", &recent, 10_000).await.unwrap();
        assert_eq!(src[0].meeting_id, "m1", "hit meeting first");
        assert!(ex.contains("## Planning |"));
        assert!(ex.contains("[2:30] the kubernetes budget was approved"));
        assert!(ex.contains("[2:00] planning filler 12"), "context before the hit");
        assert!(!ex.contains("planning filler 3\n"), "far lines dropped");
        assert!(ex.contains("## Retro |"), "recent meeting fills the rest");

        let m = save(&pool, "assistant", "hi", &src).await.unwrap();
        let h = load_history(&pool).await.unwrap();
        assert_eq!(h, vec![m]);
    }
}

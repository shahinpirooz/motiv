//! Motiv: chat with a meeting.
//!
//! Answers questions about one meeting from its transcript, using the same
//! LLM provider and model the user configured for summaries. The whole
//! transcript is sent when it fits the model's context; otherwise the most
//! relevant transcript windows are picked by keyword overlap with the
//! question. Messages are stored in `meeting_chat_messages`.

use crate::database::repositories::setting::SettingsRepository;
use crate::ollama::metadata::ModelMetadataCache;
use crate::state::AppState;
use crate::summary::llm_client::{generate_summary, LLMProvider};
use crate::summary::processor::{clean_llm_markdown_detailed, rough_token_count};
use chrono::Utc;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

static METADATA_CACHE: LazyLock<ModelMetadataCache> =
    LazyLock::new(|| ModelMetadataCache::new(Duration::from_secs(300)));

/// Tokens kept free for instructions, chat history and the answer.
const RESERVED_TOKENS: usize = 1500;
/// Most recent chat turns sent back to the model as conversation context.
const MAX_HISTORY_TURNS: usize = 8;
/// Context budget for cloud providers (they all accept far more than this).
const CLOUD_CONTEXT_TOKENS: usize = 100_000;
/// Transcript lines per retrieval window when the transcript must be trimmed.
const WINDOW_LINES: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessage {
    pub id: String,
    pub meeting_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

/// One transcript segment as stored in the database.
#[derive(Debug, Clone)]
pub struct Segment {
    pub text: String,
    pub timestamp: String,
    pub audio_start_time: Option<f64>,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit tested)
// ---------------------------------------------------------------------------

/// Formats seconds as `m:ss` or `h:mm:ss`.
pub fn format_offset(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

/// One line per segment: `[time] text`. Uses the recording offset when known,
/// otherwise the stored wall-clock timestamp.
pub fn transcript_lines(segments: &[Segment]) -> Vec<String> {
    segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| {
            let time = match s.audio_start_time {
                Some(t) => format_offset(t),
                None => s.timestamp.trim().to_string(),
            };
            if time.is_empty() {
                s.text.trim().to_string()
            } else {
                format!("[{}] {}", time, s.text.trim())
            }
        })
        .collect()
}

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "did", "do", "does", "for",
    "from", "had", "has", "have", "how", "i", "in", "is", "it", "its", "me", "of", "on", "or",
    "our", "so", "that", "the", "their", "them", "then", "there", "they", "this", "to", "was",
    "we", "were", "what", "when", "where", "which", "who", "why", "will", "with", "you", "your",
    "about", "any", "all", "said", "say", "says", "meeting", "tell",
];

fn keywords(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.chars().count() > 2 && !STOPWORDS.contains(&w.as_str()))
        .collect()
}

/// Returns the transcript text to send. All of it when it fits `budget_tokens`;
/// otherwise the windows sharing the most keywords with `question`, kept in
/// transcript order with `...` between gaps. Always includes the opening
/// window so the model sees how the meeting started.
pub fn select_context(lines: &[String], question: &str, budget_tokens: usize) -> String {
    let full = lines.join("\n");
    if rough_token_count(&full) <= budget_tokens {
        return full;
    }

    let terms = keywords(question);
    let windows: Vec<(usize, usize)> = (0..lines.len())
        .step_by(WINDOW_LINES)
        .map(|start| (start, (start + WINDOW_LINES).min(lines.len())))
        .collect();

    let mut scored: Vec<(usize, usize)> = windows
        .iter()
        .enumerate()
        .map(|(i, &(a, b))| {
            let words = keywords(&lines[a..b].join(" "));
            (i, terms.intersection(&words).count())
        })
        .collect();
    // Highest score first; earlier windows win ties.
    scored.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));

    let mut chosen: Vec<usize> = Vec::new();
    let mut used = 0usize;
    let order = std::iter::once(0).chain(scored.iter().map(|&(i, _)| i));
    for i in order {
        if chosen.contains(&i) {
            continue;
        }
        let (a, b) = windows[i];
        let cost = rough_token_count(&lines[a..b].join("\n")) + 2;
        if used + cost > budget_tokens {
            continue;
        }
        used += cost;
        chosen.push(i);
    }
    chosen.sort_unstable();

    let mut out = String::new();
    let mut prev: Option<usize> = None;
    for i in chosen {
        if let Some(p) = prev {
            if i != p + 1 {
                out.push_str("\n...\n");
            } else {
                out.push('\n');
            }
        } else if i != 0 {
            out.push_str("...\n");
        }
        let (a, b) = windows[i];
        out.push_str(&lines[a..b].join("\n"));
        prev = Some(i);
    }
    out
}

pub const SYSTEM_PROMPT: &str = "You answer questions about a single meeting using only its transcript. \
The transcript lines start with a timestamp in square brackets. \
Rules: \
1. Use only information in the transcript. If the answer is not there, say so plainly. \
2. When you state a fact, cite the timestamp it comes from, like [12:34]. \
3. Be concise. Use short markdown lists when listing several items. \
4. Do not invent names, numbers, dates or decisions. \
5. If the transcript is marked with ... it was trimmed to the most relevant parts; say so if that may limit the answer.";

/// Builds the user prompt from the meeting context, earlier turns and the question.
pub fn build_user_prompt(
    meeting_title: &str,
    transcript: &str,
    history: &[ChatMessage],
    question: &str,
) -> String {
    let mut p = String::new();
    p.push_str(&format!("Meeting: {}\n\n<transcript>\n{}\n</transcript>\n", meeting_title, transcript));
    let recent: Vec<&ChatMessage> = history
        .iter()
        .rev()
        .take(MAX_HISTORY_TURNS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
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

// ---------------------------------------------------------------------------
// Provider settings
// ---------------------------------------------------------------------------

pub(crate) struct ProviderSettings {
    pub provider: LLMProvider,
    pub api_key: String,
    pub ollama_endpoint: Option<String>,
    pub custom_endpoint: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

impl ProviderSettings {
    /// Sends one system + user prompt to the configured provider and returns
    /// the visible answer (reasoning blocks and code fences stripped).
    pub async fn complete<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        model_name: &str,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<String, String> {
        let app_data_dir: Option<PathBuf> = app.path().app_data_dir().ok();
        let client = reqwest::Client::new();
        let completion = generate_summary(
            &client,
            &self.provider,
            model_name,
            &self.api_key,
            system_prompt,
            user_prompt,
            self.ollama_endpoint.as_deref(),
            self.custom_endpoint.as_deref(),
            self.max_tokens,
            self.temperature,
            self.top_p,
            app_data_dir.as_ref(),
            None,
        )
        .await?;
        Ok(clean_llm_markdown_detailed(&completion.content).markdown)
    }
}

pub(crate) async fn load_provider_settings(pool: &SqlitePool, model_provider: &str) -> Result<ProviderSettings, String> {
    let provider = LLMProvider::from_str(model_provider)?;
    let mut s = ProviderSettings {
        provider: provider.clone(),
        api_key: String::new(),
        ollama_endpoint: None,
        custom_endpoint: None,
        max_tokens: None,
        temperature: None,
        top_p: None,
    };
    match provider {
        LLMProvider::BuiltInAI => {}
        LLMProvider::Ollama => {
            s.ollama_endpoint = SettingsRepository::get_model_config(pool)
                .await
                .ok()
                .flatten()
                .and_then(|c| c.ollama_endpoint);
        }
        LLMProvider::CustomOpenAI => {
            let config = SettingsRepository::get_custom_openai_config(pool)
                .await
                .map_err(|e| format!("Failed to read custom OpenAI settings: {}", e))?
                .ok_or("Custom OpenAI provider selected but not configured")?;
            s.custom_endpoint = Some(config.endpoint);
            s.api_key = config.api_key.unwrap_or_default();
            s.max_tokens = config.max_tokens.map(|t| t as u32);
            s.temperature = config.temperature;
            s.top_p = config.top_p;
        }
        _ => {
            s.api_key = SettingsRepository::get_api_key(pool, model_provider)
                .await
                .map_err(|e| format!("Failed to read API key: {}", e))?
                .filter(|k| !k.is_empty())
                .ok_or_else(|| format!("API key not found for {}", model_provider))?;
        }
    }
    Ok(s)
}

pub(crate) async fn context_budget(settings: &ProviderSettings, model_name: &str) -> usize {
    let window = match settings.provider {
        LLMProvider::Ollama => match METADATA_CACHE
            .get_or_fetch(model_name, settings.ollama_endpoint.as_deref())
            .await
        {
            Ok(m) => m.context_size,
            Err(e) => {
                warn!("chat: could not read Ollama context size ({}); using 4096", e);
                4096
            }
        },
        LLMProvider::BuiltInAI => crate::summary::summary_engine::models::get_model_by_name(model_name)
            .map(|m| m.context_size as usize)
            .unwrap_or(2048),
        _ => CLOUD_CONTEXT_TOKENS,
    };
    window.saturating_sub(RESERVED_TOKENS).max(500)
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

pub async fn load_history(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<ChatMessage>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, String, String, String)>(
        "SELECT id, meeting_id, role, content, created_at FROM meeting_chat_messages
         WHERE meeting_id = ? ORDER BY created_at, rowid",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, meeting_id, role, content, created_at)| ChatMessage { id, meeting_id, role, content, created_at })
        .collect())
}

async fn save_message(pool: &SqlitePool, meeting_id: &str, role: &str, content: &str) -> Result<ChatMessage, sqlx::Error> {
    let msg = ChatMessage {
        id: format!("chat-{}", uuid::Uuid::new_v4()),
        meeting_id: meeting_id.to_string(),
        role: role.to_string(),
        content: content.to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    sqlx::query("INSERT INTO meeting_chat_messages (id, meeting_id, role, content, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&msg.id)
        .bind(&msg.meeting_id)
        .bind(&msg.role)
        .bind(&msg.content)
        .bind(&msg.created_at)
        .execute(pool)
        .await?;
    Ok(msg)
}

pub(crate) async fn load_meeting(pool: &SqlitePool, meeting_id: &str) -> Result<(String, Vec<Segment>), String> {
    let title: Option<(String,)> = sqlx::query_as("SELECT title FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Failed to load meeting: {}", e))?;
    let (title,) = title.ok_or("Meeting not found")?;
    let rows = sqlx::query_as::<_, (String, String, Option<f64>)>(
        "SELECT transcript, timestamp, audio_start_time FROM transcripts
         WHERE meeting_id = ? ORDER BY COALESCE(audio_start_time, 0), timestamp, rowid",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to load transcript: {}", e))?;
    let segments = rows
        .into_iter()
        .map(|(text, timestamp, audio_start_time)| Segment { text, timestamp, audio_start_time })
        .collect();
    Ok((title, segments))
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn api_get_meeting_chat(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<ChatMessage>, String> {
    load_history(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| format!("Failed to load chat: {}", e))
}

#[tauri::command]
pub async fn api_clear_meeting_chat(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<(), String> {
    sqlx::query("DELETE FROM meeting_chat_messages WHERE meeting_id = ?")
        .bind(&meeting_id)
        .execute(state.db_manager.pool())
        .await
        .map_err(|e| format!("Failed to clear chat: {}", e))?;
    Ok(())
}

/// Asks a question about a meeting. Saves the question and the answer and
/// returns both messages, the answer last.
#[tauri::command]
pub async fn api_chat_with_meeting<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    question: String,
    model: String,
    model_name: String,
) -> Result<Vec<ChatMessage>, String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("Question is empty".into());
    }
    let pool = state.db_manager.pool().clone();

    let (title, segments) = load_meeting(&pool, &meeting_id).await?;
    let lines = transcript_lines(&segments);
    if lines.is_empty() {
        return Err("This meeting has no transcript yet.".into());
    }
    let history = load_history(&pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to load chat: {}", e))?;

    let settings = load_provider_settings(&pool, &model).await?;
    let budget = context_budget(&settings, &model_name).await;
    let history_tokens: usize = history
        .iter()
        .rev()
        .take(MAX_HISTORY_TURNS)
        .map(|m| rough_token_count(&m.content))
        .sum();
    let transcript = select_context(&lines, &question, budget.saturating_sub(history_tokens).max(500));
    let user_prompt = build_user_prompt(&title, &transcript, &history, &question);

    info!(
        "chat: asking {} ({} transcript lines, ~{} prompt tokens)",
        model_name,
        lines.len(),
        rough_token_count(&user_prompt)
    );

    let answer = settings.complete(&app, &model_name, SYSTEM_PROMPT, &user_prompt).await?;
    if answer.trim().is_empty() {
        return Err("The model returned an empty answer.".into());
    }

    let q = save_message(&pool, &meeting_id, "user", &question)
        .await
        .map_err(|e| format!("Failed to save question: {}", e))?;
    let a = save_message(&pool, &meeting_id, "assistant", &answer)
        .await
        .map_err(|e| format!("Failed to save answer: {}", e))?;
    Ok(vec![q, a])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str, t: Option<f64>) -> Segment {
        Segment { text: text.into(), timestamp: "10:00:00".into(), audio_start_time: t }
    }

    #[test]
    fn formats_offsets() {
        assert_eq!(format_offset(5.4), "0:05");
        assert_eq!(format_offset(754.0), "12:34");
        assert_eq!(format_offset(3725.0), "1:02:05");
    }

    #[test]
    fn lines_use_offset_or_timestamp_and_skip_blank() {
        let lines = transcript_lines(&[seg("hello", Some(65.0)), seg("  ", Some(70.0)), seg("bye", None)]);
        assert_eq!(lines, vec!["[1:05] hello".to_string(), "[10:00:00] bye".to_string()]);
    }

    #[test]
    fn small_transcript_is_sent_whole() {
        let lines: Vec<String> = (0..5).map(|i| format!("[0:0{}] line {}", i, i)).collect();
        assert_eq!(select_context(&lines, "anything", 10_000), lines.join("\n"));
    }

    #[test]
    fn large_transcript_keeps_opening_and_relevant_window() {
        let mut lines: Vec<String> = (0..200).map(|i| format!("[{}] filler talk number {}", i, i)).collect();
        lines[150] = "[150] the budget for kubernetes migration is approved".into();
        let full = rough_token_count(&lines.join("\n"));
        let ctx = select_context(&lines, "What about the Kubernetes budget?", full / 5);
        assert!(ctx.starts_with("[0] filler"));
        assert!(ctx.contains("kubernetes migration"));
        assert!(ctx.contains("..."));
        assert!(rough_token_count(&ctx) <= full / 5 + 10);
    }

    #[test]
    fn prompt_includes_recent_history_and_question() {
        let h: Vec<ChatMessage> = (0..12)
            .map(|i| ChatMessage {
                id: i.to_string(),
                meeting_id: "m".into(),
                role: if i % 2 == 0 { "user".into() } else { "assistant".into() },
                content: format!("turn {}", i),
                created_at: String::new(),
            })
            .collect();
        let p = build_user_prompt("Sync", "[0:01] hi", &h, "Next steps?");
        assert!(p.contains("Meeting: Sync"));
        assert!(p.contains("[0:01] hi"));
        assert!(!p.contains("turn 3\n"));
        assert!(p.contains("Assistant: turn 11"));
        assert!(p.trim_end().ends_with("Question: Next steps?"));
    }
}

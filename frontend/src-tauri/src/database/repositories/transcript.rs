use crate::api::{TranscriptSearchResult, TranscriptSegment};
use chrono::Utc;
use sqlx::{Connection, Error as SqlxError, SqlitePool};
use log::{error, info};
use uuid::Uuid;

pub struct TranscriptsRepository;

impl TranscriptsRepository {
    /// Saves a new meeting and its associated transcript segments.
    /// This function uses a transaction to ensure that either both the meeting
    /// and all its transcripts are saved, or none of them are.
    pub async fn save_transcript(
        pool: &SqlitePool,
        meeting_title: &str,
        transcripts: &[TranscriptSegment],
        folder_path: Option<String>,
    ) -> Result<String, SqlxError> {
        let meeting_id = format!("meeting-{}", Uuid::new_v4());

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let now = Utc::now();

        // 1. Create the new meeting
        let result = sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&meeting_id)
        .bind(meeting_title)
        .bind(now)
        .bind(now)
        .bind(&folder_path)
        .execute(&mut *transaction)
        .await;

        if let Err(e) = result {
            error!("Failed to create meeting with id: {}", meeting_id);
            transaction.rollback().await?;
            return Err(e);
        }

        info!("Successfully created meeting with id: {}", meeting_id);

        // 2. Save each transcript segment with audio timing fields
        for segment in transcripts {
            let transcript_id = format!("transcript-{}", Uuid::new_v4());
            let result = sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration)
                 VALUES (?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&transcript_id)
            .bind(&meeting_id)
            .bind(&segment.text)
            .bind(&segment.timestamp)
            .bind(segment.audio_start_time)
            .bind(segment.audio_end_time)
            .bind(segment.duration)
            .execute(&mut *transaction)
            .await;

            if let Err(e) = result {
                error!("Failed to save transcript segment for meeting {}", meeting_id);
                transaction.rollback().await?;
                return Err(e);
            }
        }

        info!(
            "Successfully saved {} transcript segments for meeting {}",
            transcripts.len(),
            meeting_id
        );

        // Commit the transaction
        transaction.commit().await?;

        Ok(meeting_id)
    }

    /// Motiv: full-text search (FTS5) across all transcripts.
    /// Returns the best-ranked matching segment per meeting, best meetings
    /// first, with a highlighted snippet. Falls back to a substring search
    /// when the query has no searchable words.
    pub async fn search_transcripts(
        pool: &SqlitePool,
        query: &str,
    ) -> Result<Vec<TranscriptSearchResult>, SqlxError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let Some(fts_query) = Self::fts_query(query) else {
            return Self::search_transcripts_like(pool, query).await;
        };

        let rows = sqlx::query_as::<_, (String, String, String, String)>(
            "WITH hits AS (
                 SELECT f.meeting_id AS meeting_id,
                        f.transcript_id AS transcript_id,
                        snippet(transcripts_fts, 0, '', '', '...', 24) AS snip,
                        bm25(transcripts_fts) AS score
                 FROM transcripts_fts f
                 WHERE transcripts_fts MATCH ?
             ),
             best AS (
                 SELECT meeting_id, transcript_id, snip, score,
                        ROW_NUMBER() OVER (PARTITION BY meeting_id ORDER BY score) AS rn
                 FROM hits
             )
             SELECT m.id, m.title, b.snip, t.timestamp
             FROM best b
             JOIN meetings m ON m.id = b.meeting_id
             JOIN transcripts t ON t.id = b.transcript_id
             WHERE b.rn = 1
             ORDER BY b.score
             LIMIT 200",
        )
        .bind(&fts_query)
        .fetch_all(pool)
        .await;

        match rows {
            Ok(rows) => Ok(rows
                .into_iter()
                .map(|(id, title, match_context, timestamp)| TranscriptSearchResult {
                    id,
                    title,
                    match_context,
                    timestamp,
                })
                .collect()),
            Err(e) => {
                error!("Full-text search failed, using substring search: {}", e);
                Self::search_transcripts_like(pool, query).await
            }
        }
    }

    /// Turns free text into a safe FTS5 query: every word must appear, the
    /// last word may be a prefix (search-as-you-type), and "quoted phrases"
    /// are kept together. Returns None when there is nothing to search for.
    pub fn fts_query(input: &str) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        for (i, chunk) in input.split('"').enumerate() {
            let words: Vec<String> = chunk
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .map(|w| w.to_lowercase())
                .collect();
            if words.is_empty() {
                continue;
            }
            if i % 2 == 1 {
                parts.push(format!("\"{}\"", words.join(" ")));
            } else {
                parts.extend(words.into_iter().map(|w| format!("\"{}\"", w)));
            }
        }
        if parts.is_empty() {
            return None;
        }
        let ends_in_word = input.chars().last().map_or(false, |c| c.is_alphanumeric());
        if ends_in_word {
            if let Some(last) = parts.last_mut() {
                last.push('*');
            }
        }
        Some(parts.join(" "))
    }

    /// Substring search, one result per meeting (used when FTS can't run).
    async fn search_transcripts_like(
        pool: &SqlitePool,
        query: &str,
    ) -> Result<Vec<TranscriptSearchResult>, SqlxError> {
        let search_query = format!("%{}%", query.to_lowercase());

        let rows = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT m.id, m.title, t.transcript, t.timestamp
             FROM meetings m
             JOIN transcripts t ON m.id = t.meeting_id
             WHERE LOWER(t.transcript) LIKE ?",
        )
        .bind(&search_query)
        .fetch_all(pool)
        .await?;

        let mut seen = std::collections::HashSet::new();
        let results = rows
            .into_iter()
            .filter(|(id, ..)| seen.insert(id.clone()))
            .map(|(id, title, transcript, timestamp)| {
                let match_context = Self::get_match_context(&transcript, query);
                TranscriptSearchResult {
                    id,
                    title,
                    match_context,
                    timestamp,
                }
            })
            .collect();

        Ok(results)
    }

    /// Helper function to extract a snippet of text around the first match of a query.
    fn get_match_context(transcript: &str, query: &str) -> String {
        let chars: Vec<char> = transcript.chars().collect();
        let lower: Vec<char> = transcript.to_lowercase().chars().collect();
        let needle: Vec<char> = query.to_lowercase().chars().collect();
        // Lowercasing can change length for a few scripts; fall back to the start then.
        let found = if lower.len() == chars.len() && !needle.is_empty() {
            lower.windows(needle.len()).position(|w| w == needle.as_slice())
        } else {
            None
        };
        match found {
            Some(i) => {
                let start = i.saturating_sub(100);
                let end = (i + needle.len() + 100).min(chars.len());
                let mut context = String::new();
                if start > 0 {
                    context.push_str("...");
                }
                context.extend(&chars[start..end]);
                if end < chars.len() {
                    context.push_str("...");
                }
                context
            }
            None => chars.iter().take(200).collect(),
        }
    }
}

#[cfg(test)]
mod search_tests {
    use super::TranscriptsRepository as R;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn builds_safe_fts_queries() {
        assert_eq!(R::fts_query("budget review"), Some("\"budget\" \"review\"*".into()));
        assert_eq!(R::fts_query("budget review "), Some("\"budget\" \"review\"".into()));
        assert_eq!(R::fts_query("\"next steps\" owner"), Some("\"next steps\" \"owner\"*".into()));
        assert_eq!(R::fts_query("AND OR NOT ("), Some("\"and\" \"or\" \"not\"".into()));
        assert_eq!(R::fts_query("*** --"), None);
    }

    #[test]
    fn match_context_handles_multibyte_text() {
        let t = format!("{}café décision{}", "é".repeat(150), "ü".repeat(150));
        let ctx = R::get_match_context(&t, "décision");
        assert!(ctx.contains("café décision"));
        assert!(ctx.starts_with("...") && ctx.ends_with("..."));
    }

    #[tokio::test]
    async fn fts_ranks_and_stays_in_sync() {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for (id, title) in [("m1", "Planning"), ("m2", "Retro")] {
            sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, '2026-01-01', '2026-01-01')")
                .bind(id).bind(title).execute(&pool).await.unwrap();
        }
        let rows = [
            ("t1", "m1", "We reviewed the Kubernetes migration budget."),
            ("t2", "m1", "Lunch options were discussed."),
            ("t3", "m2", "Budget budget budget, the budgets are tight."),
        ];
        for (id, m, text) in rows {
            sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES (?, ?, ?, '10:00')")
                .bind(id).bind(m).bind(text).execute(&pool).await.unwrap();
        }

        let r = R::search_transcripts(&pool, "kubernetes budget").await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "m1");
        assert!(r[0].match_context.contains("Kubernetes"));

        // Stemming + prefix: "budg" matches both meetings, one row each.
        let r = R::search_transcripts(&pool, "budg").await.unwrap();
        assert_eq!(r.len(), 2);

        // Updates and deletes are reflected.
        sqlx::query("UPDATE transcripts SET transcript = 'Nothing here' WHERE id = 't3'").execute(&pool).await.unwrap();
        assert_eq!(R::search_transcripts(&pool, "budget").await.unwrap().len(), 1);
        sqlx::query("DELETE FROM transcripts WHERE id = 't1'").execute(&pool).await.unwrap();
        assert!(R::search_transcripts(&pool, "kubernetes").await.unwrap().is_empty());

        // Punctuation-only input falls back without error.
        assert!(R::search_transcripts(&pool, "!!!").await.unwrap().is_empty());
    }
}

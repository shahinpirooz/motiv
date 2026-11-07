//! Motiv: speaker identification ("who said what") with opt-in voiceprints.
//!
//! After a meeting is recorded, its audio is decoded to 16 kHz mono and each
//! transcript segment gets a speaker embedding from the WeSpeaker ResNet34
//! model (ONNX, run with the same ONNX Runtime as Parakeet). Embeddings are
//! clustered into speakers (S1, S2, ...), stored on the transcript rows, and
//! matched against saved voiceprints so known people are named automatically.
//!
//! Voiceprints are biometric data. They are created only when the user ticks
//! "Recognise this voice" for a speaker, stay in the local database, and can
//! be deleted individually or all at once.

pub mod cluster;
pub mod fbank;

use crate::state::AppState;
use cluster::{centroid, cluster, cosine, match_profiles, normalize};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const MODEL_FILE: &str = "wespeaker_en_voxceleb_resnet34.onnx";
const MODEL_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/wespeaker_en_voxceleb_resnet34.onnx";
const MODEL_SHA256: &str = "5ef208a9da1453335308a6b6f4e6dfbd7e183a38b604de0a57664f45d257fe94";
const MODEL_BYTES: u64 = 26_534_365;

/// Segments shorter than this are labelled from their neighbours.
const MIN_SEGMENT_SECS: f64 = 1.0;
/// Long segments are trimmed to their middle part for embedding.
const MAX_SEGMENT_SECS: f64 = 12.0;
/// Average-linkage cosine distance at which clusters stop merging.
const CLUSTER_DISTANCE: f32 = 0.7;
/// Clusters with less than this share of speech fold into the nearest one.
const MIN_SPEAKER_SHARE: f32 = 0.03;
/// Cosine similarity needed to recognise a saved voiceprint.
const PROFILE_MATCH: f32 = 0.55;

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

fn model_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("models").join("speaker").join(MODEL_FILE))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

#[derive(Clone, Serialize)]
struct Progress {
    meeting_id: String,
    stage: &'static str,
    percent: u32,
}

fn progress<R: Runtime>(app: &AppHandle<R>, meeting_id: &str, stage: &'static str, percent: u32) {
    let _ = app.emit("speakers-progress", Progress { meeting_id: meeting_id.into(), stage, percent });
}

/// Downloads the speaker model on first use (26 MB) and verifies it.
async fn ensure_model<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> Result<PathBuf, String> {
    let path = model_path(app)?;
    if path.exists() && std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) == MODEL_BYTES {
        return Ok(path);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    info!("speakers: downloading speaker model");
    let mut resp = reqwest::Client::new()
        .get(MODEL_URL)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Could not download the speaker model: {}", e))?;
    let tmp = path.with_extension("onnx.part");
    let mut file = tokio::fs::File::create(&tmp).await.map_err(|e| e.to_string())?;
    let mut got: u64 = 0;
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Download interrupted: {}", e))? {
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        got += chunk.len() as u64;
        progress(app, meeting_id, "download", ((got * 100) / MODEL_BYTES).min(100) as u32);
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    let digest = sha256_file(&tmp)?;
    if digest != MODEL_SHA256 {
        let _ = std::fs::remove_file(&tmp);
        return Err("The downloaded speaker model failed its integrity check. Try again.".into());
    }
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Runs the embedding model; loaded once and kept.
pub struct Embedder {
    session: ort::session::Session,
}

static EMBEDDER: LazyLock<Mutex<Option<Embedder>>> = LazyLock::new(|| Mutex::new(None));

impl Embedder {
    pub fn load(path: &Path) -> Result<Self, String> {
        let session = ort::session::Session::builder()
            .and_then(|b| b.with_intra_threads(2))
            .and_then(|b| b.commit_from_file(path))
            .map_err(|e| format!("Could not load the speaker model: {}", e))?;
        Ok(Self { session })
    }

    /// L2-normalised 256-d embedding of 16 kHz mono audio, or None if too short.
    pub fn embed(&mut self, samples: &[f32]) -> Result<Option<Vec<f32>>, String> {
        let feats = fbank::fbank(samples);
        if feats.len() < 50 {
            return Ok(None);
        }
        let flat: Vec<f32> = feats.iter().flat_map(|r| r.iter().copied()).collect();
        let arr = ndarray::Array3::from_shape_vec((1, feats.len(), fbank::NUM_BINS), flat).map_err(|e| e.to_string())?;
        let outputs = self
            .session
            .run(ort::inputs!["feats" => ort::value::TensorRef::from_array_view(arr.view()).map_err(|e| e.to_string())?])
            .map_err(|e| format!("Speaker model failed: {}", e))?;
        let emb = outputs
            .get("embs")
            .ok_or("Speaker model output missing")?
            .try_extract_array::<f32>()
            .map_err(|e| e.to_string())?;
        let mut v: Vec<f32> = emb.iter().copied().collect();
        normalize(&mut v);
        Ok(Some(v))
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (unit tested)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SegmentTime {
    pub id: String,
    pub start: f64,
    pub end: f64,
}

/// The audio span to embed for a segment: clamped to the audio, trimmed to
/// its middle `MAX_SEGMENT_SECS`, None if shorter than `MIN_SEGMENT_SECS`.
pub fn embed_span(start: f64, end: f64, audio_secs: f64) -> Option<(usize, usize)> {
    let (s, e) = (start.max(0.0), end.min(audio_secs));
    if e - s < MIN_SEGMENT_SECS {
        return None;
    }
    let (s, e) = if e - s > MAX_SEGMENT_SECS {
        let mid = (s + e) / 2.0;
        (mid - MAX_SEGMENT_SECS / 2.0, mid + MAX_SEGMENT_SECS / 2.0)
    } else {
        (s, e)
    };
    Some(((s * 16_000.0) as usize, (e * 16_000.0) as usize))
}

/// Fills unlabelled segments (too short to embed) from the previous labelled
/// one, or the next one at the start.
pub fn fill_gaps(labels: &mut [Option<usize>]) {
    let first = labels.iter().flatten().next().copied();
    let mut last = first;
    for l in labels.iter_mut() {
        match l {
            Some(v) => last = Some(*v),
            None => *l = last,
        }
    }
}

pub fn speaker_key(index: usize) -> String {
    format!("S{}", index + 1)
}

pub fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Weighted running average of a voiceprint with a new meeting's centroid.
pub fn blend(profile: &[f32], samples: i64, new: &[f32]) -> Vec<f32> {
    let w = samples.max(1) as f32;
    let mut v: Vec<f32> = profile.iter().zip(new).map(|(p, n)| (p * w + n) / (w + 1.0)).collect();
    normalize(&mut v);
    v
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Speaker {
    pub key: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub profile_id: Option<String>,
    pub seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoiceProfile {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub samples: i64,
    pub updated_at: String,
}

pub async fn load_speakers(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<Speaker>, String> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, f64)>(
        "SELECT speaker_key, name, email, profile_id, seconds FROM meeting_speakers WHERE meeting_id = ?
         ORDER BY CAST(SUBSTR(speaker_key, 2) AS INTEGER)",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to load speakers: {}", e))?;
    Ok(rows.into_iter().map(|(key, name, email, profile_id, seconds)| Speaker { key, name, email, profile_id, seconds }).collect())
}

/// Display name per speaker key ("Priya N", or "Speaker 2" when unnamed).
pub async fn speaker_names(pool: &SqlitePool, meeting_id: &str) -> std::collections::HashMap<String, String> {
    load_speakers(pool, meeting_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let label = s.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Speaker {}", &s.key[1..]));
            (s.key, label)
        })
        .collect()
}

async fn profiles_with_embeddings(pool: &SqlitePool) -> Vec<(VoiceProfile, Vec<f32>)> {
    sqlx::query_as::<_, (String, String, Option<String>, Vec<u8>, i64, String)>(
        "SELECT id, name, email, embedding, samples, updated_at FROM voice_profiles",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(id, name, email, emb, samples, updated_at)| (VoiceProfile { id, name, email, samples, updated_at }, from_blob(&emb)))
    .collect()
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

fn meeting_audio(folder: &Path) -> Option<PathBuf> {
    let preferred = ["audio.mp4", "audio.m4a", "audio.wav", "recording.mp4"];
    for name in preferred {
        let p = folder.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && crate::storage::is_audio(p))
        .max_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
}

/// Embeds each long-enough segment and clusters them. Returns per-segment
/// labels (None = no audio) and per-speaker (centroid, seconds).
pub fn analyze_audio(
    embedder: &mut Embedder,
    audio: &[f32],
    segments: &[SegmentTime],
    expected_speakers: Option<usize>,
    mut on_progress: impl FnMut(u32),
) -> Result<(Vec<Option<usize>>, Vec<(Vec<f32>, f64)>), String> {
    let audio_secs = audio.len() as f64 / 16_000.0;
    let mut embedded: Vec<(usize, Vec<f32>, f32)> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        if let Some((a, b)) = embed_span(seg.start, seg.end, audio_secs) {
            if let Some(e) = embedder.embed(&audio[a..b.min(audio.len())])? {
                embedded.push((i, e, (seg.end - seg.start) as f32));
            }
        }
        on_progress(((i + 1) * 100 / segments.len().max(1)) as u32);
    }
    if embedded.is_empty() {
        return Err("Not enough speech in the recording to tell speakers apart.".into());
    }
    let embs: Vec<Vec<f32>> = embedded.iter().map(|(_, e, _)| e.clone()).collect();
    let weights: Vec<f32> = embedded.iter().map(|(_, _, w)| *w).collect();
    let target = expected_speakers.filter(|n| *n >= 1 && *n < embs.len());
    let labels = cluster(&embs, &weights, CLUSTER_DISTANCE, target, MIN_SPEAKER_SHARE);

    let mut seg_labels: Vec<Option<usize>> = vec![None; segments.len()];
    for ((i, _, _), l) in embedded.iter().zip(&labels) {
        seg_labels[*i] = Some(*l);
    }
    fill_gaps(&mut seg_labels);

    let n = labels.iter().max().map_or(0, |m| m + 1);
    let mut speakers = Vec::with_capacity(n);
    for s in 0..n {
        let members: Vec<&[f32]> = embedded.iter().zip(&labels).filter(|(_, l)| **l == s).map(|((_, e, _), _)| e.as_slice()).collect();
        let secs: f64 = segments.iter().zip(&seg_labels).filter(|(_, l)| **l == Some(s)).map(|(g, _)| (g.end - g.start).max(0.0)).sum();
        speakers.push((centroid(&members), secs));
    }
    Ok((seg_labels, speakers))
}

/// Number of people who attended (Teams report) or were invited and didn't
/// decline, to guide clustering. None if unknown or implausible.
async fn expected_from_calendar(pool: &SqlitePool, meeting_id: &str) -> Option<usize> {
    let link = crate::calendar::get_link(pool, meeting_id).await?;
    let n = match &link.attendance {
        Some(a) => a.len(),
        None => return None, // invite lists over-count (people who never joined)
    };
    (2..=12).contains(&n).then_some(n)
}

#[derive(Debug, Clone, Serialize)]
pub struct MeetingSpeakers {
    pub speakers: Vec<Speaker>,
    /// transcript id -> speaker key
    pub assignments: Vec<(String, String)>,
    pub analyzed: bool,
}

async fn meeting_speakers_view(pool: &SqlitePool, meeting_id: &str) -> Result<MeetingSpeakers, String> {
    let speakers = load_speakers(pool, meeting_id).await?;
    let assignments = sqlx::query_as::<_, (String, String)>(
        "SELECT id, speaker FROM transcripts WHERE meeting_id = ? AND speaker LIKE 'S%'",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(MeetingSpeakers { analyzed: !speakers.is_empty(), speakers, assignments })
}

static RUNNING: LazyLock<Mutex<std::collections::HashSet<String>>> = LazyLock::new(Default::default);

/// Identifies speakers in a recorded meeting (downloads the model on first use).
#[tauri::command]
pub async fn speakers_analyze<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    expected_speakers: Option<usize>,
) -> Result<MeetingSpeakers, String> {
    if !RUNNING.lock().unwrap().insert(meeting_id.clone()) {
        return Err("Already identifying speakers for this meeting.".into());
    }
    let result = analyze(&app, state.db_manager.pool(), &meeting_id, expected_speakers).await;
    RUNNING.lock().unwrap().remove(&meeting_id);
    result
}

async fn analyze<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool, meeting_id: &str, expected: Option<usize>) -> Result<MeetingSpeakers, String> {
    let (folder,): (Option<String>,) = sqlx::query_as("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Meeting not found")?;
    let folder = folder.filter(|f| !f.is_empty()).ok_or("This meeting has no recording folder.")?;
    let audio_path = meeting_audio(Path::new(&folder)).ok_or("This meeting's audio is no longer available (it may have been cleaned up).")?;

    let segments: Vec<SegmentTime> = sqlx::query_as::<_, (String, Option<f64>, Option<f64>)>(
        "SELECT id, audio_start_time, audio_end_time FROM transcripts WHERE meeting_id = ? ORDER BY COALESCE(audio_start_time, 0), rowid",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?
    .into_iter()
    .filter_map(|(id, s, e)| Some(SegmentTime { id, start: s?, end: e? }))
    .collect();
    if segments.is_empty() {
        return Err("This meeting's transcript has no timing information, so speakers can't be matched to it.".into());
    }

    let model = ensure_model(app, meeting_id).await?;
    let expected = expected.or(expected_from_calendar(pool, meeting_id).await);
    progress(app, meeting_id, "decode", 0);

    let app2 = app.clone();
    let mid = meeting_id.to_string();
    let segs = segments.clone();
    let (labels, speakers) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let decoded = crate::audio::decoder::decode_audio_file(&audio_path).map_err(|e| format!("Could not read the recording: {}", e))?;
        let audio = decoded.to_whisper_format();
        let mut guard = EMBEDDER.lock().unwrap();
        if guard.is_none() {
            *guard = Some(Embedder::load(&model)?);
        }
        let embedder = guard.as_mut().unwrap();
        analyze_audio(embedder, &audio, &segs, expected, |p| progress(&app2, &mid, "analyze", p))
    })
    .await
    .map_err(|e| e.to_string())??;

    // Recognise saved voices.
    let profiles = profiles_with_embeddings(pool).await;
    let centroids: Vec<Vec<f32>> = speakers.iter().map(|(c, _)| c.clone()).collect();
    let matches = match_profiles(&centroids, &profiles.iter().map(|(_, e)| e.clone()).collect::<Vec<_>>(), PROFILE_MATCH);

    // Keep names the user gave earlier when re-running (match old speakers by voice).
    let old: Vec<(Speaker, Vec<f32>)> = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, f64, Option<Vec<u8>>)>(
        "SELECT speaker_key, name, email, profile_id, seconds, embedding FROM meeting_speakers WHERE meeting_id = ?",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .filter_map(|(key, name, email, profile_id, seconds, emb)| Some((Speaker { key, name, email, profile_id, seconds }, from_blob(&emb?))))
    .collect();
    let old_matches = match_profiles(&centroids, &old.iter().map(|(_, e)| e.clone()).collect::<Vec<_>>(), 0.8);

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM meeting_speakers WHERE meeting_id = ?").bind(meeting_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    for (i, (c, secs)) in speakers.iter().enumerate() {
        let (mut name, mut email, mut profile_id) = (None, None, None);
        if let Some(p) = matches[i] {
            let (prof, _) = &profiles[p];
            name = Some(prof.name.clone());
            email = prof.email.clone();
            profile_id = Some(prof.id.clone());
        } else if let Some(o) = old_matches[i] {
            let (s, _) = &old[o];
            name = s.name.clone();
            email = s.email.clone();
        }
        sqlx::query(
            "INSERT INTO meeting_speakers (meeting_id, speaker_key, name, email, profile_id, embedding, seconds) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(meeting_id)
        .bind(speaker_key(i))
        .bind(name)
        .bind(email)
        .bind(profile_id)
        .bind(to_blob(c))
        .bind(*secs)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }
    for (seg, label) in segments.iter().zip(&labels) {
        sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ?")
            .bind(label.map(speaker_key))
            .bind(&seg.id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    info!("speakers: {} speaker(s) in {} segment(s)", speakers.len(), segments.len());
    progress(app, meeting_id, "done", 100);
    meeting_speakers_view(pool, meeting_id).await
}

// ---------------------------------------------------------------------------
// Commands: naming, merging, voiceprints
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn speakers_get(state: tauri::State<'_, AppState>, meeting_id: String) -> Result<MeetingSpeakers, String> {
    meeting_speakers_view(state.db_manager.pool(), &meeting_id).await
}

/// Names a speaker. With `remember`, saves (or updates) a voiceprint so the
/// person is recognised in future meetings; without it, any link to a saved
/// voiceprint is dropped for this meeting.
#[tauri::command]
pub async fn speakers_rename(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    key: String,
    name: Option<String>,
    email: Option<String>,
    remember: bool,
) -> Result<MeetingSpeakers, String> {
    rename(state.db_manager.pool(), meeting_id, key, name, email, remember).await
}

pub async fn rename(pool: &SqlitePool, meeting_id: String, key: String, name: Option<String>, email: Option<String>, remember: bool) -> Result<MeetingSpeakers, String> {
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    let email = email.map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty());
    let (emb, current_profile): (Option<Vec<u8>>, Option<String>) =
        sqlx::query_as("SELECT embedding, profile_id FROM meeting_speakers WHERE meeting_id = ? AND speaker_key = ?")
            .bind(&meeting_id)
            .bind(&key)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("Speaker not found")?;

    let mut profile_id = None;
    if remember {
        let label = name.clone().ok_or("Enter a name to recognise this voice.")?;
        let emb = from_blob(&emb.ok_or("No voice sample for this speaker.")?);
        let now = chrono::Utc::now().to_rfc3339();
        // Same person: the profile already linked, or one with this email / name.
        let existing: Option<(String, Vec<u8>, i64)> = sqlx::query_as(
            "SELECT id, embedding, samples FROM voice_profiles
             WHERE id = ? OR (? IS NOT NULL AND email = ?) OR LOWER(name) = LOWER(?) LIMIT 1",
        )
        .bind(current_profile.clone().unwrap_or_default())
        .bind(&email)
        .bind(&email)
        .bind(&label)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
        let id = match existing {
            Some((id, old, samples)) => {
                let v = if current_profile.as_deref() == Some(id.as_str()) { from_blob(&old) } else { blend(&from_blob(&old), samples, &emb) };
                let inc = if current_profile.as_deref() == Some(id.as_str()) { 0 } else { 1 };
                sqlx::query("UPDATE voice_profiles SET name = ?, email = COALESCE(?, email), embedding = ?, samples = samples + ?, updated_at = ? WHERE id = ?")
                    .bind(&label).bind(&email).bind(to_blob(&v)).bind(inc).bind(&now).bind(&id)
                    .execute(pool).await.map_err(|e| e.to_string())?;
                id
            }
            None => {
                let id = format!("voice-{}", uuid::Uuid::new_v4());
                sqlx::query("INSERT INTO voice_profiles (id, name, email, embedding, samples, created_at, updated_at) VALUES (?, ?, ?, ?, 1, ?, ?)")
                    .bind(&id).bind(&label).bind(&email).bind(to_blob(&emb)).bind(&now).bind(&now)
                    .execute(pool).await.map_err(|e| e.to_string())?;
                id
            }
        };
        profile_id = Some(id);
    }
    sqlx::query("UPDATE meeting_speakers SET name = ?, email = ?, profile_id = ? WHERE meeting_id = ? AND speaker_key = ?")
        .bind(&name)
        .bind(&email)
        .bind(&profile_id)
        .bind(&meeting_id)
        .bind(&key)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    meeting_speakers_view(pool, &meeting_id).await
}

/// Merges speaker `from` into `into` (when one person was split in two).
#[tauri::command]
pub async fn speakers_merge(state: tauri::State<'_, AppState>, meeting_id: String, from: String, into: String) -> Result<MeetingSpeakers, String> {
    merge(state.db_manager.pool(), meeting_id, from, into).await
}

pub async fn merge(pool: &SqlitePool, meeting_id: String, from: String, into: String) -> Result<MeetingSpeakers, String> {
    if from == into {
        return Err("Pick a different speaker to merge into.".into());
    }
    let rows: Vec<(String, Option<Vec<u8>>, f64)> =
        sqlx::query_as("SELECT speaker_key, embedding, seconds FROM meeting_speakers WHERE meeting_id = ? AND speaker_key IN (?, ?)")
            .bind(&meeting_id).bind(&from).bind(&into)
            .fetch_all(pool).await.map_err(|e| e.to_string())?;
    if rows.len() != 2 {
        return Err("Speaker not found".into());
    }
    let get = |k: &str| rows.iter().find(|r| r.0 == k).unwrap();
    let (a, b) = (get(&into), get(&from));
    let emb = match (&a.1, &b.1) {
        (Some(x), Some(y)) => {
            let (vx, vy) = (from_blob(x), from_blob(y));
            let (wx, wy) = (a.2.max(1.0) as f32, b.2.max(1.0) as f32);
            let mut v: Vec<f32> = vx.iter().zip(&vy).map(|(p, q)| p * wx + q * wy).collect();
            normalize(&mut v);
            Some(to_blob(&v))
        }
        (Some(x), None) => Some(x.clone()),
        (None, y) => y.clone(),
    };
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?").bind(&into).bind(&meeting_id).bind(&from).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE meeting_speakers SET embedding = ?, seconds = ? WHERE meeting_id = ? AND speaker_key = ?")
        .bind(emb).bind(a.2 + b.2).bind(&meeting_id).bind(&into).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM meeting_speakers WHERE meeting_id = ? AND speaker_key = ?").bind(&meeting_id).bind(&from).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    meeting_speakers_view(pool, &meeting_id).await
}

/// Assigns one transcript segment to a different speaker.
#[tauri::command]
pub async fn speakers_set_segment(state: tauri::State<'_, AppState>, meeting_id: String, transcript_id: String, key: String) -> Result<MeetingSpeakers, String> {
    let pool = state.db_manager.pool();
    sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ? AND meeting_id = ?").bind(&key).bind(&transcript_id).bind(&meeting_id).execute(pool).await.map_err(|e| e.to_string())?;
    meeting_speakers_view(pool, &meeting_id).await
}

#[tauri::command]
pub async fn voice_profiles_list(state: tauri::State<'_, AppState>) -> Result<Vec<VoiceProfile>, String> {
    let mut v: Vec<VoiceProfile> = profiles_with_embeddings(state.db_manager.pool()).await.into_iter().map(|(p, _)| p).collect();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(v)
}

/// Deletes one voiceprint (or all when `id` is None). Meetings keep the names.
#[tauri::command]
pub async fn voice_profiles_delete(state: tauri::State<'_, AppState>, id: Option<String>) -> Result<(), String> {
    delete_profiles(state.db_manager.pool(), id).await
}

pub async fn delete_profiles(pool: &SqlitePool, id: Option<String>) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    match &id {
        Some(id) => {
            sqlx::query("UPDATE meeting_speakers SET profile_id = NULL WHERE profile_id = ?").bind(id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
            sqlx::query("DELETE FROM voice_profiles WHERE id = ?").bind(id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        }
        None => {
            sqlx::query("UPDATE meeting_speakers SET profile_id = NULL").execute(&mut *tx).await.map_err(|e| e.to_string())?;
            sqlx::query("DELETE FROM voice_profiles").execute(&mut *tx).await.map_err(|e| e.to_string())?;
        }
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    if id.is_none() {
        warn!("speakers: all voiceprints deleted");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans() {
        assert_eq!(embed_span(0.0, 0.5, 100.0), None);
        assert_eq!(embed_span(2.0, 4.0, 100.0), Some((32_000, 64_000)));
        assert_eq!(embed_span(98.0, 105.0, 100.0), Some((1_568_000, 1_600_000)));
        let (a, b) = embed_span(0.0, 60.0, 100.0).unwrap();
        assert_eq!((b - a) as f64 / 16_000.0, MAX_SEGMENT_SECS);
        assert_eq!(a, 24 * 16_000);
    }

    #[test]
    fn fills_gaps() {
        let mut l = vec![None, Some(1), None, None, Some(0), None];
        fill_gaps(&mut l);
        assert_eq!(l, vec![Some(1), Some(1), Some(1), Some(1), Some(0), Some(0)]);
        let mut none: Vec<Option<usize>> = vec![None, None];
        fill_gaps(&mut none);
        assert_eq!(none, vec![None, None]);
    }

    #[test]
    fn blobs_and_blend() {
        let v = vec![0.6f32, 0.8, 0.0];
        assert_eq!(from_blob(&to_blob(&v)), v);
        let b = blend(&[1.0, 0.0, 0.0], 3, &[0.0, 1.0, 0.0]);
        assert!(b[0] > b[1] && (b.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-5);
        assert_eq!(speaker_key(0), "S1");
        let _ = cosine(&v, &v);
    }

    async fn db() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('m', 'T', 'x', 'x')").execute(&pool).await.unwrap();
        for (k, e, secs) in [("S1", vec![1.0f32, 0.0], 30.0), ("S2", vec![0.0f32, 1.0], 10.0)] {
            sqlx::query("INSERT INTO meeting_speakers (meeting_id, speaker_key, embedding, seconds) VALUES ('m', ?, ?, ?)")
                .bind(k).bind(to_blob(&e)).bind(secs).execute(&pool).await.unwrap();
        }
        for (id, k) in [("t1", "S1"), ("t2", "S2"), ("t3", "S2")] {
            sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, speaker) VALUES (?, 'm', 'x', '0', ?)")
                .bind(id).bind(k).execute(&pool).await.unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn rename_remember_and_forget() {
        let pool = db().await;
        // Name only: no voiceprint.
        let v = rename(&pool, "m".into(), "S1".into(), Some(" Priya ".into()), None, false).await.unwrap();
        assert_eq!(v.speakers[0].name.as_deref(), Some("Priya"));
        assert!(profiles_with_embeddings(&pool).await.is_empty());
        // Remember: creates a voiceprint and links it.
        let v = rename(&pool, "m".into(), "S1".into(), Some("Priya".into()), Some("P@W.com".into()), true).await.unwrap();
        let profs = profiles_with_embeddings(&pool).await;
        assert_eq!(profs.len(), 1);
        assert_eq!(profs[0].0.email.as_deref(), Some("p@w.com"));
        assert_eq!(v.speakers[0].profile_id.as_deref(), Some(profs[0].0.id.as_str()));
        // Saving again doesn't double-count the same meeting.
        rename(&pool, "m".into(), "S1".into(), Some("Priya".into()), Some("p@w.com".into()), true).await.unwrap();
        assert_eq!(profiles_with_embeddings(&pool).await[0].0.samples, 1);
        // Same person by email from another speaker blends into the profile.
        rename(&pool, "m".into(), "S2".into(), Some("Priya N".into()), Some("p@w.com".into()), true).await.unwrap();
        let profs = profiles_with_embeddings(&pool).await;
        assert_eq!((profs.len(), profs[0].0.samples, profs[0].0.name.as_str()), (1, 2, "Priya N"));
        assert!((profs[0].1[0] - profs[0].1[1]).abs() < 1e-5, "two samples blend evenly");
        // Remember without a name is refused.
        assert!(rename(&pool, "m".into(), "S2".into(), None, None, true).await.is_err());
        // Deleting voiceprints unlinks but keeps names.
        delete_profiles(&pool, None).await.unwrap();
        let v = meeting_speakers_view(&pool, "m").await.unwrap();
        assert!(v.speakers.iter().all(|s| s.profile_id.is_none()));
        assert_eq!(v.speakers[0].name.as_deref(), Some("Priya"));
        let names = speaker_names(&pool, "m").await;
        assert_eq!(names.get("S2").map(String::as_str), Some("Priya N"));
    }

    #[tokio::test]
    async fn merge_moves_segments_and_time() {
        let pool = db().await;
        assert!(merge(&pool, "m".into(), "S1".into(), "S1".into()).await.is_err());
        let v = merge(&pool, "m".into(), "S2".into(), "S1".into()).await.unwrap();
        assert_eq!(v.speakers.len(), 1);
        assert_eq!(v.speakers[0].seconds, 40.0);
        assert_eq!(v.assignments.iter().filter(|(_, k)| k == "S1").count(), 3);
        assert_eq!(speaker_names(&pool, "m").await.get("S1").map(String::as_str), Some("Speaker 1"));
    }

    /// End-to-end on a real 4-speaker recording with the real model. Run with
    /// MOTIV_SPK_MODEL=<model.onnx> MOTIV_SPK_WAV=<16k wav> cargo test -- --ignored speakers
    #[test]
    #[ignore]
    fn four_speaker_recording() {
        let model = std::env::var("MOTIV_SPK_MODEL").expect("MOTIV_SPK_MODEL");
        let wav = std::env::var("MOTIV_SPK_WAV").expect("MOTIV_SPK_WAV");
        let decoded = crate::audio::decoder::decode_audio_file(Path::new(&wav)).unwrap();
        let audio = decoded.to_whisper_format();
        // Fake transcript: 2 s segments every 1.5 s where there is sound.
        let mut segs = Vec::new();
        let mut t = 0.0;
        while t + 2.0 < audio.len() as f64 / 16_000.0 {
            let (a, b) = ((t * 16_000.0) as usize, ((t + 2.0) * 16_000.0) as usize);
            let rms = (audio[a..b].iter().map(|x| x * x).sum::<f32>() / (b - a) as f32).sqrt();
            if rms > 0.01 {
                segs.push(SegmentTime { id: format!("{}", t), start: t, end: t + 2.0 });
            }
            t += 1.5;
        }
        let mut e = Embedder::load(Path::new(&model)).unwrap();
        let (labels, speakers) = analyze_audio(&mut e, &audio, &segs, None, |_| {}).unwrap();
        let s: String = labels.iter().map(|l| l.map_or('-', |v| char::from(b'0' + v as u8))).collect();
        println!("speakers={} labels={}", speakers.len(), s);
        assert!((3..=5).contains(&speakers.len()), "found {} speakers", speakers.len());
        let (_, forced) = analyze_audio(&mut e, &audio, &segs, Some(4), |_| {}).unwrap();
        assert_eq!(forced.len(), 4);
    }
}

-- Motiv: speaker identification.
-- transcripts.speaker (added upstream, unused) now holds a speaker key such as 'S1'.

-- Speakers found in a meeting.
CREATE TABLE IF NOT EXISTS meeting_speakers (
    meeting_id TEXT NOT NULL,
    speaker_key TEXT NOT NULL,
    name TEXT,
    email TEXT,
    -- voice_profiles.id when recognised or remembered
    profile_id TEXT,
    -- centroid embedding, little-endian f32
    embedding BLOB,
    seconds REAL NOT NULL DEFAULT 0,
    PRIMARY KEY (meeting_id, speaker_key),
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

-- Opt-in voiceprints: created only when the user ticks "Recognise this voice".
CREATE TABLE IF NOT EXISTS voice_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT,
    embedding BLOB NOT NULL,
    -- how many meetings contributed (running average weight)
    samples INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

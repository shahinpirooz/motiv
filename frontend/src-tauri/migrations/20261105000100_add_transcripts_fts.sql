-- Motiv: full-text search over transcripts (SQLite FTS5).
-- A standalone FTS table keyed by transcript id, kept in sync by triggers.
-- (External-content FTS would rely on the transcripts table's implicit rowid,
-- which VACUUM may renumber.)
CREATE VIRTUAL TABLE IF NOT EXISTS transcripts_fts USING fts5(
    transcript,
    transcript_id UNINDEXED,
    meeting_id UNINDEXED,
    tokenize = 'porter unicode61 remove_diacritics 2'
);

INSERT INTO transcripts_fts (transcript, transcript_id, meeting_id)
SELECT transcript, id, meeting_id FROM transcripts;

CREATE TRIGGER IF NOT EXISTS transcripts_fts_ai AFTER INSERT ON transcripts BEGIN
    INSERT INTO transcripts_fts (transcript, transcript_id, meeting_id)
    VALUES (new.transcript, new.id, new.meeting_id);
END;

CREATE TRIGGER IF NOT EXISTS transcripts_fts_ad AFTER DELETE ON transcripts BEGIN
    DELETE FROM transcripts_fts WHERE transcript_id = old.id;
END;

CREATE TRIGGER IF NOT EXISTS transcripts_fts_au AFTER UPDATE OF transcript, meeting_id ON transcripts BEGIN
    DELETE FROM transcripts_fts WHERE transcript_id = old.id;
    INSERT INTO transcripts_fts (transcript, transcript_id, meeting_id)
    VALUES (new.transcript, new.id, new.meeting_id);
END;

-- Motiv: structured action items, decisions, highlights and open questions
-- extracted from a meeting by the LLM. One JSON document per meeting.
CREATE TABLE IF NOT EXISTS meeting_insights (
    meeting_id TEXT PRIMARY KEY,
    insights TEXT NOT NULL,
    model TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

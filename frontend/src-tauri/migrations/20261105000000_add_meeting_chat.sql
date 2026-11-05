-- Motiv: chat with a meeting.
-- One row per chat message; deleted with the meeting.
CREATE TABLE IF NOT EXISTS meeting_chat_messages (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_meeting_chat_messages_meeting
    ON meeting_chat_messages (meeting_id, created_at);

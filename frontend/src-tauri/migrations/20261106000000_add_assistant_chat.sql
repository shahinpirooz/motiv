-- Motiv: the app-wide assistant chat (questions across all meetings).
CREATE TABLE IF NOT EXISTS assistant_chat_messages (
    id TEXT PRIMARY KEY,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content TEXT NOT NULL,
    -- JSON array of {meeting_id, title, date} the answer drew on
    sources TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL
);

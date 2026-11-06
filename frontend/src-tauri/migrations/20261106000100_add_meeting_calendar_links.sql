-- Motiv: which calendar event a recorded meeting belongs to, with its
-- invited attendees and (Teams) who actually attended.
CREATE TABLE IF NOT EXISTS meeting_calendar_links (
    meeting_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    event_id TEXT NOT NULL,
    -- JSON CalendarEvent snapshot (title, times, organizer, attendees, join URL)
    event TEXT NOT NULL,
    -- JSON list of attendance records, when available
    attendance TEXT,
    -- 'auto' (matched by time) or 'manual' (picked by the user)
    source TEXT NOT NULL DEFAULT 'auto',
    updated_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

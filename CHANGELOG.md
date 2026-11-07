# Changelog

Motiv versions. Each feature release bumps the minor version
(`scripts/fork/bump-version.sh X.Y.Z` updates every version field, commits
and tags). Fixes bump the patch version.

## 0.6.0 (2026-11-06)

- **Assistant pane:** chat on the right of every screen, about the open meeting or across all meetings, with links to the meetings it drew on (Cmd/Ctrl+J).
- **Search across meetings:** full-text search (SQLite FTS5) with stemming, prefix matching, phrases and ranking.
- **Action items:** action items with owners and due dates, decisions, highlights and open questions; tick items off; copy as markdown.
- **Meeting awareness:** notices Zoom, Teams and Google Meet calls and asks to start or stop recording (or does it automatically).
- **Calendars:** connect Microsoft 365 and Google; recordings are named after the calendar event; People tab with invitees and Teams attendance; participant names in summaries, chat and action items.
- **Names & terms:** correct spellings of names and jargon for summaries, chat and Whisper.
- **Storage:** choose the recordings folder (including synced cloud folders) and automatic cleanup of old audio by age or total size. Fixes the saved folder being ignored.
- **Motiv identity:** new app, tray and installer icons; About page; meeting popup redesign; upstream analytics disconnected.

## 0.5.0 (2026-11-05)

- Split from Meetily Community Edition v0.5.0 (MIT) as Motiv.
- Copy summary as rich text or markdown, and send it by email.

# Changelog

Motiv versions. Each feature release bumps the minor version
(`scripts/fork/bump-version.sh X.Y.Z` updates every version field, commits
and tags). Fixes bump the patch version.

## 0.7.1 (2026-11-06)

- **Check for Updates** works: it compares against the latest release on GitHub and links to its download page (or says none are published yet), instead of failing with "Unknown error".
- Removed Meetily's screenshots, logos, docs and legacy backend; README, privacy policy and guides now describe Motiv. `MOTIV_DEBUG_LOG` / `MOTIV_LLAMA_HELPER` replace the `MEETILY_` variables (old names still work).

## 0.7.0 (2026-11-06)

- **Speaker identification:** People tab > Identify speakers labels who said what (on-device model, downloaded once). Name speakers from the meeting's participants, merge split speakers, and the transcript, chat, action items and summaries use the names.
- **Voiceprints (opt-in):** tick "Recognise this voice" to have Motiv name that person automatically in later meetings. Stored only on this computer; manage or delete them in Settings > Meetings.

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

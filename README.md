<p align="center">
  <img src="frontend/design/motiv-logo-1024.png" width="128" alt="Motiv">
</p>

<h1 align="center">Motiv</h1>

<p align="center">A private meeting assistant for macOS and Windows: record, transcribe, summarise and ask questions about your meetings, on your own machine.</p>

---

## What it does

- **Records and transcribes locally** with Whisper or Parakeet. Audio and transcription never leave your computer.
- **Summaries** with any model: local (Ollama or the built-in engine) or cloud (Claude, OpenAI, Groq, OpenRouter, any OpenAI-compatible endpoint). Only transcript text is sent, and only to the provider you pick.
- **Assistant pane** (Cmd/Ctrl+J): ask about the open meeting or across all meetings, with links to the meetings it drew on.
- **Action items, decisions, highlights and open questions**, with a checklist and copy as markdown.
- **Search across meetings** (full-text, ranked).
- **Speaker identification**: who said what, named from the meeting's participants, with opt-in voiceprints to recognise people in later meetings.
- **Meeting awareness**: notices Zoom, Teams and Google Meet calls and offers to record (or does it automatically), and to stop when the call ends.
- **Calendars**: connect Microsoft 365 and Google to name recordings after the meeting and see invitees and Teams attendance.
- **Names & terms**: your spellings of names and jargon, used in summaries, chat and Whisper.
- **Rich export**: copy summaries as rich text or markdown, or send them by email.
- **Storage control**: choose where recordings go (including synced cloud folders) and expire old audio by age or total size.

See [CHANGELOG.md](CHANGELOG.md) for what's in each version.

## Privacy

Motiv has no account and no server. Recordings, transcripts, summaries, voiceprints and calendar sign-ins stay on your computer. See [PRIVACY_POLICY.md](PRIVACY_POLICY.md).

## Install

There are no published installers yet; build from source (below).

## Build from source

Prerequisites: Node.js 20+, pnpm 9.15.9, Rust (https://rustup.rs), CMake, and LLVM/Clang 19 or older. Platform details are in [docs/BUILDING.md](docs/BUILDING.md); GPU options in [docs/GPU_ACCELERATION.md](docs/GPU_ACCELERATION.md).

**macOS / Linux**

```bash
git clone https://github.com/shahinpirooz/motiv.git
cd motiv
scripts/fork/app.sh dev      # run in development mode
scripts/fork/app.sh build    # installable build in target/release/bundle/
```

**Windows** (Command Prompt or PowerShell, with the Visual Studio C++ Build Tools installed)

```powershell
git clone https://github.com/shahinpirooz/motiv.git
cd motiv
scripts\fork\app.cmd dev
scripts\fork\app.cmd build
```

The scripts build the llama-helper sidecar the built-in summary engine needs. The first build takes 20 to 40 minutes.

**Calendars** need app registrations for Microsoft and Google, set up once by whoever builds Motiv. See [docs/CALENDAR_SETUP.md](docs/CALENDAR_SETUP.md).

## Tests

```bash
scripts/fork/test-fork.sh           # frontend tests + TypeScript
scripts/fork/test-fork.sh --rust    # also the Rust tests that need a full app build
cd frontend/src-tauri && cargo test --lib
```

## Releasing a version

Add a section to [CHANGELOG.md](CHANGELOG.md), then:

```bash
scripts/fork/bump-version.sh 0.8.0
git push origin main --tags
```

## Project layout

| Path | What |
| --- | --- |
| `frontend/src` | Next.js / React UI |
| `frontend/src-tauri/src` | Rust core: audio, transcription, summaries, database |
| `frontend/src-tauri/src/{chat,insights,meeting_watch,storage,vocabulary,accounts,calendar,speakers}` | Motiv features |
| `frontend/src-tauri/migrations` | SQLite schema |
| `frontend/design` | Logo and tray icon sources (SVG) |
| `llama-helper` | Sidecar for the built-in local summary model |
| `docs` | Build, architecture and setup guides |

## Credits

Motiv began as a fork of [Meetily](https://github.com/Zackriya-Solutions/meetily) Community Edition v0.5.0 by Zackriya Solutions, released under the MIT License, and has been developed independently since. See [LICENSE.md](LICENSE.md).

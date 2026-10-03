# Changelog

All notable changes to **Oczi** are recorded here.

## 0.1.2 — 2026-10-03

First release under its own name. Oczi is a Windows fork of Coucou: the code is
MIT, the name, the character and the icon are not (see LICENSE-ASSETS.md).

**The character**

- Iskra replaces the original mascot: a crystal drawn from data — a parametric
  hull, a palette, two eyes and three shards orbiting her — in `src/bot/skin.ts`,
  with the engine reading that description instead of hard-coded shapes. The app
  icon is generated from the same geometry by `npm run icons`, so the two cannot
  drift apart.

**The chat**

- The chat looks things up instead of answering from memory: the model gets a web
  search tool and a page reader, and sources come back in the answer. Settings →
  Internet has an off switch and the backend choice (keyless DuckDuckGo by
  default, Brave or Tavily with a key). Today's date goes into the prompt so the
  model stops guessing the year.
- The chat has hands. Settings → Terminal is off by default and warns before it
  turns on; once on, `run_terminal` runs a command and waits for it, `launch_app`
  opens a program or a file by name, and `terminal_job` / `terminal_output` /
  `terminal_kill` handle long work in the background. Every command is logged,
  commands are killed when they overrun, and nothing of ours outlives the app.
- Runs against the DeepSeek API (with a Thinking switch) instead of the Claude API.
- Added the "eye": a region screenshot pinned to the next question.

**The app**

- English by default, Polish one setting away. `npm run check:i18n` keeps the two
  tables in step and fails the build when Polish text is left in a UI file.
- File drag-and-drop onto the island goes through the app's own drop target, so
  files land reliably on Windows.
- Survives a crash inside WebView2: the reason and exit code go to the log and
  Oczi restarts itself instead of showing the runtime's error page. Three
  restarts in ten minutes and it gives up, saying so in the tray tooltip.
- Lighter when nobody is looking: the frame loop stops while the island is tucked
  away (~1 % of a core), runs at 30 fps for an open card nobody is touching, 15 fps
  for the small bar, and at full rate the moment the cursor is on the island or
  the chat is in use.

**Removed**

- The macOS/Swift front end and every Linux/macOS build step: the repo is
  Windows-only now.
- The Claude Code integration (session ticker, permission approval, hook relay)
  entirely.

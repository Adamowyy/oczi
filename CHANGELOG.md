# Changelog

All notable changes to **Oczi** are recorded here.

## 0.1.4 — 2026-10-04

- **Settings → Island lives on** now lists every display, numbered the way Windows
  numbers it, so the island can be pinned to one specific monitor instead of only
  the main one or the one under the cursor. The pick is stored under the display's
  device name, which outlives a restart; a monitor that is unplugged later falls
  back to the main one, and the picker says so instead of silently moving the
  island somewhere else. The list is re-read every time the settings window is
  opened, so a display plugged in after launch still shows up.
- After an update the island says what changed: a short card with the notable
  changes for the version that just started, shown once, and only after the
  greeting has had its turn. A fresh install is not offered one — there is nothing
  to catch up on.
- That same card reports a newer release. It costs one anonymous call to the GitHub
  API per launch, waits at most six seconds for it, and says nothing at all when
  there is no connection or no release to compare against. It is the only request
  the app makes on its own, and it does not repeat while Oczi sits in the
  background.

## 0.1.3 — 2026-10-04

- Coming back after a long idle no longer shows her as a giant square, or as nothing
  at all: the first frame could arrive carrying the clock of the last frame before the
  sleep, and the step it produced ran the day backwards. The frame loop now keeps its
  own clock, so the little Iskras beside the pills keep their shape too.

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
  tables in step and fails the build when Polish text is left in a UI file, and
  switching the language re-loads the island so every view is re-translated. The
  remembered choice is applied before the first view is built — the island builds
  its home view while the boot reply is still on its way, so without that the home
  page came up English on every start.
- File drag-and-drop onto the island goes through the app's own drop target, so
  files land reliably on Windows.
- Survives a crash inside WebView2: the reason and exit code go to the log and
  Oczi restarts itself instead of showing the runtime's error page. Three
  restarts in ten minutes and it gives up, saying so in the tray tooltip.
- Lighter when nobody is looking: the frame loop stops while the island is tucked
  away (~1 % of a core), runs at 30 fps for an open card nobody is touching, 15 fps
  for the small bar, and at full rate the moment the cursor is on the island or
  the chat is in use.
- Coming back from the top edge, the bar arrives before the character in it:
  Iskra overhangs her bar by design, and used to be on screen while it was still a
  sliver.
- The drop card's two buttons work: the sequence's card is painted on a canvas, and
  the invisible card of ordinary views used to lie on top of it and swallow the
  click — *Ask about it* only worked where the two happened to line up, and *Cancel*
  never did.
- How long the small bar waits before it hides is a setting of its own, next to the
  one for the big card, instead of a fixed sixty seconds.
- The island can be dragged along the top of the screen: press and hold her body
  (not a control) and she follows, and where she is left is remembered as a
  fraction of the room she has, so it survives a restart and another resolution.
- A link in an answer is a link: the chat is plain text, so addresses were just grey
  words you could not click. They are found in the text, underlined in the accent
  colour, and open in your own browser.
- An answer can carry a little structure: **bold** for key words, `code` for commands
  and paths, and plain bullet or numbered lists. The model is told to keep it short
  and to use nothing else — no headings, no tables, the card is too small for them.
- Iskra stays inside the card while it changes shape. She is drawn outside the
  card's clip on purpose — she overhangs the bar — so while a card grew or shrank
  she hung off its corner; the silhouette holds everything in until it settles.
- The big light-blue square that flashed past the window edge is gone: it was Iskra
  travelling from the bar to her place in the card, with her glow lighting up before
  the card had finished growing.
- No more flash of a big stretched square when she wakes: the sizes and places the
  frame loop caches are dropped the moment the island goes to sleep, so the first
  frame back re-asserts them instead of painting what the DOM still carried.

- Reading a page no longer takes the app down. Two places cut the page text by
  byte count instead of by character, so a page with an accent in the wrong spot
  panicked — and a release build aborts on a panic. A panic now also writes its
  own line to the log, with the file and line number.

**Removed**

- The macOS/Swift front end and every Linux/macOS build step: the repo is
  Windows-only now.
- The Claude Code integration (session ticker, permission approval, hook relay)
  entirely.

# Changelog

All notable changes to **Oczi** are recorded here.

## 0.3.0 — 2026-10-10

- Dictation, in the installer whose name says voice: press its shortcut, speak, and
  the sentence goes to the chat by itself. The words are written down on this
  computer, with no key, no account and nothing sent anywhere to be transcribed.
- Oczi listens again once it has answered, so a conversation can be held by voice:
  speak, read the answer, speak again. The microphone becomes a red circle while it
  is recording, and an amber one while the words are being written.
- The dictation shortcut and the language it listens for are in the settings next to
  the other shortcuts. The ordinary installer shows them too, and says that the
  engine is not in that one.
- The model and the libraries the engine needs travel inside the voice installer,
  which is why that one is a few hundred megabytes instead of two.
- The terminal tool no longer waits for a program it started, so opening a browser
  answers instead of leaving the answer stuck.
- A frame that throws takes nothing with it: the island keeps drawing instead of
  freezing on the last thing it painted.
- Clicking elsewhere closes the island while an answer is being written, and a turn
  that never comes back gives up after two minutes rather than leaving the dots
  running.

## 0.2.0 — 2026-10-08

- Oczi sets reminders itself now: ask for one in the chat and a card comes up at
  the right moment, with **OK** and **+10 min**. Nothing is installed for it and
  nothing is written outside `%APPDATA%\Oczi`: no scripts, no scheduled tasks.
- A reminder can repeat: every day, on working days, or once a week at the same
  time. The card says which.
- A reminder that came due while Oczi was closed is shown as **missed**, and it
  offers no **+10 min**. Ten more minutes of a day that has gone makes no sense. One
  left unanswered for half an hour turns into **missed** where it stands, even when
  it came up while Oczi was running.
- A reminder that was never answered comes back on the next launch, so a card lost
  to a shutdown or a restart is not lost with it. Asking what is planned in a new
  conversation lists the same reminders.
- Clicking somewhere else hides a reminder card without dismissing it: it is still
  there the next time the island is opened, and only **OK** or **+10 min** end it.
  The first three seconds ignore that click, so one already on its way when the card
  appears cannot take it away.
- An empty chat shows what Oczi can do, one line at a time: set a reminder, ask
  about a page, snip the screen, ask what is planned today, or turn the terminal on
  and hand it work on your computer.
- The model is told to set reminders rather than build them out of scripts, and it
  is told to set a repeating one when that is what was asked for.
- Oczi knows which processes in Task Manager are its own: its windows are drawn by
  Edge WebView2, so the `msedgewebview2.exe` processes running under Oczi belong to
  Oczi — closing one closes its window.
- A question about autostart is answered from a real read of the machine: the Run
  keys for the user and the machine, the Startup folders, and the services set to
  start on their own, each with the Task Manager switch and whether the program is
  still on the disk. An entry an uninstalled program left behind is called a leftover
  instead of being listed as something that loads at logon. `oczi.exe --startup`
  prints that same read.

## 0.1.7 — 2026-10-06

- You can pick both keyboard shortcuts yourself now: in Settings → General, click
  the field and press the combination you like, one for the island and one for the
  screenshot. A key on its own is turned down, because Oczi would then take it
  everywhere on the desktop.
- New pill **PC**: processor, memory, disk space and battery, read on your own
  machine. No key, no account, and it says nothing about a battery if you have none.
  Under the bars it lists the three processes holding the most memory, added up per
  executable name so a browser with a dozen helpers reads as one row. Windows' own
  plumbing — the shell and its hosts, the services, Defender, the update machinery —
  and the WebView2 helpers stay out of that list: nobody can act on them, and what
  is left is what a person can actually close.
- New pill **Music**: whatever Windows says is playing, so Spotify, a browser tab
  and VLC all work. No login, no key. Track, artist, a progress bar, play/pause and
  skipping.
- While music is playing, its pill becomes the active one: the island opens on the
  player and the chat field waits underneath.
- Clicking the island opens the home screen. The summon shortcut still opens the
  chat, with the cursor in the field.
- A fresh install starts with **PC** and **Music** switched on, the two that need no
  key and no account. Every service that asks for one stays off until it is picked.
- The compact bar shows every service that is switched on.
- A new track no longer makes the island peek: the player updates quietly, and the
  island still opens on the player when it is clicked.
- A card no longer counts down to close while the cursor is on the island.
- A long track title no longer widens the player until the service column beside it
  is clipped: the player's rows shrink and ellipsise instead.
- The countdown buttons inside the island save only the two numbers they change,
  instead of writing the whole settings file back. A page holding an older copy can
  no longer put its own values over the chords you recorded.
- "New chat" is a labelled button, instead of a plus that looked like adding a file.

## 0.1.6 — 2026-10-06

- The card that appears after an update carries the notes for this build. 0.1.5
  shipped without them, so updating to it passed in silence; nothing else changed.

## 0.1.5 — 2026-10-06

- Answers read as prose again. A bullet's text was laid out inside a row of columns:
  every bold word, every fragment of a sentence became its own column, so one line
  came out scattered across the card, with words flung to the edge and sentences cut
  in half. The text of a list item is one block now, long paths wrap inside the card
  instead of widening it, and rows sit a little closer.
- Markdown the model sends anyway is folded into the few shapes the card shows well:
  a table becomes one line per row, a heading a bold line, a fenced block a small
  monospace panel, and rules and quotes are dropped.
- Answers are shorter: two to four lines, with more only when the question asks for
  steps or detail, and no preamble, repetition or closing offer of help.
- The model finally sees its own replies. They were missing from the conversation,
  so it read a stack of its own unanswered questions and answered several at once,
  repeated what it had already said, or took an older question for the current one.

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

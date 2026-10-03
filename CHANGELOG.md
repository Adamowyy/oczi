# Changelog

All notable changes to **Oczi** are recorded here.

## Unreleased

- The chat can look things up instead of answering from memory: the model gets a
  web search tool and a page reader, and it runs them in the app when a question
  depends on something newer than its training data. Sources come back in the
  answer, and the settings window has an Internet section with an off switch, the
  backend choice (keyless DuckDuckGo by default, Brave or Tavily with a key) and
  today's date is put in the prompt so the model stops guessing the year.
- The app now survives a crash inside WebView2: the failure is written to the log
  with the runtime's reason and exit code, and Oczi restarts itself instead of
  leaving the island showing WebView2's error page. Gives up after three restarts
  in ten minutes and says so in the tray tooltip.
- Removed the macOS/Swift front end and every Linux/macOS build step — the repo is now Windows-only.
- Chat runs against the DeepSeek API (with a Thinking switch) instead of the Claude API.
- Added the "eye": a built-in region screenshot tool that pins the shot to the next question.
- File drag-and-drop onto the island is handled by the app's own drop target, so files land reliably on Windows.
- Dropped the Claude Code integration (session ticker, permission approval, hook relay) entirely.

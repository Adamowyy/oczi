<div align="center">

# Oczi for Windows

**Iskra — a tiny crystal that lives at the top of your screen.**

Drop a file, chat through the DeepSeek API, snap a region screenshot — all without leaving what you're doing.

![Windows 10/11](https://img.shields.io/badge/Windows-10%2F11-0078D4?logo=windows)
![Tauri 2](https://img.shields.io/badge/Tauri-2-FFC131?logo=tauri&logoColor=black)
![Rust](https://img.shields.io/badge/Rust-backend-000?logo=rust)

<img src="docs/island-card.jpg" alt="The Oczi card: Iskra on the left, her service pills on the right, and the chat field below" width="640">

</div>

---

## Screenshots

<img src="docs/island-drop.jpg" alt="The drop view: Drop files here, with PDF, Images, Code and Docs" width="640">

*Drop a file on her and she takes it — the original is copied into an inbox, never touched.*

<img src="docs/settings-terminal.jpg" alt="Settings — Terminal, on, with the warning it shows" width="430">

*Terminal access is off by default, and says what it will do before it does it.*

<img src="docs/settings-integrations.jpg" alt="Settings — integrations, keys masked" width="430">

*Integrations: each service gets a pill with its own little character. Keys never leave the Credential Manager.*

## Install

Download **`Oczi-Windows-0.1.4-setup.exe`** from
[Releases](https://github.com/Adamowyy/oczi/releases/latest) and run it. Windows
10 or 11; WebView2 is already there on a normal install — the installer offers it
if it is missing.

The installer is **not code-signed**, so Windows may show a SmartScreen warning
(*More info → Run anyway*), and Defender has once flagged it as
`Trojan:Win32/WacatacH!ml` — a false positive from an unsigned NSIS installer,
reported to Microsoft. Build it yourself from source if that bothers you; the
whole thing is in this repository.

Oczi asks for a DeepSeek API key on first run (Settings → DeepSeek). Without one
it can still sit there and look pretty, but it cannot answer anything.

## What it does

Iskra is a soft little crystal with three shards orbiting her, living at the top centre of the screen. She waves hello, follows your cursor with her eyes, gets annoyed when you poke her (and dizzy if you insist), and turns into a box to swallow a file you drop on her.

- 📎 **Drop a file** — Iskra swallows it, then offers to answer questions about it. Dropped files are copied into an inbox so the original is never touched.
- 💬 **Chat** — ask anything; the answer runs against the OpenAI-compatible DeepSeek API, in Rust, so the key and the file bytes never reach the web view. Pick the model (`deepseek-flash` or `deepseek-v4-pro`) and a **Thinking** switch in Settings.
- 🖱️ **The eye** — snap a region of the screen and pin it to the next question, like a screenshot you don't have to paste anywhere.
- 🔌 **Integrations** — Stripe, n8n, GitHub, Vercel, Resend, Notion, Cal.com. Each one gets its own coloured mini character pill.
- 🫥 **Invisible when idle** — hides into the top edge and peeks out when you hover it. `Ctrl+Alt+M` summons it from anywhere; `Ctrl+Alt+Shift+S` starts a snip.
- 🔒 **Private by design** — no telemetry, no account. Keys live in the Windows Credential Manager. The app only talks to the services you plug in.
- 🌍 **English by default** — Polish is one setting away; another language is one table in `src/core/i18n.ts`.
- 🖥️ **Terminal, off by default** — turn it on and the chat can run commands on your PC, in the background too. It warns you first, and every command lands in the app log.

See [`windows/README.md`](windows/README.md) for how everything works under the hood.

## Build from source

You need [Rust](https://rustup.rs), [Node 20+](https://nodejs.org), and the **MSVC build tools** (or the GNU toolchain — see the Windows readme).

```powershell
cd windows
npm install
npm run tauri dev      # live-reloading development build
npm run pack           # installer lands in windows/release/
```

## License

The **code** is [MIT](LICENSE): © Louis Raillé for the Coucou base this fork is
built on, © Adam Warzecha for the Oczi changes. The **Oczi name, the Iskra
character and the app icon** are not covered by it — see
[LICENSE-ASSETS.md](LICENSE-ASSETS.md). Forking to ship your own app is welcome;
give it your own name, icon and character.

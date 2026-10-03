<div align="center">

# Oczi for Windows

**Iskra — a tiny crystal that lives at the top of your screen.**

Drop a file, chat through the DeepSeek API, snap a region screenshot — all without leaving what you're doing.

![Windows 10/11](https://img.shields.io/badge/Windows-10%2F11-0078D4?logo=windows)
![Tauri 2](https://img.shields.io/badge/Tauri-2-FFC131?logo=tauri&logoColor=black)
![Rust](https://img.shields.io/badge/Rust-backend-000?logo=rust)

</div>

---

## What it does

Iskra is a soft little crystal with three shards orbiting her, living at the top centre of the screen. She waves hello, follows your cursor with her eyes, gets annoyed when you poke her (and dizzy if you insist), and turns into a box to swallow a file you drop on her.

- 📎 **Drop a file** — Iskra swallows it, then offers to answer questions about it. Dropped files are copied into an inbox so the original is never touched.
- 💬 **Chat** — ask anything; the answer runs against the OpenAI-compatible DeepSeek API, in Rust, so the key and the file bytes never reach the web view. Pick the model (`deepseek-flash` or `deepseek-v4-pro`) and a **Thinking** switch in Settings.
- 🖱️ **The eye** — snap a region of the screen and pin it to the next question, like a screenshot you don't have to paste anywhere.
- 🔌 **Integrations** — Stripe, n8n, GitHub, Vercel, Resend, Notion, Cal.com. Each one gets its own coloured mini character pill.
- 🫥 **Invisible when idle** — hides into the top edge and peeks out when you hover it. `Ctrl+Alt+M` summons it from anywhere; `Ctrl+Alt+Shift+S` starts a snip.
- 🔒 **Private by design** — no telemetry, no account. Keys live in the Windows Credential Manager. The app only talks to the services you plug in.
- 🌍 **English by default** — Polish is one setting away; another language is one table in `src/core/i18n.ts`.

See [`windows/README.md`](windows/README.md) for screenshots and the full details.

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

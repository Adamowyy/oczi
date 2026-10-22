<div align="center">

# Oczi for Windows

**Oczi — a tiny crystal that lives at the top of your screen.**

Drop a file, chat through DeepSeek or Gemini — the Gemini key is free — snap a region screenshot, all without leaving what you're doing.

![Windows 10/11](https://img.shields.io/badge/Windows-10%2F11-0078D4?logo=windows)
![Tauri 2](https://img.shields.io/badge/Tauri-2-FFC131?logo=tauri&logoColor=black)
![Rust](https://img.shields.io/badge/Rust-backend-000?logo=rust)

<img src="docs/island-card.jpg" alt="The Oczi card: Oczi on the left, her service pills on the right, and the chat field below" width="640">

</div>

---

## Screenshots

<table>
<tr>
<td width="50%" valign="top"><img src="docs/integration-pc.jpg" alt="The PC pill: processor, memory, free disk space and the three processes holding the most memory" width="100%"><br>
<em>PC — the machine's own numbers, and the three processes holding the most memory. No key, no account, nothing leaving the computer.</em></td>
<td width="50%" valign="top"><img src="docs/integration-music.jpg" alt="The Music pill: the track Windows is playing, with a progress bar and transport buttons" width="100%"><br>
<em>Music — whatever Windows says is playing, Spotify or a browser tab, with a progress bar, play/pause and skip.</em></td>
</tr>
</table>

<img src="docs/integration-games.jpg" alt="The Games card: a title search, the cover, the cheapest key beside Steam's price, and both record lows" width="640">

*Games — a title in, and the cheapest key right now beside what Steam charges, with both record lows. Prices from GG.deals on your own key, artwork from Steam.*

<img src="docs/reminder-card.jpg" alt="A reminder card: Reminder, what to remember, when it is due, and the +10 min and OK buttons" width="640">

*A reminder comes up as a card of its own: what to remember, when it is due, and the two answers.*

<img src="docs/reminder-chat.jpg" alt="The chat: asking which reminder is the closest one, then taking it off the list" width="640">

*Ask what is planned, or take one off again. The chat and the cards are the same list.*

<img src="docs/island-drop.jpg" alt="The drop view: Drop files here, with PDF, Images, Code and Docs" width="640">

*Drop a file on her and she takes it — the original is copied into an inbox, never touched.*

<table>
<tr>
<td width="50%" valign="top"><img src="docs/settings-integrations.jpg" alt="Settings — integrations, keys masked" width="100%"><br>
<em>Integrations: each service gets a pill with its own little character. Keys never leave the Credential Manager.</em></td>
<td width="50%" valign="top"><img src="docs/settings-terminal.jpg" alt="Settings — Terminal, on, with the warning it shows" width="100%"><br>
<em>Terminal access is off by default, and says what it will do before it does it.</em></td>
</tr>
</table>

## Install

Download **`Oczi-Windows-0.5.1-setup.exe`** from
[Releases](https://github.com/Adamowyy/oczi/releases/latest) and run it. Windows
10 or 11; WebView2 is already there on a normal install — the installer offers it
if it is missing.

The installer is **not code-signed**, so Windows may show a SmartScreen warning
(*More info → Run anyway*), and Defender has once flagged it as
`Trojan:Win32/WacatacH!ml` — a false positive from an unsigned NSIS installer,
reported to Microsoft. Build it yourself from source if that bothers you; the
whole thing is in this repository.

Oczi asks for a key on first run (Settings → Chat), and it asks you to pick the provider.
**Gemini** is the free one: a key from [Google AI Studio](https://aistudio.google.com/apikey)
costs nothing, no card and no billing, and the free tier carries everyday use. **DeepSeek** is
paid per use. Without a key Oczi can still sit there and look pretty, but she cannot answer
anything.

## What it does

Oczi is a soft little crystal with three shards orbiting her, living at the top centre of the screen. She waves hello, follows your cursor with her eyes, gets annoyed when you poke her (and dizzy if you insist), and turns into a box to swallow a file you drop on her.

- 📎 **Drop a file** — Oczi swallows it, then offers to answer questions about it. Dropped files are copied into an inbox so the original is never touched.
- 💬 **Chat** — ask anything; the answer runs against an OpenAI-compatible endpoint (DeepSeek or Gemini, both with your own key), in Rust, so the key and the file bytes never reach the web view. Pick the provider and the model in Settings → Chat; the Gemini key is free, and DeepSeek adds a **Thinking** switch.
- ⏰ **Reminders** — ask for one in the chat and a card comes up at the right moment, with **OK** and **+10 min**. They can repeat every day, on working days or once a week, and they live in a file of their own, so a new conversation still sees them. Nothing is installed for them and nothing is left behind on the disk.
- 🖱️ **The eye** — snap a region of the screen and pin it to the next question, like a screenshot you don't have to paste anywhere.
- 📌 **Pinned screenshots** — `Ctrl+Alt+Shift+P` leaves a dragged region hanging on the screen, above everything and on any monitor. Right-click it to ask about it, copy the text inside, copy or save the picture, fade it, or close every pin at once; the wheel resizes it.
- 🔌 **Integrations** — Stripe, n8n, GitHub, Vercel, Resend, Notion, Cal.com, plus two that need no key and no account: **PC** (CPU, RAM, disks and battery — it reads your own machine) and **Music** (whatever Windows says is playing — Spotify, a browser tab, VLC — with play/pause and skip). Each one gets its own coloured mini character pill. PC and Music are on out of the box; the rest wait until you pick them.
- 🫥 **Invisible when idle** — hides into the top edge and peeks out when you hover it. `Ctrl+Alt+M` summons it from anywhere; `Ctrl+Alt+Shift+S` starts a snip. Both chords are yours to change: Settings → General records whatever you press.
- 🔒 **Private by design** — no telemetry, no account. Keys live in the Windows Credential Manager. The app only talks to the services you plug in.
- 🌍 **English by default** — Polish is one setting away; another language is one table in `src/core/i18n.ts`.
- 🏷️ **Game prices** — a **Games** pill: type a title and it shows the cheapest key right now beside what Steam charges, with each one's record low, the cover and a way to the shops. Prices come from the **[GG.deals API](https://gg.deals/api/prices/)** on your own free key; the title lookup and the artwork come from Steam. The region field picks the currency — `pl` means złoty.
- 🖥️ **Terminal, off by default** — turn it on and the chat can run commands on your PC, in the background too. It warns you first, and every command lands in the app log.

See [`windows/README.md`](windows/README.md) for how everything works under the hood.

### Game prices and GG.deals

The **Games** pill needs a free API key from [gg.deals](https://gg.deals/settings/) (Settings → API) and a region code (`pl`, `de`, `gb`, `us`…), which is what decides the currency of every number on the card — Poland gets złoty, and without a region the API answers in dollars. Free GG.deals keys are meant for personal, hobby and open-source use, so Oczi shows the link back to the game's GG.deals page that their terms ask for.

The card itself reads only two public endpoints: GG.deals' `prices/by-steam-app-id` for the numbers, and Steam's store search for the title and the artwork. Nothing is scraped, and nothing about your library leaves the machine.

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
built on, © Adam Warzecha for the Oczi changes. The **Oczi name, the Oczi
character and the app icon** are not covered by it — see
[LICENSE-ASSETS.md](LICENSE-ASSETS.md). Forking to ship your own app is welcome;
give it your own name, icon and character.

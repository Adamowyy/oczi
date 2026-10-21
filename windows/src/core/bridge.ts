// Thin wrapper over the Tauri commands and events; a no-op in a plain browser.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { HealthReport, Settings, WatchReport } from "./state";

export const IS_TAURI =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!IS_TAURI) return null;
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    console.error(`[oczi] ${cmd} failed`, err);
    return null;
  }
}

export interface BootInfo {
  settings: Settings;
  /** Logical screen rect of the monitor the island lives on. */
  screen: { x: number; y: number; width: number; height: number; scale: number };
  version: string;
}

/** One display the monitor picker can pin the island to (Rust: `MonitorInfo`). */
export interface MonitorOption {
  /** The value `settings.screen` stores to pin this display. */
  key: string;
  /** The number Windows shows for it (1, 2, 3…), when its name carries one. */
  number: number | null;
  /** The raw device name, e.g. `\\.\DISPLAY2`. */
  name: string | null;
  width: number;
  height: number;
  scale: number;
  /** Windows' own main display. */
  primary: boolean;
}

/** A published release newer than the one running (Rust: `update::Release`). */
export interface UpdateInfo {
  /** The tag with any leading `v` trimmed: `0.1.4`. */
  version: string;
  /** The GitHub release page. */
  url: string;
}

/** One game's prices, as the Rust side reports them — see `src-tauri/src/prices.rs`. Prices are
 *  strings exactly as the API sent them, and `currency` is the code the user's own key reports:
 *  no conversion happens anywhere, so a PLN key and a USD key each see their own numbers. */
/** One package the debloat window can offer, and how safe it is to take away. */
export interface DebloatItem {
  name: string;
  tier: "safe" | "careful";
}

/** What a removal actually did — asked of Windows afterwards, not reported by the script. */
export interface DebloatRemoval {
  removed: string[];
  failed: string[];
  took: number;
}

/** One package this app has taken away, kept for the window's history. */
export interface DebloatRemoved {
  name: string;
  at: number;
}

export interface PricesOffer {
  title: string;
  appId: number;
  image: string | null;
  currency: string;
  best: string | null;
  bestFrom: "retail" | "keyshops" | null;
  retail: string | null;
  keyshops: string | null;
  historicalRetail: string | null;
  historicalKeyshops: string | null;
  url: string | null;
}

export const Bridge = {
  boot: () => call<BootInfo>("boot"),

  /** Prices for a game title: Steam finds it, gg.deals prices it. Throws the reason it could not. */
  pricesLookup: (title: string) => callOrThrow<PricesOffer>("prices_lookup", { title }),

  /** One health scan, on demand: a few seconds of load samples plus the event log. */
  healthScan: (deep: boolean) => callOrThrow<HealthReport>("health_scan", { deep }),

  /** One watch for a freeze: samples once a second for `minutes` and reports the gaps. */
  healthWatch: (minutes: number) => callOrThrow<WatchReport>("health_watch", { minutes }),

  /** Stops the watch that is running; it returns what it saw up to that moment. */
  healthWatchCancel: () => call<void>("health_watch_cancel"),

  /** The last watch ever run, so a freeze report survives a restart. */
  healthLastWatch: () => call<WatchReport>("health_last_watch"),

  /** The verdict of the last scan ever taken — what the PC pill shows before anything else. */
  healthLast: () => call<HealthReport>("health_last"),

  /** The one repair the report can offer, and only after the user pressed its button. */
  healthFix: (action: string, arg: string) => callOrThrow<string>("health_fix", { action, arg }),

  /** The packages Windows ships that the catalogue is willing to offer, as installed right now. */
  debloatList: () => callOrThrow<DebloatItem[]>("debloat_list"),

  /** Removes the ticked packages and reports what Windows says is really gone. */
  debloatRemove: (names: string[]) => callOrThrow<DebloatRemoval>("debloat_remove", { names }),

  /** What this app has taken away, newest first. */
  debloatHistory: () => call<DebloatRemoved[]>("debloat_history"),

  /** Shows one of the two PC windows: `health` or `debloat`. */
  openPcWindow: (which: "health" | "debloat") => call<void>("open_pc_window", { which }),

  /** The connected displays, for the settings window's monitor picker. */
  listMonitors: () => call<MonitorOption[]>("list_monitors"),

  /** A newer release than the one running, or null, checked once per launch. */
  checkUpdate: () => call<UpdateInfo>("check_update"),

  /** Records that this version's card has been shown, so it appears once. */
  markVersionSeen: (version: string) => call<void>("mark_version_seen", { version }),

  saveSettings: (settings: Settings) => call<void>("save_settings", { settings }),

  /** The island's own two countdowns: it sends only these, never a whole settings
   *  object, so a stale page cannot write its defaults over the file. */
  setIntervals: (intervals: { autoClose?: number; notchHide?: number }) =>
    call<void>("set_intervals", { autoClose: intervals.autoClose, notchHide: intervals.notchHide }),

  /** Shrink the window down to the invisible wake strip (hidden) or back to full. */
  setCollapsed: (collapsed: boolean) => call<void>("set_collapsed", { collapsed }),

  /** Slide the island along the top of its screen. `persist` writes it down. */
  setIslandAnchor: (anchor: number, persist: boolean) =>
    call<void>("set_island_anchor", { anchor, persist }),

  /** Island rect in window coords; Rust flips click-through from its own cursor poll, not this. */
  setIslandRect: (x: number, y: number, width: number, height: number) =>
    call<void>("set_island_rect", { x, y, width, height }),

  /** Files are only taken while the upload flow (the plus tab) is on screen. */
  setAcceptDrops: (accept: boolean) => call<void>("set_accept_drops", { accept }),

  /** Give the window keyboard focus (chat field) and take it away again. */
  focusWindow: (focused: boolean) => call<void>("focus_window", { focused }),

  reposition: () => call<void>("reposition"),

  openUrl: (url: string) => call<void>("open_url", { url }),

  quit: () => call<void>("quit_app"),

  openSettingsWindow: () => call<void>("open_settings_window"),

  /** Writes to %LOCALAPPDATA%\Oczi\oczi.log, next to the Rust lines. */
  log: (message: string) => call<void>("log_line", { message }),

  // ── Chat, files, secrets ──────────────────────────────────────────────────
  /** One chat turn. The API key and any file bytes never leave Rust. */
  chatSend: (query: string, context: ChatContext | null) =>
    callOrThrow<{ text: string }>("chat_send", { query, context }),
  /** Dictation: 16 kHz mono floats as raw bytes — the transcript comes back as text.
   *  Raw and not JSON because two minutes of audio is ~8 MB of floats, and a JSON number
   *  array of that size is both bigger and slower to parse on both sides. */
  voiceTranscribe: (pcm: Float32Array) => callRaw<string>("voice_transcribe", pcm.buffer as ArrayBuffer),
  chatReset: () => call<void>("chat_reset"),
  /** The eye, step one: freeze the desktop and show the selection overlay.
   *  The result arrives as the `snip-done` event, not this promise's value. */
  beginSnip: () => callOrThrow<void>("begin_snip"),
  /** Copies a dropped file into the inbox. */
  ingestFile: (path: string) => callOrThrow<DroppedFile>("ingest_file", { path }),
  /** Only ever tells you whether a key exists, never its value. */
  secretPresent: (key: string) => call<boolean>("secret_present", { key }),
  /** A setting that is not a secret (a region, an instance URL), read back for display. */
  secretValue: (key: string) => call<string | null>("secret_value", { key }),
  secretSet: (key: string, value: string) => callOrThrow<void>("secret_set", { key, value }),
  secretClear: (key: string) => callOrThrow<void>("secret_clear", { key }),

  // ── Integrations ──────────────────────────────────────────────────────────
  refreshIntegration: (id: string) => call<void>("refresh_integration", { id }),
  /** Opens the configured n8n instance in the browser. */
  openN8n: () => call<void>("open_n8n"),

  /** Transport buttons on the now-playing card: "toggle", "next" or "prev". */
  mediaControl: (action: "toggle" | "next" | "prev") =>
    call<void>("media_control", { action }),

  /** Tray → Pause. Stops the integration pollers, not just the island. */
  setPaused: (paused: boolean) => call<void>("set_paused", { paused }),

  reminderAdd: (text: string, at: string, repeat: string | null = null) =>
    callOrThrow<{ id: number; text: string; at: number }>("reminder_add", { text, at, repeat }),

  /** Called once the reminder listener is wired. Until this arrives nothing is
   *  shown: the event would land before this page could hear it. */
  remindersReady: () => call<void>("reminders_ready"),

  reminderDone: (id: number) => call<number | null>("reminder_done", { id }),

  /** Called once the routine listener is wired — the same handshake as `remindersReady`,
   *  for a card that must not be emitted before the island can hear it. */
  routinesReady: () => call<void>("routines_ready"),

  /** The report card has been read. A one-off is finished; a repeating routine keeps its
   *  place and only loses the report. */
  routineDone: (id: number) => call<boolean>("routine_done", { id }),

  /** Run one now instead of waiting for its time — the card reports it like any other run. */
  routineRunNow: (id: number) => callOrThrow<void>("routine_run_now", { id }),
};

export interface IntegrationUpdate {
  id: string;
  data: Record<string, unknown>;
  error: string | null;
  event: { success: boolean; label: string; detail: string | null } | null;
}

export type ChatContext =
  | { kind: "file"; name: string; path: string }
  | { kind: "window"; appName: string; title: string; url?: string }
  | { kind: "image"; name: string };

/** What the snip tool came back with. Dimensions only, the pixels stay in Rust. */
export interface SnipInfo {
  width: number;
  height: number;
  bytes: number;
}

export interface DroppedFile {
  name: string;
  path: string;
  size: number;
}

/** A reminder that has come due: Rust kept the time, the island shows the card. */
export interface ReminderEvent {
  id: number;
  text: string;
  at: number;
  /** The due time, formatted in Rust, one clock, one format on both sides. */
  atText: string;
  /** Seconds past due when it was shown, for the log. */
  lateSeconds: number;
  /** It was already due when Oczi started: it came up while the app was not there
   *  to show it, whatever the number of seconds. */
  missed: boolean;
  /** `daily`, `weekdays`, `weekly` or `monthly` when it comes back; absent for a one-off. */
  repeat?: "daily" | "weekdays" | "weekly" | "monthly";
}

/** A routine that has just run: Rust did the work, the island reports it. */
export interface RoutineEvent {
  id: number;
  label: string;
  atText: string;
  lateSeconds: number;
  /** It never ran — it came due while Oczi was off, or a run was interrupted. */
  missed: boolean;
  repeat?: "daily" | "weekdays" | "weekly" | "monthly";
  /** One entry per step, in the order it ran; `arg` is the name it acted on. */
  outcomes: { tool: string; arg: string; ok: boolean }[];
}

/** Same as `call`, but surfaces the error so the UI can show what went wrong. */
async function callOrThrow<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!IS_TAURI) throw new Error("not running inside Oczi");
  return invoke<T>(cmd, args);
}

/** A command whose argument is one binary blob rather than a set of fields: Tauri reads
 *  the bodies of these through `tauri::ipc::Request`, which is how the dictation samples
 *  cross without being turned into a JSON number array first. */
async function callRaw<T>(cmd: string, body: ArrayBuffer): Promise<T> {
  if (!IS_TAURI) throw new Error("not running inside Oczi");
  return invoke<T>(cmd, body as unknown as Record<string, unknown>);
}

export type BridgeEvent =
  | { name: "cursor"; payload: { x: number; y: number } }
  | { name: "tray"; payload: string }
  | { name: "hotkey"; payload: null }
  | { name: "hotkey-snip"; payload: null }
  | { name: "hotkey-voice"; payload: null }
  | { name: "click-outside"; payload: null }
  | { name: "snip-done"; payload: SnipInfo }
  | { name: "reminder"; payload: ReminderEvent }
  | { name: "routine"; payload: RoutineEvent }
  | { name: "health"; payload: HealthReport }
  | { name: "screen-changed"; payload: null };

export interface DragDropPayload {
  type: "enter" | "over" | "drop" | "leave";
  paths?: string[];
}

/** Files dragged onto the island. Raised by Rust off the window's own `WM_DROPFILES`, *  see `own_file_drops` for why Tauri's built-in drag path cannot work on this stack. */
export async function onDragDrop(handler: (e: DragDropPayload) => void) {
  if (!IS_TAURI) return () => {};
  return listen<DragDropPayload>("drag", (event) => handler(event.payload));
}

export async function onEvent<T>(name: string, handler: (payload: T) => void) {
  if (!IS_TAURI) return () => {};
  return listen<T>(name, (e) => handler(e.payload));
}

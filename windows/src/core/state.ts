// App state — mirror of AppState.swift (the parts the island needs).

import type { BotEmoteName, BotStateName, IslandMode, IslandViewName } from "./layout";
import type { PricesOffer } from "./bridge";
import type { EyeShape } from "../bot/engine";
import type { Lang } from "./i18n";

export type PillBadge = "finished" | "error";

export interface AgentTask {
  id: string;
  name: string;
  color: string;
  state: BotStateName;
  stepIndex: number;
  steps: string[];
  isIntegration: boolean;
  emote?: BotEmoteName | null;
  miniEye?: EyeShape | null;
  pillBadge?: PillBadge | null;
}

export interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  content: string;
}

export type PromptContext =
  | { kind: "window"; appName: string; title: string; url?: string }
  | { kind: "file"; name: string; path?: string };

export interface ResultItem {
  label: string;
  detail: string;
  url?: string;
}

export interface SearchResult {
  title: string;
  items: ResultItem[];
  note?: string;
}

/** A reminder that has come due — the payload of Rust's `reminder` event, in the
 *  shape the card needs. */
export interface DueReminder {
  id: number;
  text: string;
  atText: string;
  lateSeconds: number;
  /** Already due when Oczi started, so it came up while the app was not running. */
  missed: boolean;
  /** How it comes back, absent for a one-off: `daily`, `weekdays`, `weekly`, `monthly`, or `every:<minutes>`
   *  for an interval counted from the last time it was due. */
  repeat?: string;
}

/** One step of a routine that has just run. `tool` names the action, `arg` is what it
 *  acted on — a program, a host, a process — and the card translates the verb itself. */
export interface RoutineStep {
  tool: string;
  arg: string;
  ok: boolean;
}

/** A routine that has just run — the payload of Rust's `routine` event. Same slot as a
 *  reminder: the work is already done, and this is what the user is told about it. */
export interface DueRoutine {
  id: number;
  /** The short name the model gave it, shown as the card's title. */
  label: string;
  /** The moment it was due, formatted in Rust, so both sides agree on the clock. */
  atText: string;
  lateSeconds: number;
  /** It never ran: it came due while Oczi was off, or a run was interrupted. */
  missed: boolean;
  /** As Rust sends it: `daily`, `weekdays`, `weekly`, `monthly`, or `every:<minutes>` for an interval. */
  repeat?: string;
  outcomes: RoutineStep[];
}

/** One thing a health scan found. `kind` is a token — the wording lives in the language tables,
 *  see `health.f.<kind>`. Src: `src-tauri/src/health.rs`. */
export interface HealthFinding {
  kind: string;
  severity: "crit" | "warn" | "info";
  subject: string;
  value: string;
  /** What it most likely means — a token, `health.cause.<cause>` — or empty. */
  cause: string;
  /** The app can act on this, so the report offers a button — never without the user pressing it. */
  fixable: boolean;
  action: string;
  arg: string;
}

/** One part of the scan, so the report can show what was looked at rather than only what broke. */
export interface HealthCheck {
  id: string;
  state: "ok" | "warn" | "bad" | "skip";
}

/** The verdict of one scan. Nothing of it runs in the background: it is a snapshot, taken when
 *  the user asked for one. */
export interface HealthReport {
  score: number;
  /** Local wall-clock seconds, and the same moment as text — a local stamp cannot be turned into
   *  a date on this side without shifting it by the time zone. */
  at: number;
  atText: string;
  tookMs: number;
  samples: number;
  cpuAvg: number;
  cpuMax: number;
  memAvg: number;
  memMax: number;
  /** Logical processors: a process share is always of the whole machine, never of one core. */
  cores: number;
  /** The process that ate the most CPU during the scan, and its share of one core. */
  hog: string;
  hogPct: number;
  findings: HealthFinding[];
  checks: HealthCheck[];
  /** A deep scan's reading, written by the model when a key is connected — empty otherwise. */
  advice: string;
}

/** One moment the machine stopped, as a watch saw it. Src: `src-tauri/src/health.rs`. */
export interface Stall {
  at: number;
  atText: string;
  /** How long the machine was gone, past the second the sample was due. */
  ms: number;
  /** The process on the processor when it came back — the closest thing to a culprit. */
  top: string;
  topPct: number;
  /** Share of processor time spent in driver code around the stop — high means a driver. */
  dpc: number;
  /** Total machine load at that moment, percent. */
  cpu: number;
  /** The three busiest programs then — the second is where a driver's helper shows up. */
  tops: { name: string; pct: number }[];
}

/** The result of one watch: a bounded pass that looks for a freeze while it happens. */
export interface WatchReport {
  minutes: number;
  started: number;
  startedText: string;
  samples: number;
  cores: number;
  stalls: Stall[];
  cpuAvg: number;
  cpuPeak: number;
  memPeak: number;
  busiest: string;
  busiestPct: number;
  /** Typical spacing between stops, minutes. Zero when there is only one. */
  gapMinutes: number;
  /** Driver suites running here, named only when the driver time was high. */
  suspects: string[];
  /** Peaks of driver and interrupt time: the trace a driver-shaped stutter leaves. */
  dpcPeak: number;
  interruptPeak: number;
  /** Stopped by the user before its time. */
  cancelled: boolean;
  /** Each name once, with how many stops it showed up in — not one line per stop. */
  culprits: { name: string; count: number; pct: number }[];
  /** Stops where driver time, not a program, was the story. */
  driverStalls: number;
  /** Stops where nothing could be named at all. */
  blindStalls: number;
}

/** One package the debloat view can offer. Src: `src-tauri/src/debloat.rs`. */
export interface DebloatItem {
  name: string;
  tier: "safe" | "careful";
}

/** The debloat view's state: the list Windows reports, what is ticked, and what the last removal
 *  did. Kept here rather than inside the view so a repaint cannot lose a tick. */
export interface DebloatState {
  items: DebloatItem[];
  history: { name: string; at: number }[];
  ticked: string[];
  loading: boolean;
  busy: boolean;
  /** Names waiting for the confirmation panel, and the outcome of the last run. */
  pending: string[];
  result: { removed: string[]; failed: string[]; error?: string } | null;
  /** Which of the panel's tabs is open. The list of packages and the extra settings are two
   *  different jobs, and one strip cannot hold both without the list hidden behind a scroll. */
  tab: DebloatTab;
  /** Game Bar and the capture switches, read with the list and re-read after every change.
   *  Mirrors `GameBar` in `src-tauri/src/gamebar.rs`. */
  game: GameBarState | null;
  /** Which of the card's controls is running, so the others go quiet instead of racing it. */
  gameBusy: "capture" | "install" | "remove" | "admin" | null;
  /** The card's last word: what happened, or why nothing did. */
  gameNote: { text: string; bad?: boolean } | null;
  /** The Windows settings table, read with the list; `null` until the first reading lands. */
  tweaks: TweakState[] | null;
  /** The row mid-write, by id, and the last line the block has to say. */
  tweakBusy: string | null;
  tweakNote: { text: string; bad?: boolean } | null;
  /** The changes this app made, newest first — what "undo" walks back through. */
  changes: TweakChange[];
  /** Everything Windows would start at logon, and the entry mid-flip. */
  startup: StartupEntry[] | null;
  startupBusy: string | null;
  /** The cleaning tab: what is there now, and what the last sweep freed. */
  clean: CleanSizes | null;
  cleanBusy: boolean;
  /** A line under the sizes: "measured again", so a fast scan still shows it ran. */
  cleanNote: { text: string } | null;
  cleanFreed: CleanFreed | null;
}

/** Game Bar as Windows reports it: the package, the user's capture switches, and the machine-wide
 *  policy — `null` when nobody ever set one. */
export interface GameBarState {
  installed: boolean;
  version: string | null;
  capture: boolean;
  policy: boolean | null;
  needsAdmin: boolean;
}

/** The debloat panel's tabs: the package list Windows reports, the settings that are not a package,
 *  what starts with Windows, and the two piles worth emptying. */
export type DebloatTab = "packages" | "tweaks" | "startup" | "clean";

/** One Windows setting from `src-tauri/src/tweaks.rs`. `on` is the quiet state whatever number
 *  Windows spells it with; `needsAdmin` means the machine level refused the write. */
export interface TweakState {
  id: string;
  group: "explorer" | "taskbar" | "privacy";
  on: boolean;
  /** Machine hive: the write will ask for an administrator. */
  admin: boolean;
  needsAdmin: boolean;
  restart: boolean;
  /** Empty for a switch. When it is not, the row is a segmented control and these are its values,
   *  in the order the window shows them. */
  choices: number[];
  /** The number Windows holds right now, for a choice row. */
  value: number | null;
}

/** One change this app made, with the value it replaced — `null` meaning the value was absent. */
export interface TweakChange {
  id: string;
  at: number;
  was: number | null;
  on: boolean;
}

/** One entry Windows would start at logon. */
export interface StartupEntry {
  name: string;
  source: string;
  command: string;
  on: boolean;
  missing: boolean;
}

/** What is sitting in the temp folder and the Recycle Bin. */
export interface CleanSizes {
  temp: number;
  recycle: number;
  recycleItems: number;
}

/** What one sweep actually freed. */
export interface CleanFreed {
  temp: number;
  recycle: number;
  skipped: number;
}

/** The two shapes a routine's report can take, in one place: the island hides its header for
 *  both, and the view the report is shown in comes from `routineView`. */
export const ROUTINE_VIEWS: ReadonlySet<IslandViewName> = new Set(["routine", "routineFull"]);

/** Views that draw over the whole island and so hide the header: a report and a checklist are
 *  places with their own title, not tabs to navigate from — and the header's icons are what the
 *  mascot used to sit on. */
export const HEADERLESS_VIEWS: ReadonlySet<IslandViewName> = new Set([
  "routine",
  "routineFull",
  "report",
  "debloat",
  "watch",
  "health",
]);

/** Which report card a routine's result needs. The slim one is the usual case: it says what ran
 *  and nothing else. Anything with something to explain — a step that failed, or a run that never
 *  happened — gets the taller card, and so does a run of several steps, because a line about
 *  four things does not fit in a strip. */
export function routineView(r: DueRoutine): IslandViewName {
  const failed = r.outcomes.some((o) => !o.ok);
  return r.missed || failed || r.outcomes.length >= 3 ? "routineFull" : "routine";
}

const task = (
  id: string, name: string, color: string,
): AgentTask => ({
  id, name, color, state: "idle", stepIndex: 0, steps: [], isIntegration: true,
});

/** AgentTask.integrationAgents — same ids, names and colours as macOS. */
export const INTEGRATION_AGENTS: AgentTask[] = [
  task("integration_resend", "Resend", "#22C55E"),
  task("integration_n8n", "n8n", "#F29B38"),
  task("integration_vercel", "Vercel", "#7C5CFF"),
  task("integration_github", "GitHub", "#F4505E"),
  task("integration_notion", "Notion", "#8C8C8C"),
  task("integration_calcom", "Cal.com", "#C9956A"),
  task("integration_stripe", "Stripe", "#0570DE"),
  // The machine itself: no account, no key, no request — always available.
  task("integration_pc", "PC", "#4CC2FF"),
  // Game prices: gg.deals with the user's own API key, found by title through Steam.
  task("integration_prices", "Games", "#E4A11B"),
  // One pill for whatever player Windows says is playing — see src-tauri/media.rs.
  task("integration_music", "Music", "#F472B6"),
];

export const INTEGRATION_IDS = INTEGRATION_AGENTS.map((t) => t.id);

/** What an integration poller last reported. */
export interface IntegrationInfo {
  data: Record<string, unknown>;
  error: string | null;
  loaded: boolean;
  configured: boolean;
}

export interface Settings {
  soundEnabled: boolean;
  soundVolume: number;
  autoCloseInterval: number;
  /** Seconds the small bar waits before it hides once the cursor leaves it. */
  notchHideInterval: number;
  /** Where the island sits along the top of its screen: 0 left, 0.5 centre, 1 right. */
  islandAnchor: number;
  absenceInterval: number;
  activeIntegrations: string[];
  /** Where the island lives: "primary", "cursor", or "monitor:<key>" for one
   *  pinned display — see `monitor_key` in island.rs. */
  screen: string;
  autostart: boolean;
  /** Which provider answers the chat: "deepseek" or "gemini" — see `core/providers`. */
  provider: string;
  /** The chat model, taken from the selected provider's own list. */
  model: string;
  /** Thinking mode: slower, but the model reasons before answering. */
  thinking: boolean;
  /** The chord that summons the island, shown as a hint in compact mode. */
  hotkey: string;
  /** The chord that starts a snip — the same thing as clicking the eye. */
  snipHotkey: string;
  /** The chord that starts dictation — the same thing as clicking the microphone. */
  voiceHotkey: string;
  /** The chord that pins a region to the screen. */
  pinHotkey: string;
  /** What the speech engine listens for: "auto", "pl" or "en". A wrong fixed language is
   *  worse than detection — Whisper translates instead of transcribing — so auto is the
   *  default and the two others exist for the user who knows better. */
  voiceLanguage: "auto" | "pl" | "en";
  /** Whether the chat may search the live web through its tools. */
  webSearch: boolean;
  /** Game mode: quiet over a fullscreen app on the island's own display. */
  gameMode: boolean;
  /** Which search backend backs those tools: "duckduckgo", "brave" or "tavily". */
  searchProvider: string;
  /** UI language. English unless the user picked Polish. */
  language: Lang;
  /** Let the chat run commands on this PC. Off unless the user turned it on. */
  terminalEnabled: boolean;
  /** The version whose card has been shown. Empty on a fresh install, which is
   *  how a first run is told apart from an update. Owned by Rust. */
  lastSeenVersion: string;
}

export const DEFAULT_SETTINGS: Settings = {
  soundEnabled: true,
  soundVolume: 0.12,
  autoCloseInterval: 15,
  notchHideInterval: 60,
  islandAnchor: 0.5,
  absenceInterval: 180,
  // On a fresh install only the two that need no key: PC and Music. Every service
  // that asks for a key is opted into in Settings.
  activeIntegrations: ["integration_pc", "integration_music"],
  screen: "primary",
  autostart: false,
  provider: "deepseek",
  model: "deepseek-flash",
  thinking: false,
  hotkey: "Ctrl+Alt+M",
  snipHotkey: "Ctrl+Alt+Shift+S",
  voiceHotkey: "Ctrl+Shift+Space",
  pinHotkey: "Ctrl+Alt+Shift+P",
  voiceLanguage: "auto",
  webSearch: true,
  gameMode: true,
  searchProvider: "duckduckgo",
  language: "en",
  terminalEnabled: false,
  lastSeenVersion: "",
};

type Listener = () => void;

class AppState {
  mode: IslandMode = "hidden";
  view: IslandViewName = "overview";

  tasks: AgentTask[] = [];
  focusId: string | null = null;

  stateOverride: BotStateName | null = null;

  /** Cursor in logical screen pixels, origin top-left (like AppState.mousePosition). */
  mouse = { x: 0, y: 0 };
  /** Cursor relative to the island's top-left corner. */
  mouseInIsland = { x: 0, y: 0 };

  isPinned = false;
  paused = false;
  /** A fullscreen app owns the island's own display: no wake band, no hover reveal, and
   *  the island stays off the screen until a chord summons it. */
  fullscreen = false;

  uploadProgress = 0;
  uploadDuration = 2.4;
  fileDragOver = false;

  promptContext: PromptContext | null = null;
  droppedFile: { name: string; path: string } | null = null;
  /** A screenshot waiting to ride along with the next question. */
  snip: { width: number; height: number } | null = null;
  /** The one-off card about this version: what changed, and any release newer
   *  than the one running. Set once at boot, shown once, then cleared. */
  newsMessage: { title: string; body: string } | null = null;
  /** A reminder that has come due and is on the card right now. Set from Rust's
   *  event, cleared when it is answered or the card folds away. */
  reminder: DueReminder | null = null;
  /** Dictation: absent when nothing is being recorded, "listening" while the microphone is
   *  open, "transcribing" while the engine works. Owned by the island, read by the view. */
  voice: "listening" | "transcribing" | null = null;
  /** Loudness of the last 50 ms of microphone, 0..1, for the level bars. */
  voiceLevel = 0;
  /** Reminders that came due while another one was on the card. Rust has already
   *  taken them off the list, so this queue is the only place they still exist. */
  queuedReminders: DueReminder[] = [];
  /** A routine that has run and whose report is on the card right now. Set from Rust's
   *  event; only the card's own button ends it, so a report cannot be missed. */
  routineReport: DueRoutine | null = null;
  /** Reports that arrived while another card was up, one at a time. */
  queuedRoutines: DueRoutine[] = [];
  /** When the report card appeared, for the few seconds it ignores a stray click. */
  routineShownAt = 0;
  /** When the card on screen appeared, for the few seconds it ignores a click that
   *  would hide the island. */
  reminderShownAt = 0;
  noteMessage: string | null = null;

  /** The last watch: read back from disk at boot, replaced when a new one runs. */
  watch: WatchReport | null = null;
  /** A watch is running: the hub says so and counts down instead of looking dead. */
  watching = false;
  /** When the running watch started (Date.now()), and how long it was asked for. Both live here so
   *  a repaint cannot restart the countdown from a different origin. */
  watchStartAt = 0;
  watchMinutes = 15;
  /** A length picked but not started yet: the explanation is on screen waiting for "go". */
  watchPending: number | null = null;

  /** The debloat list, once it has been read. `null` until the view is opened for the first time. */
  debloat: DebloatState | null = null;

  /** The last health scan — from this session, or read back from disk when there is one. */
  health: HealthReport | null = null;
  /** A scan is running: the tiles say so instead of looking dead. */
  healthBusy = false;
  /** Which scan is running, so the card can say "a deep scan, about a minute". */
  healthDeep = false;
  /** Why the last scan could not finish, if it could not. */
  healthError: string | null = null;

  /** The last price card — whatever the Games pill or the model last looked up. */
  prices: PricesOffer | null = null;
  pricesBusy = false;
  pricesError: string | null = null;
  searchResult: SearchResult | null = null;
  chatHistory: ChatMessage[] = [];
  /** Wall-clock ms of the last chat turn — drives the one-hour session timeout. */
  chatLastActivity = 0;

  integrations: Record<string, IntegrationInfo> = {};

  lastActivity = performance.now();

  settings: Settings = { ...DEFAULT_SETTINGS };

  private listeners = new Set<Listener>();

  subscribe(fn: Listener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Marks the UI dirty; the island re-renders on the next frame. */
  notify() {
    for (const fn of this.listeners) fn();
  }

  get focusTask(): AgentTask | null {
    return this.tasks.find((t) => t.id === this.focusId) ?? this.tasks[0] ?? null;
  }

  get effectiveState(): BotStateName {
    return this.stateOverride ?? this.focusTask?.state ?? "idle";
  }

  get otherTasks(): AgentTask[] {
    return this.tasks.filter((t) => t.id !== this.focusId);
  }

  setFocus(id: string) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    this.focusId = id;
    t.pillBadge = null;
    this.notify();
  }

  updateTask(id: string, state: BotStateName) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.state = state;
    this.notify();
  }

  appendStep(id: string, step: string) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.steps.push(step);
    if (t.steps.length > 20) t.steps.shift();
    t.stepIndex = t.steps.length - 1;
    this.notify();
  }

  setPillBadge(id: string, badge: PillBadge | null) {
    const t = this.tasks.find((x) => x.id === id);
    if (!t) return;
    t.pillBadge = badge;
    this.notify();
  }

  /** loadIntegrationTasks() — only the pills switched on in settings. */
  loadIntegrationTasks() {
    for (const proto of INTEGRATION_AGENTS) {
      const shouldLoad = this.settings.activeIntegrations.includes(proto.id);
      const idx = this.tasks.findIndex((t) => t.id === proto.id);
      if (shouldLoad && idx < 0) this.tasks.push({ ...proto, steps: [] });
      if (!shouldLoad && idx >= 0) this.tasks.splice(idx, 1);
    }
    // Keep the declared order so pills never shuffle.
    this.tasks.sort((a, b) => INTEGRATION_IDS.indexOf(a.id) - INTEGRATION_IDS.indexOf(b.id));
    if (!this.focusId || !this.tasks.some((t) => t.id === this.focusId)) {
      this.focusId = this.tasks[0]?.id ?? null;
    }
    this.notify();
  }

  toggleIntegration(id: string) {
    const active = this.settings.activeIntegrations;
    if (active.includes(id)) {
      this.settings.activeIntegrations = active.filter((x) => x !== id);
    } else {
      if (active.length >= 4) return;
      this.settings.activeIntegrations = [...active, id];
    }
    this.loadIntegrationTasks();
  }

  defaultView(): IslandViewName {
    return "home";
  }
}

export const State = new AppState();

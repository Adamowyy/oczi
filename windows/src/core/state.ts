// App state — mirror of AppState.swift (the parts the island needs).

import type { BotEmoteName, BotStateName, IslandMode, IslandViewName } from "./layout";
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
  /** DeepSeek model used by the chat. */
  model: string;
  /** Thinking mode: slower, but the model reasons before answering. */
  thinking: boolean;
  /** The chord that summons the island, shown as a hint in compact mode. */
  hotkey: string;
  /** Whether the chat may search the live web through its tools. */
  webSearch: boolean;
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
  activeIntegrations: [
    "integration_resend", "integration_n8n", "integration_vercel", "integration_github",
  ],
  screen: "primary",
  autostart: false,
  model: "deepseek-flash",
  thinking: false,
  hotkey: "Ctrl+Alt+M",
  webSearch: true,
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
  noteMessage: string | null = null;
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

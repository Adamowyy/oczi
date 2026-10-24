// DOM ports of the Swift island views: paddings, font sizes, colours and wording follow them.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { Bridge } from "../core/bridge";
import {
  State,
  HEADERLESS_VIEWS,
  type AgentTask,
  type DebloatState,
  type GameBarState,
  type StartupEntry,
  type TweakState,
} from "../core/state";
import { washRGBA, type IslandViewName, type Wash } from "../core/layout";
import { t, type TextKey } from "../core/i18n";
import { chatKeyFor, chatLabel } from "../core/providers";
import { createMiniBot, pruneMiniBots } from "../bot/minibots";
import { buildPrompt, bubble } from "./chat";
import { buildChoose, buildUpload, buildUploading } from "./upload";
import { renderIntegrationCard, type IntegrationCardHooks } from "./integrations";
import { debloatLabel } from "../core/debloat-labels";

export interface ViewActions {
  setView(v: IslandViewName): void;
  collapse(): void;
  setFocus(id: string): void;
  /** The eye: snip a region of the screen and ask about it. */
  snip(): void;
  /** The microphone: start listening, or stop a recording already running. */
  dictate(): void;
  /** Keep the island open for `seconds` (0 = until released): a chat turn takes as
   *  long as it takes, and the answer must stay readable. */
  holdOpen(seconds: number): void;
  /** The ↗ button: opens whatever the focused pill points at. */
  openTarget(): void;
  openUrl(url: string): void;
  setAutoClose(seconds: number): void;
  setNotchHide(seconds: number): void;
  openSettingsWindow(): void;
  /** One health scan, on demand. `deep` takes a minute of samples instead of three seconds. */
  healthScan(deep: boolean): void;
  /** The watch has ended: put it on screen and hold it there until it is acknowledged. */
  watchFinished(): void;
  blip(): void;
}

export interface ViewHost {
  el: HTMLElement;
  sync(): void;
  /** Called when the view becomes active, for views with a text field. */
  focus?(): void;
  /** Called every frame while the view is on screen. */
  tick?(nowMs: number): void;
}

// ── Shared pieces ─────────────────────────────────────────────────────────────

function card(wash: Wash, ...children: (Node | string)[]): HTMLElement {
  const el = h("div", { class: wash ? "card wash" : "card" }, ...children);
  if (wash) el.style.setProperty("--wash", washRGBA(wash));
  return el;
}

function btn(
  label: string,
  kind: "primary" | "secondary",
  onClick: () => void,
  kbd?: string,
): HTMLElement {
  return h(
    "button",
    { class: `btn ${kind}`, onclick: onClick },
    h("span", { text: label }),
    kbd ? h("span", { class: "kbd", text: kbd }) : null,
  );
}

// ── Header ────────────────────────────────────────────────────────────────────

export function buildHeader(actions: ViewActions): ViewHost {
  const tabHome = h("button", { class: "tab", title: t("tab.home"), onclick: () => go("home") }, svg(ICONS.house, 13));
  const tabChat = h("button", { class: "tab", title: t("tab.ask"), onclick: () => go("prompt") }, svg(ICONS.bubble, 13));
  const tabDrop = h("button", { class: "tab", title: t("tab.add"), onclick: () => go("upload") }, svg(ICONS.plus, 13));

  const gearBtn = h("button", { title: t("int.tip"), onclick: () => go("settings") }, svg(ICONS.gear, 14));

  function go(v: IslandViewName) {
    actions.blip();
    actions.setView(v);
  }

  const el = h(
    "div",
    { id: "header" },
    h("div", { class: "tabs" }, tabHome, tabChat, tabDrop),
    h("div", { class: "header-actions" }, gearBtn),
  );

  return {
    el,
    sync() {
      const v = State.view;
      tabHome.classList.toggle("on", v === "home" || v === "empty");
      tabChat.classList.toggle("on", v === "prompt");
      tabDrop.classList.toggle("on", v === "upload");
      gearBtn.classList.toggle("on", v === "settings");
      clear(gearBtn);
      gearBtn.append(svg(v === "settings" ? ICONS.gearFill : ICONS.gear, 14));
      el.style.opacity = v === "confused" ? "0" : "1";
      // A routine's report is a notice, not a place to navigate from: the tabs and the gear are
      // hidden *and* take up no room, so the card fills the island instead of sitting under a
      // strip of controls that have nothing to do with what just happened.
      el.style.display = HEADERLESS_VIEWS.has(v) ? "none" : "";
    },
  };
}

// ── Home ──────────────────────────────────────────────────────────────────────

/** Wipes the conversation on both sides of the IPC, so the model forgets it too. */
function clearSession(actions: ViewActions) {
  State.chatHistory = [];
  State.snip = null;
  State.droppedFile = null;
  State.chatLastActivity = Date.now();
  void Bridge.chatReset();
  actions.blip();
  State.notify();
}

function buildChatBar(actions: ViewActions): { el: HTMLElement; sync(): void } {
  // The eye is the one thing here that must not open the chat: it works on the
  // screen, not in the conversation.
  const eye = h("button", { class: "snip-btn", title: t("chat.snipTip") }, svg(ICONS.eye, 13));
  eye.addEventListener("mousedown", (e) => {
    e.stopPropagation();
    actions.snip();
  });
  // The pills card has one line for the bar, so here the plus stands on its own and the
  // title carries the words.
  const fresh = h(
    "button",
    { class: "new-chat-btn bare", title: t("chat.newTip") },
    svg(ICONS.plus, 11),
  );
  fresh.addEventListener("mousedown", (e) => {
    e.stopPropagation();
    clearSession(actions);
  });

  const bar = h(
    "div",
    { class: "chat-bar home-bar", title: t("chat.openBar") },
    fresh,
    eye,
    h("div", { class: "home-hint", text: t("chat.placeholder") }),
    h("button", { class: "send-btn" }, svg(ICONS.arrowUp, 11)),
  );
  bar.addEventListener("mousedown", () => {
    actions.blip();
    actions.setView("prompt");
  });

  return {
    el: bar,
    sync() {
      // The new-chat button only means anything once there is a conversation to
      // clear, and it takes its space with it, so the bar stays centred.
      const hasSession =
        State.chatHistory.length > 0 || State.snip != null || State.droppedFile != null;
      fresh.style.display = hasSession ? "" : "none";
    },
  };
}

/** The island's home screen: Oczi, the integration pills above the bar, and the bar
 *  as a doorway into the chat. One text input in the app, not two fighting over focus. */
function buildHome(actions: ViewActions): ViewHost {
  const pills = h("div", { class: "pills" });
  const chatBar = buildChatBar(actions);

  const shortcuts = h("div", { class: "home-shortcuts" }, h("span", {}));

  const el = h(
    "div",
    { class: "view home" },
    h("div", { class: "card wash home-card" },
      h("div", { class: "home-body" },
        // Empty on purpose: Oczi is drawn here by the canvas, not by the DOM.
        h("div", { class: "home-left" }),
        h("div", { class: "home-right" }, pills, shortcuts, chatBar.el),
      ),
    ),
  );
  (el.querySelector(".card") as HTMLElement).style.setProperty("--wash", "rgba(99,102,241,0.5)");

  let pillKey = "";
  return {
    el,
    sync() {
      chatBar.sync();

      // The chord is a setting now, so it is read every frame, not once at build.
      const hint = t("chat.snipHint", State.settings.snipHotkey);
      if (shortcuts.textContent !== hint) shortcuts.textContent = hint;

      const key = State.tasks.map((t) => `${t.id}:${t.pillBadge ?? ""}`).join("|");
      if (key !== pillKey) {
        pillKey = key;
        clear(pills);
        for (const t of State.tasks.slice(0, 4)) pills.append(buildPill(t, actions));
        pruneMiniBots();
      }
    },
  };
}

// ── Overview ──────────────────────────────────────────────────────────────────

function buildOverview(actions: ViewActions): ViewHost {
  const leftBody = h("div", { class: "left-body" });
  const jump = h(
    "button",
    { class: "icon-btn jump", title: t("int.tipOpen"), onclick: () => actions.openTarget() },
    svg(ICONS.arrowUpRight, 8),
  );
  const left = card(null, leftBody, jump);
  const pills = h("div", { class: "pills" });
  const chatBar = buildChatBar(actions);
  const right = card(null, pills, chatBar.el);

  const el = h("div", { class: "view overview" },
    h("div", { class: "left" }, left),
    // `with-bar` sits on the column, not on the card: the pills grid has to give up
    // its full height for the field to fit inside the card.
    h("div", { class: "right with-bar" }, right),
  );

  let pillIds = "";
  let detailOpen = false;
  let lastFocus: string | null = null;
  let cardKey = "";

  const hooks: IntegrationCardHooks = {
    get detailOpen() {
      return detailOpen;
    },
    openDetail() {
      detailOpen = true;
      cardKey = "";
      State.notify();
    },
    closeDetail() {
      detailOpen = false;
      cardKey = "";
      State.notify();
    },
    openSettings: () => actions.openSettingsWindow(),
  };

  return {
    el,
    sync() {
      const task = State.focusTask;
      if (task?.id !== lastFocus) {
        lastFocus = task?.id ?? null;
        detailOpen = false;
        cardKey = "";
      }

      if (task) {
        const info = State.integrations[task.id];
        const key = [
          task.id, detailOpen, task.state, task.steps.join("|"),
          info?.loaded, info?.error, info?.configured,
          JSON.stringify(info?.data ?? {}),
        ].join("~");
        if (key !== cardKey) {
          cardKey = key;
          clear(leftBody);
          leftBody.append(renderIntegrationCard(task, hooks));
        }
      }

      // The ↗ button opens whatever the card points at; the player and the machine
      // have nothing to open, so it stays out of their way.
      const opensSomething = task != null && !["integration_music", "integration_pc"].includes(task.id);
      jump.style.display = detailOpen || !opensSomething ? "none" : "";

      chatBar.sync();

      const others = State.otherTasks.slice(0, 4);
      const pillKey = others.map((t) => `${t.id}:${t.pillBadge ?? ""}`).join("|");
      if (pillKey !== pillIds) {
        pillIds = pillKey;
        clear(pills);
        for (const t of others) pills.append(buildPill(t, actions));
        pruneMiniBots();
      }
    },
  };
}

function buildPill(task: AgentTask, actions: ViewActions): HTMLElement {
  const canvas = createMiniBot(task, 24);
  const pill = h(
    "div",
    {
      class: "pill",
      // The tab is gone, so a pill is the doorway to the overview: focus the
      // service and show its card. Prices are the exception: a price needs a title typed in, so
      // that pill opens the card with the field instead of an overview entry with nothing in it.
      onclick: () => {
        actions.setFocus(task.id);
        if (task.id === "integration_prices") {
          actions.holdOpen(0);
          actions.setView("prices");
          return;
        }
        // The PC pill opens its own screen, not the overview: the buttons it carries are the whole
        // point, and an `overview` here would put the old pills-plus-field column back a moment
        // after `setFocus` had already moved to `pc`.
        if (task.id === "integration_pc") {
          actions.setView("pc");
          return;
        }
        actions.setView("overview");
      },
    },
    canvas,
    h("span", { class: "lbl", text: task.name }),
  );
  pill.style.borderColor = `${task.color}24`;
  pill.addEventListener("mouseenter", () => {
    pill.style.background = `${task.color}2e`;
    pill.style.borderColor = `${task.color}8c`;
    pill.style.boxShadow = `0 2px 10px ${task.color}59`;
    (pill.querySelector(".lbl") as HTMLElement).style.color = lighten(task.color, 0.3);
  });
  pill.addEventListener("mouseleave", () => {
    pill.style.background = "";
    pill.style.borderColor = `${task.color}24`;
    pill.style.boxShadow = "";
    (pill.querySelector(".lbl") as HTMLElement).style.color = "";
  });

  if (task.pillBadge) {
    const colors = { finished: "#22C55E", error: "#F4505E" } as const;
    const icons = { finished: ICONS.check, error: ICONS.xmark } as const;
    const inner = h("i", { style: `background:${colors[task.pillBadge]}` }, svg(icons[task.pillBadge], 6, { stroke: task.pillBadge === "finished" ? 3 : 0 }));
    const badge = h("div", { class: "pill-badge" }, inner);
    badge.style.boxShadow = `0 0 4px ${colors[task.pillBadge]}99`;
    pill.append(badge);
  }
  return pill;
}

function lighten(hex: string, amount: number): string {
  const v = parseInt(hex.replace("#", ""), 16);
  const c = [(v >> 16) & 255, (v >> 8) & 255, v & 255].map((x) =>
    Math.min(255, Math.round(x + amount * 255)),
  );
  return `rgb(${c[0]},${c[1]},${c[2]})`;
}

// ── Empty ─────────────────────────────────────────────────────────────────────

function buildEmpty(actions: ViewActions): ViewHost {
  // The button names the provider that will answer, so it is refreshed on every sync: the
  // provider is a setting, and the view is built once.
  const ask = btn(t("chat.askButton", chatLabel(State.settings.provider)), "primary", () =>
    actions.setView("prompt"),
  );
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 118px;flex-direction:row;align-items:center;gap:16px" },
    h(
      "div",
      { style: "display:flex;flex-direction:column;gap:5px" },
      h("div", { class: "title", text: t("empty.quiet") }),
      h("div", { class: "sub", text: t("chat.askSub") }),
    ),
    h("div", { class: "grow" }),
    ask,
  );
  return {
    el: h("div", { class: "view" }, card(null, body)),
    sync() {
      const label = ask.querySelector("span");
      if (label) label.textContent = t("chat.askButton", chatLabel(State.settings.provider));
    },
  };
}

// ── Confused ──────────────────────────────────────────────────────────────────

function buildConfused(): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 128px" },
    h("div", { class: "title", text: t("empty.tooMuch") }),
    h("div", { class: "sub", text: t("absence.text") }),
  );
  return { el: h("div", { class: "view" }, card("pink", body)), sync() {} };
}

// ── Note ──────────────────────────────────────────────────────────────────────

function buildNote(): ViewHost {
  const title = h("div", { class: "title" });
  const el = h("div", { class: "view" }, card(null, h("div", { class: "stack", style: "padding:0 18px 0 98px" }, title)));
  return {
    el,
    sync() {
      title.textContent = State.noteMessage ?? "";
    },
  };
}

// ── Dictation ─────────────────────────────────────────────────────────────────

/** Dictation: Oczi alone with the microphone. No card and no chat — the sentence is not
 *  there yet, and anything else on screen is something to read instead of to speak. The
 *  bars follow the voice, so an open microphone is visible rather than assumed. */
function buildListening(): ViewHost {
  const bars = Array.from({ length: 5 }, () => h("div", { class: "voice-bar" }));
  const row = h("div", { class: "voice-bars" }, ...bars);
  const label = h("div", { class: "voice-label" });
  const el = h("div", { class: "view" }, h("div", { class: "voice-wrap" }, row, label));
  // A fan of heights around the middle so the row reads as a waveform rather than a meter.
  const shape = [0.5, 0.8, 1, 0.8, 0.5];
  return {
    el,
    sync() {
      const transcribing = State.voice === "transcribing";
      // While the engine runs there is no level any more; a steady mid-height says "busy"
      // instead of pretending the microphone is still hearing something.
      const level = transcribing ? 0.45 : State.voiceLevel;
      bars.forEach((bar, i) => {
        bar.style.height = `${6 + Math.round(level * shape[i] * 46)}px`;
      });
      label.textContent = transcribing ? t("voice.thinking") : t("voice.listening");
    },
  };
}

// ── Reminder ──────────────────────────────────────────────────────────────────

function answer(actions: ViewActions) {
  if (State.reminder) void Bridge.reminderDone(State.reminder.id);
  const next = State.queuedReminders.shift() ?? null;
  State.reminder = next;
  // Logged because the queue is the only place a second reminder exists by then.
  void Bridge.log(
    next ? `reminder #${next.id} next (${State.queuedReminders.length} more)` : "reminder answered",
  );
  if (next) {
    State.isPinned = true;
    State.reminderShownAt = performance.now();
    actions.setView("reminder");
    return;
  }
  State.isPinned = false;
  actions.collapse();
}

/** The i18n key for each repeat, so the card can say what comes back. */
const CYCLE: Record<string, TextKey> = {
  daily: "rem.daily",
  weekdays: "rem.weekdays",
  weekly: "rem.weekly",
  monthly: "rem.monthly",
};

/** How a repeat reads on a card, from the one word Rust sends: `daily`/`weekdays`/`weekly`/
 *  `monthly`, or
 *  `every:<minutes>`. An interval needs its own phrase rather than a key lookup, because "co 45
 *  minut" is a number and a unit that inflects — "co 2 minuty" and "co 5 minut" are not the same
 *  word, and neither is "co godzinę". */
function cycleLabel(spec: string): string | null {
  const [kind, arg] = spec.split(":");
  if (kind !== "every") return CYCLE[kind] ? t(CYCLE[kind]) : null;
  const minutes = Number(arg);
  if (!Number.isFinite(minutes) || minutes <= 0) return null;
  if (minutes === 60) return t("rem.hourly");
  if (minutes % 60 === 0) return t(countKey("rem.everyHours", minutes / 60), String(minutes / 60));
  if (minutes === 1) return t("rem.everyMinute");
  return t(countKey("rem.everyMinutes", minutes), String(minutes));
}

/** Polish counts two at a time: 2–4 take the "few" form ("co 2 minuty"), everything else — bar the
 *  teens, where the same word as the many form is right — the "many" one ("co 5 minut"). English
 *  points both keys at one string. */
function countKey(base: "rem.everyMinutes" | "rem.everyHours", n: number): TextKey {
  const mod100 = n % 100;
  const mod10 = n % 10;
  const few = mod10 >= 2 && mod10 <= 4 && !(mod100 >= 12 && mod100 <= 14);
  return (few ? `${base}Few` : base) as TextKey;
}

const STALE_REMINDER_MS = 30 * 60 * 1000;

function buildReminder(actions: ViewActions): ViewHost {
  const kind = h("div", { class: "eyebrow" });
  const text = h("div", { class: "title" });
  const whenEl = h("div", { class: "sub" });
  const snooze = btn(t("rem.snooze"), "secondary", () => {
    const r = State.reminder;
    if (r) void Bridge.reminderAdd(r.text, "+10m");
    answer(actions);
  });
  const el = h(
    "div",
    { class: "view" },
    card("amber",
      h("div", { class: "stack", style: "padding:0 18px 0 98px" },
        kind,
        text,
        whenEl,
        h("div", { class: "actions", style: "margin-top:8px" },
          h("div", { class: "grow" }),
          snooze,
          btn(t("rem.ok"), "primary", () => answer(actions)),
        ),
      ),
    ),
  );
  return {
    el,
    sync() {
      const r = State.reminder;
      if (!r) return;
      const waiting = State.reminderShownAt ? performance.now() - State.reminderShownAt : 0;
      const missed = r.missed || waiting >= STALE_REMINDER_MS;
      kind.textContent = missed ? t("rem.missedTitle") : t("rem.title");
      kind.classList.toggle("late", missed);
      snooze.style.display = missed ? "none" : "";
      text.textContent = r.text;
      // A repeating reminder has to say so on the card: "codziennie o 11:50" and a
      // one-off at the same hour look identical otherwise.
      const cycle = r.repeat ? cycleLabel(r.repeat) : null;
      const when = t("rem.at", r.atText);
      whenEl.textContent = cycle ? `${when} · ${cycle}` : when;
    },
  };
}

// ── Routine report ────────────────────────────────────────────────────────────

/** A failed step is *named*, never described with a verb: "✗ otworzyłem X" claims the step
 *  worked while crossing it out, and a tool's own name is an implementation detail the card has
 *  no business showing. `routine.stepFailed` says what happened; the name is data. */

/** The ✓ that draws itself: this card is gone within seconds, and a tick being drawn is what
 *  says "done" in that time without a sentence. */
function doneTick(): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const el = document.createElementNS(ns, "svg");
  el.setAttribute("viewBox", "0 0 78 78");
  el.setAttribute("class", "done-tick");
  el.setAttribute("aria-hidden", "true");
  const ring = document.createElementNS(ns, "circle");
  ring.setAttribute("cx", "39");
  ring.setAttribute("cy", "39");
  ring.setAttribute("r", "32");
  const mark = document.createElementNS(ns, "path");
  mark.setAttribute("d", "M23.5 40.5l10.8 10.8 20.5-21.6");
  el.append(ring, mark);
  return el;
}

function buildRoutine(_actions: ViewActions, full: boolean): ViewHost {
  const tick = h("div", { class: full ? "done-tick-wrap" : "done-tick-wrap slim" }, doneTick());
  const kind = h("div", { class: "eyebrow" });
  const title = h("div", { class: "title" });
  const whenEl = h("div", { class: "sub" });
  const note = h("div", { class: "detail" });
  // Two shapes of the same news. The usual one is a single line — what ran, and the tick — in a
  // strip barely taller than the bar; the taller one is for a report that has something to
  // explain: a failure, or a run that never happened at all.
  const shell = card("green",
    tick,
    h(
      "div",
      { class: "stack", style: full ? "padding:0 156px 0 98px" : "padding:0 116px 0 98px" },
      kind,
      title,
      whenEl,
      note,
    ),
  );
  const el = h("div", { class: "view" }, shell);
  // The report the tick was last drawn for: the animation replays when a different report
  // takes the card, and not on every repaint of the same one.
  let tickId = -1;
  return {
    el,
    sync() {
      const r = State.routineReport;
      if (!r) return;
      const failed = r.outcomes.filter((o) => !o.ok);
      // The slim card says one thing: what ran. The kind of card, the time and the detail line
      // belong to the shape that has something to explain.
      if (!full) {
        kind.style.display = "none";
        whenEl.style.display = "none";
        note.style.display = "none";
      }
      // The wash says how it went before any of the words do: green all round, amber when part
      // of it worked, red when nothing ran at all.
      shell.style.setProperty("--wash", washRGBA(r.missed ? "red" : failed.length ? "amber" : "green"));
      kind.textContent = r.missed
        ? t("routine.missedTitle")
        : failed.length
          ? t("routine.partialTitle")
          : t("routine.title");
      kind.classList.toggle("late", r.missed || failed.length > 0);
      title.textContent = r.label;
      // A repeating routine has to say so, exactly as a reminder card does.
      const cycle = r.repeat ? cycleLabel(r.repeat) : null;
      const when = t("rem.at", r.atText);
      whenEl.textContent = cycle ? `${when} · ${cycle}` : when;

      // One line, and only when there is something to say: what went wrong, or that nothing
      // happened at all. A card that has to be gone in four seconds says the least it can.
      const wrong = failed[0];
      const line = r.missed
        ? t("routine.missedWhy")
        : wrong
          ? `✗ ${t("routine.stepFailed", wrong.arg)}`
          : "";
      note.textContent = line;
      note.style.display = full && line ? "" : "none";

      // The tick is drawn per appearance, not per build: this view is made once and shown
      // many times, so the animation is taken off, the element reflowed, and put back.
      tick.classList.toggle("warn", !r.missed && failed.length > 0);
      tick.classList.toggle("bad", r.missed);
      if (r.id !== tickId) {
        tickId = r.id;
        tick.classList.remove("play");
        void tick.getBoundingClientRect();
        tick.classList.add("play");
      }
    },
  };
}

// ── What's new (once, after an update) ────────────────────────────────────────

function buildWhatsNew(actions: ViewActions): ViewHost {
  const title = h("div", { class: "title" });
  const body = h("div", {});
  const el = h(
    "div",
    { class: "view" },
    card(null,
      h("div", { class: "stack", style: "padding:0 18px 0 98px" },
        title,
        body,
        h("div", { class: "row" },
          h("div", { class: "grow" }),
          btn(t("news.dismiss"), "primary", () => {
            State.newsMessage = null;
            actions.setView(State.defaultView());
          }),
        ),
      ),
    ),
  );
  /** The stack is centred in a card that clips, so content taller than the card loses its top and
   *  bottom. Measured only once the card is actually on screen: at boot the island is still the
   *  compact bar, and the height read there says nothing about the card. */
  function measureWhenShown() {
    // The island animates its own height, so a read taken the moment the view turns on is a frame
    // in the middle of the growth and says nothing about the card.
    window.setTimeout(() => {
      if (!el.classList.contains("on")) return;
      const stack = el.querySelector(".stack") as HTMLElement | null;
      const room = stack?.parentElement?.clientHeight ?? 0;
      if (!stack || room <= 0) return;
      void Bridge.log(`news  card ${stack.scrollHeight}/${room}`);
      if (stack.scrollHeight > room + 1) {
        void Bridge.log(`news  card overflows: ${stack.scrollHeight} > ${room}`);
      }
    }, 1100);
  }

  let shown: typeof State.newsMessage = null;
  return {
    el,
    sync() {
      const news = State.newsMessage;
      if (news === shown) return;
      shown = news;
      if (!news) return;
      title.textContent = news.title;
      clear(body);
      body.append(bubble({ id: 0, role: "assistant", content: news.body }));
      measureWhenShown();
    },
  };
}

// ── In-island settings ────────────────────────────────────────────────────────

function buildSettings(actions: ViewActions): ViewHost {
  const autoLabel = h("span", {});
  const segButtons = [10, 15, 30].map((s) =>
    h("button", { onclick: () => actions.setAutoClose(s) }, `${s}s`),
  );
  const notchLabel = h("span", {});
  const notchButtons = [10, 30, 60].map((s) =>
    h("button", { onclick: () => actions.setNotchHide(s) }, `${s}s`),
  );
  // The island may only ask whether the key exists, never read it, so the
  // badge is refreshed from Rust instead of from State.
  const keyBadge = h("span", { class: "status-badge" });
  let hasKey = false;
  let lastKeyCheck = -Infinity;
  async function refreshKey() {
    lastKeyCheck = performance.now();
    hasKey = (await Bridge.secretPresent(chatKeyFor(State.settings.provider))) ?? false;
    State.notify();
  }
  void refreshKey();

  const rows = h(
    "div",
    { class: "settings-rows" },
    h(
      "div",
      { class: "settings-row" },
      svg(ICONS.timer, 12),
      autoLabel,
      h("div", { class: "seg" }, ...segButtons),
    ),
    h(
      "div",
      { class: "settings-row" },
      svg(ICONS.timer, 12),
      notchLabel,
      h("div", { class: "seg" }, ...notchButtons),
    ),
    h(
      "div",
      { class: "settings-row", style: "gap:14px" },
      keyBadge,
      h("div", { class: "grow" }),
      h("button", {
        class: "btn secondary",
        onclick: () => actions.openSettingsWindow(),
      }, svg(ICONS.gear, 13), document.createTextNode(t("int.settings"))),
    ),
  );

  const el = h("div", { class: "view" },
    card(null, h("div", { class: "stack", style: "padding:14px 16px 14px 84px" }, rows)));

  return {
    el,
    sync() {
      const s = State.settings;
      autoLabel.textContent = t("set.autoCloseChip", Math.round(s.autoCloseInterval));
      segButtons.forEach((b, i) => b.classList.toggle("on", s.autoCloseInterval === [10, 15, 30][i]));
      notchLabel.textContent = t("set.notchChip", Math.round(s.notchHideInterval));
      notchButtons.forEach((b, i) => b.classList.toggle("on", s.notchHideInterval === [10, 30, 60][i]));
      // Re-check while the view is open, so saving a key in the settings window
      // shows up without a restart.
      if (performance.now() - lastKeyCheck > 1500) void refreshKey();
      clear(keyBadge);
      keyBadge.append(dot(hasKey ? "#22C55E" : "#F4505E", 6), h("span", { text: chatLabel(State.settings.provider) }));
    },
  };
}

// ── Placeholders filled in later stages ───────────────────────────────────────

function buildPlaceholder(title: string, sub: string): ViewHost {
  const body = h(
    "div",
    { class: "stack", style: "padding:0 18px 0 118px" },
    h("div", { class: "title", text: title }),
    h("div", { class: "sub", text: sub }),
  );
  return { el: h("div", { class: "view" }, card(null, body)), sync() {} };
}

// ── Registry ──────────────────────────────────────────────────────────────────

// ── PC: the machine's own screen ──────────────────────────────────────────────

/** A finding as one line of text. The wording is a template per kind, so the same scan reads in
 *  whichever language the island is in — `subject` is the program, service or drive the finding is
 *  about and `value` is the number that goes with it. */
function findingText(f: { kind: string; subject: string; value: string }): string {
  const key = `health.f.${f.kind}` as TextKey;
  return f.subject ? t(key, f.subject, f.value) : t(key, f.value);
}

/** How loud a score is: the number decides the colour, so a glance is enough. */
function scoreColor(score: number): string {
  return score >= 90 ? "#22c55e" : score >= 70 ? "#f5a524" : "#f4505e";
}

/** "12 minutes ago", from the seconds Rust counted — the island does the arithmetic once rather
 *  than keeping a clock of its own. */
function agoText(seconds: number): string {
  if (seconds < 90) return t("health.justNow");
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return t("health.agoMin", minutes);
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t("health.agoHour", hours);
  return t("health.agoDay", Math.round(hours / 24));
}

/** The right half of the PC screen: two things worth doing with a machine, and what is already
 *  known about it. It replaces the pills and the chat field, which the other pills keep — this
 *  screen is not a doorway to anything, it is the machine's own page. */
function buildPc(actions: ViewActions): ViewHost {
  const leftBody = h("div", { class: "left-body" });
  const left = card(null, leftBody);

  const healthTile = h(
    "button",
    {
      class: "pc-tile",
      // Clicking always leads somewhere: mid-scan to the card that says the scan is running, which
      // is also where the length of the run is written. A tile that ignores presses during the one
      // moment you want to look inside it is a tile that feels broken.
      onclick: () =>
        State.healthBusy || State.health ? actions.setView("report") : actions.healthScan(false),
    },
    h("span", { class: "big", text: t("pc.health") }),
    h("span", { class: "small", text: t("pc.healthSub") }),
  );
  const debloatTile = h(
    "button",
    { class: "pc-tile", onclick: () => actions.setView("debloat") },
    h("span", { class: "big", text: t("pc.debloat") }),
    h("span", { class: "small", text: t("pc.debloatSub") }),
  );

  const score = h("button", { class: "pc-score", onclick: () => actions.setView("report") });
  const when = h("div", { class: "pc-when" });
  const note = h("div", { class: "pc-note" });

  // The watch is a small button, not a third tile: two big tasks fit the card, three pushed the
  // frame out of the island and clipped the top of it.
  const watchBtn = h("button", {
    class: "pc-watch",
    text: t("pc.watchTitle"),
    title: t("pc.watchSub"),
    // Opens the watch and arms the explanation — it does not start anything. A run that begins on a
    // single click, with the length still to be chosen, is a click that cannot be taken back.
    onclick: () => {
      // A watch already running: this button is the way back into its card, where the countdown and
      // "stop watching" live. Disabling it left the run with no door.
      if (State.watching) {
        actions.setView("watch");
        return;
      }
      actions.setView("watch");
      State.watchPending = 15;
      State.notify();
    },
  });
  // The watch sits under the two tiles, centred: beside the score it read as part of the number.
  const right = card(
    null,
    h("div", { class: "pc-actions" }, healthTile, debloatTile),
    h("div", { class: "pc-watch-row" }, watchBtn),
    h("div", { class: "pc-stats" },
      h("div", { class: "pc-meta" }, h("div", { class: "pc-when-row" }, score), when, note),
    ),
  );
  const el = h(
    "div",
    { class: "view overview pc" },
    h("div", { class: "left" }, left),
    h("div", { class: "right pc-right" }, right),
  );

  const hooks: IntegrationCardHooks = {
    get detailOpen() {
      return false;
    },
    openDetail() {},
    closeDetail() {},
    openSettings: () => actions.openSettingsWindow(),
  };
  let cardKey = "";

  return {
    el,
    sync() {
      const task = State.tasks.find((t) => t.id === "integration_pc");
      if (task) {
        const info = State.integrations[task.id];
        const key = [task.state, info?.loaded, info?.error, JSON.stringify(info?.data ?? {})].join("~");
        if (key !== cardKey) {
          cardKey = key;
          clear(leftBody);
          leftBody.append(renderIntegrationCard(task, hooks));
        }
      }

      // The watch button says when a watch is running, and stops taking presses: a second one would
      // be ignored, and a button that does nothing is worse than one that says why.
      watchBtn.textContent = State.watching ? t("pc.watchRunning") : t("pc.watchTitle");

      // The tiles say what they are doing; a scan takes a few seconds and a tile that looked dead
      // for three of them is a tile that gets pressed twice.
      healthTile.classList.toggle("busy", State.healthBusy);
      (healthTile.querySelector(".small") as HTMLElement).textContent = State.healthBusy
        ? t("pc.healthBusy")
        : t("pc.healthSub");

      const report = State.health;
      score.textContent = report ? `${report.score}%` : "—";
      score.style.color = report ? scoreColor(report.score) : "";
      if (!report) {
        when.textContent = t("pc.never");
        note.textContent = "";
        return;
      }
      score.title = t("pc.openReport");
      const ago = Math.max(0, localNowSeconds() - report.at);
      when.textContent = t("pc.lastScan", agoText(ago));
      const problems = report.findings.filter((f) => f.severity !== "info").length;
      note.textContent = problems > 0 ? t("pc.problems", problems) : t("pc.allGood");
      note.classList.toggle("bad", problems > 0);
    },
  };
}

/** Now, in the same flavour Rust counts in: *local* wall-clock seconds since the epoch, because a
 *  plain `Date.now()` is UTC and the two differ by the time-zone offset. */
function localNowSeconds(): number {
  return Math.floor(Date.now() / 1000) - new Date().getTimezoneOffset() * 60;
}

/** The findings a check is responsible for. Its tooltip lists them, which is the answer to "what
 *  does this dot mean" — the number beside it stays a number, as it was. */
function checkFindings<T extends { kind: string }>(id: string, findings: T[]): T[] {
  const kinds: Record<string, string[]> = {
    events: ["restart", "bugcheck", "hardware", "driverReset", "service", "appCrash", "appHang", "throttle", "diskError"],
    disks: ["diskFull"],
    startup: ["startupLeftover"],
    load: ["cpuHigh", "memHigh", "hog"],
    origin: ["oddProcess"],
    wer: ["bugcheck", "crashModule", "hangModule"],
    reboot: ["reboot"],
  };
  const wanted = kinds[id];
  if (!wanted) return [];
  return findings.filter((f) => wanted.includes(f.kind));
}

/** The verdict after a scan: the score, the loudest findings and the way on to the full report.
 *  Pinned by the caller, because it is read — and its buttons are what close it. */
function buildHealth(actions: ViewActions): ViewHost {
  const eyebrow = h("div", { class: "eyebrow" });
  const title = h("div", { class: "title" });
  const when = h("div", { class: "sub" });
  const lines = h("div", { class: "health-lines" });
  const note = h("div", { class: "detail" });

  const details = btn(t("health.details"), "primary", () => actions.setView("report"));
  const deeper = btn(t("health.deeper"), "secondary", () => actions.healthScan(true));
  const close = btn(t("health.close"), "secondary", () => actions.collapse());
  const buttons = h("div", { class: "health-actions" }, details, deeper, close);

  // No header on this card, so the text starts where the tabs used to end: the mascot has the
  // corner to itself and nothing is drawn over a line of text.
  const shell = card(null, h("div", { class: "stack", style: "padding:0 18px 0 66px" },
    eyebrow, title, when, lines, note, buttons));
  const el = h("div", { class: "view" }, shell);

  return {
    el,
    sync() {
      const report = State.health;
      if (!report) {
        eyebrow.textContent = t("health.eyebrow");
        title.textContent = t("health.none");
        when.textContent = "";
        lines.replaceChildren();
        note.textContent = State.healthError ?? "";
        return;
      }
      const loud = report.findings.filter((f) => f.severity !== "info");
      eyebrow.textContent = t("health.eyebrow");
      title.textContent = t("health.score", report.score);
      title.style.color = scoreColor(report.score);
      when.textContent = t("health.took", report.atText, (report.tookMs / 1000).toFixed(1));
      // Three at most: the card is a glance, the window is the list.
      lines.replaceChildren(
        ...report.findings.slice(0, 3).map((f) =>
          h("div", { class: `health-line ${f.severity}` },
            h("i", { class: "health-dot" }), findingText(f)),
        ),
      );
      // The card is the verdict, not the essay: the model's reading runs to several sentences and
      // clamping it here cut it mid-word and left it on screen after the card was put away. It
      // lives in the report, which is one press away.
      note.textContent = loud.length === 0 ? t("health.clean") : "";
      (deeper.querySelector("span") as HTMLElement).textContent = State.healthBusy
        ? t("health.scanning")
        : t("health.deeper");
      (deeper as HTMLButtonElement).disabled = State.healthBusy;
    },
  };
}

// ── Full-panel views: the report and the debloat list ─────────────────────────

/** The whole report, in the island instead of a window of its own. The panel is 640 × 320, so the
 *  list scrolls; everything the card summarises is here in one column, loudest first. */
function buildReport(actions: ViewActions): ViewHost {
  const head = h("div", { class: "panel-head" });
  const scroll = h("div", { class: "panel-body" });
  const foot = h("div", { class: "panel-foot" });
  const el = h(
    "div",
    { class: "view" },
    card(null, h("div", { class: "panel-view" }, head, scroll, foot)),
  );
  let key = "";

  return {
    el,
    sync() {
      const report = State.health;
      // Repaint only when the report or the busy flag changed: this runs every frame, and
      // rebuilding a list of findings sixty times a second is how a card starts to stutter.
      const next = report
        ? `${report.at}-${State.healthBusy}-${State.healthDeep}-${report.score}`
        : `none-${State.healthBusy}-${State.healthDeep}`;
      if (next === key) return;
      key = next;

      clear(head);
      clear(scroll);
      clear(foot);

      // A scan in progress owns the card: the previous report stays in memory, but printing it next
      // to a running scan reads as "these numbers are from the scan you just started".
      if (State.healthBusy) {
        scroll.append(
          h(
            "div",
            { class: "scan-wait" },
            h("div", { class: "scan-dot" }),
            h("div", { class: "scan-title", text: t("health.scanRunning") }),
            h("div", {
              class: "scan-note",
              text: t(State.healthDeep ? "health.scanRunningDeep" : "health.scanRunningShort"),
            }),
          ),
        );
        // The way out sits where it sits on every other card: top right. In the footer it drifted to
        // the bottom left, which is the one corner nobody looks at.
        head.append(
          h("div", { class: "grow" }),
          h("button", {
            class: "ghost close",
            text: "×",
            title: t("health.close"),
            onclick: () => actions.collapse(),
          }),
        );
        return;
      }

      head.append(
        h(
          "div",
          { class: "grow" },
          h("div", { class: "eyebrow", text: t("health.eyebrow") }),
          h("div", { class: "title", text: report ? t("health.score", report.score) : t("health.none") }),
          report ? h("div", { class: "sub", text: report.atText }) : null,
        ),
        // The header is hidden here, so leaving is a button rather than an icon in the corner.
        h("button", { class: "ghost close", text: "×", title: t("health.close"), onclick: () => actions.collapse() }),
      );

      if (!report) {
        scroll.append(h("div", { class: "panel-advice", text: State.healthError ?? t("health.none") }));
      } else {
        // The model gets its own label, because an unlabelled paragraph under an unlabelled
        // paragraph is two verdicts arguing: one is the scan's, the other is the model's reading.
        if (report.advice) {
          scroll.append(
            h("div", { class: "eyebrow", text: t("health.adviceTitle") }),
            h("div", { class: "panel-advice", text: report.advice }),
          );
        }
        const shown = report.findings.slice(0, 4);
        const rest = report.findings.length - shown.length;
        const rows = report.findings.length
          ? shown.map((f) =>
              h(
                "div",
                { class: `panel-row ${f.severity}` },
                h("i", { class: "sev" }),
                h(
                  "span",
                  { class: "text" },
                  findingText(f),
                  // Naming the culprit is the difference between "it crashed" and "this module
                  // crashed it" — and it costs one dim line under the finding, not a window.
                  f.cause
                    ? h("span", {
                        class: "cause",
                        // A peak carries the name of whoever was on the processor then.
                        text: f.cause.startsWith("peak.")
                          ? t("health.cause.peak", f.cause.slice(5))
                          : t(("health.cause." + f.cause) as TextKey),
                      })
                    : null,
                ),
                f.fixable
                  ? h("button", {
                      text: t(f.action === "disableStartup" ? "health.fixDisable" : "health.fixClose"),
                      onclick: () => {
                        // The browser's own confirm: one press, one sentence, and the outcome is
                        // the app's text — the fix never runs because a scan said so.
                        if (!window.confirm(t("health.fixAsk", f.subject))) return;
                        void Bridge.healthFix(f.action, f.arg).then((message) => {
                          void Bridge.log(`health fix: ${message}`);
                        });
                      },
                    })
                  : null,
              ),
            )
          : [
              // …and "nothing to report" is not repeated under a paragraph that already said it.
              report.advice ? "" : h("div", { class: "panel-advice", text: t("health.noFindings") }),
            ];
        scroll.append(...rows);
        // Everything past the fourth finding is a number, not a line: the strip has room for four
        // and the tool text has them all.
        if (rest > 0) scroll.append(h("div", { class: "panel-advice", text: t("health.more", rest) }));
        // Each check names itself and its state — a row of coloured dots with the words hidden in a
        // tooltip is a legend nobody reads.
        // A dot that is not green says how much stands behind it, and the findings above say what
        // it is: an amber dot next to a bare label was the thing nobody could act on.
        scroll.append(
          h(
            "div",
            { class: "panel-checks" },
            ...report.checks.map((c) => {
              const behind = checkFindings(c.id, report.findings);
              const label = t(("health.check." + c.id) as TextKey);
              return h(
                "span",
                {
                  class: c.state === "ok" ? "on" : c.state === "skip" ? "mute" : "off",
                  // The dot and its number stay as they were; hovering names what was found, so a
                  // coloured area with a count is never a mystery.
                  title: behind.length
                    ? `${label}: ${behind.map((f) => findingText(f)).join("; ")}`
                    : label,
                },
                h("i", { class: "dot" }),
                behind.length > 0 ? `${label} · ${behind.length}` : label,
              );
            }),
          ),
        );
      }

      // One button at a time: while a scan runs there is nothing to press, so the deep option
      // steps aside instead of sitting there greyed out beside a label that already says "scanning".
      const controls: HTMLElement[] = State.healthBusy
        ? [h("button", { class: "primary busy", disabled: "true" as unknown as boolean, text: t("health.scanning") })]
        : [
            h("button", {
              class: "primary",
              text: t("health.rescan"),
              onclick: () => actions.healthScan(false),
            }),
            h("button", {
              class: "ghost",
              text: t("health.rescanDeep"),
              onclick: () => actions.healthScan(true),
            }),
          ];
      // The repair itself is the conversation: the model already has the machine's tools and the
      // rule that it asks before changing anything, so this hands it the findings and lets it work
      // there — rather than a hidden auto-fixer nobody can watch.
      if (!State.healthBusy && report && report.findings.some((f) => f.severity !== "info")) {
        controls.push(
          h("button", {
            class: "primary",
            text: t("health.repair"),
            onclick: () => {
              const list = report.findings
                .filter((f) => f.severity !== "info")
                .slice(0, 4)
                .map((f) => findingText(f))
                .join("; ");
              actions.setView("prompt");
              // The chat field belongs to its own view; the seed is written into it once it is on
              // screen, where the caret goes anyway.
              window.setTimeout(() => {
                const field = document.querySelector(".chat-input") as HTMLInputElement | null;
                if (!field) return;
                field.value = t("health.repairAsk", list);
                field.focus();
                field.setSelectionRange(field.value.length, field.value.length);
                State.notify();
              }, 160);
            },
          }),
        );
      }
      foot.append(
        h("span", {
          class: "count",
          text: report
            ? t("health.loadLine", Math.round(report.cpuAvg), Math.round(report.cpuMax), Math.round(report.memMax))
            : "",
        }),
        ...controls,
      );
    },
  };
}

/** The debloat list, in the island. Reading it is `debloat_list`; nothing here decides anything
 *  without a tick and a confirmation naming what will go. */
async function loadDebloat() {
  const previous = State.debloat;
  State.debloat = {
    items: previous?.items ?? [],
    history: previous?.history ?? [],
    ticked: previous?.ticked ?? [],
    loading: true,
    busy: false,
    pending: [],
    result: previous?.result ?? null,
    tab: previous?.tab ?? "packages",
    game: previous?.game ?? null,
    gameBusy: null,
    gameNote: previous?.gameNote ?? null,
    tweaks: previous?.tweaks ?? null,
    tweakBusy: null,
    tweakNote: previous?.tweakNote ?? null,
    changes: previous?.changes ?? [],
    startup: previous?.startup ?? null,
    startupBusy: null,
    clean: previous?.clean ?? null,
    cleanBusy: false,
    cleanNote: previous?.cleanNote ?? null,
    cleanFreed: previous?.cleanFreed ?? null,
  };
  State.notify();
  try {
    const items = await Bridge.debloatList();
    const history = (await Bridge.debloatHistory()) ?? [];
    // Each of these is its own reading and its own failure: a table that will not load must not
    // take the package list down with it.
    const game = await Bridge.gamebarStatus().catch(() => null);
    const tweaks = await Bridge.tweaksList().catch(() => null);
    const changes = (await Bridge.tweaksHistory()) ?? [];
    const startup = await Bridge.startupList().catch(() => null);
    const clean = await Bridge.cleanScan().catch(() => null);
    // A fresh reading is the truth about what is installed, so the ticks start from it: the safe
    // packages pre-ticked, the careful ones never.
    State.debloat = {
      items,
      history,
      ticked: items.filter((i) => i.tier === "safe").map((i) => i.name),
      loading: false,
      busy: false,
      pending: [],
      result: State.debloat?.result ?? null,
      tab: State.debloat?.tab ?? "packages",
      game,
      gameBusy: null,
      gameNote: State.debloat?.gameNote ?? null,
      tweaks,
      tweakBusy: null,
      tweakNote: State.debloat?.tweakNote ?? null,
      changes,
      startup,
      startupBusy: null,
      clean,
      cleanBusy: false,
      cleanNote: State.debloat?.cleanNote ?? null,
      cleanFreed: State.debloat?.cleanFreed ?? null,
    };
  } catch (e) {
    State.debloat = {
      items: [],
      history: [],
      ticked: [],
      loading: false,
      busy: false,
      pending: [],
      result: { removed: [], failed: [], error: e instanceof Error ? e.message : String(e) },
      tab: State.debloat?.tab ?? "packages",
      game: State.debloat?.game ?? null,
      gameBusy: null,
      gameNote: State.debloat?.gameNote ?? null,
      tweaks: State.debloat?.tweaks ?? null,
      tweakBusy: null,
      tweakNote: State.debloat?.tweakNote ?? null,
      changes: State.debloat?.changes ?? [],
      startup: State.debloat?.startup ?? null,
      startupBusy: null,
      clean: State.debloat?.clean ?? null,
      cleanBusy: false,
      cleanNote: State.debloat?.cleanNote ?? null,
      cleanFreed: State.debloat?.cleanFreed ?? null,
    };
  }
  State.notify();
}

/** The one package the block owns by name; the catalogue still lists it below. */
const GAME_BAR = "Microsoft.XboxGamingOverlay";

/** Game Bar and the capture behind it are one thing: the overlay records, and a machine without
 *  the overlay has nothing to record with. So one row, one button — removing it switches the
 *  recording off, installing it switches the recording back on. */
function gameBarRow(d: DebloatState, g: GameBarState): HTMLElement {
  const busy = d.gameBusy !== null;
  const state = t(g.capture ? "game.dvrOn" : "game.dvrOff");
  const button = g.installed
    ? h("button", {
        class: "ghost",
        text: d.gameBusy === "remove" ? t("game.removing") : t("game.uninstall"),
        disabled: busy,
        onclick: () => void switchGameBar(g),
      })
    : h("button", {
        class: "primary",
        text: d.gameBusy === "install" ? t("game.installing") : t("game.install"),
        disabled: busy,
        onclick: () => void switchGameBar(g),
      });
  return h(
    "div",
    { class: "game-row" },
    h(
      "span",
      { class: "game-name" },
      h("strong", { text: t("game.bar") }),
      h("span", { class: "pkg", text: g.version ? `${g.version} · ${state}` : state }),
    ),
    h("span", {
      class: g.installed ? "game-pill ok" : "game-pill off",
      text: t(g.installed ? "game.present" : "game.absent"),
    }),
    button,
  );
}

/** One click, both halves: the package and the capture switch together, in whichever direction the
 *  machine is in now. The reading afterwards decides what the row says, not the click. */
async function switchGameBar(state: GameBarState) {
  const d = State.debloat;
  if (!d || d.gameBusy) return;
  const removing = state.installed;
  d.gameBusy = removing ? "remove" : "install";
  d.gameNote = null;
  State.notify();
  try {
    if (removing) {
      const outcome = await Bridge.debloatRemove([GAME_BAR]);
      if (outcome.removed.length === 0) {
        throw new Error(outcome.failed.join(", ") || t("game.gone"));
      }
      // Removed: nothing can record any more, so the switches follow the package.
      d.game = await Bridge.gamebarCapture(false);
      d.gameNote = { text: t("game.gone") };
    } else {
      await Bridge.gamebarInstall();
      // Installed: the recording comes back with it, which is the state a fresh Windows has.
      d.game = await Bridge.gamebarCapture(true);
      d.gameNote = { text: t("game.back") };
    }
    void Bridge.log(
      `debloat game bar: ${removing ? "removed with dvr off" : "installed with dvr on"}`,
    );
  } catch (e) {
    d.gameNote = { text: t("game.failed", e instanceof Error ? e.message : String(e)), bad: true };
    void Bridge.log(`debloat game bar: ${removing ? "remove" : "install"} failed`);
  }
  d.gameBusy = null;
  await loadDebloat();
}

function gameBlock(d: DebloatState): HTMLElement {
  const g = d.game;
  const block = h(
    "div",
    { class: "game-block" },
    h("div", { class: "game-eyebrow", text: t("debloat.games") }),
  );
  if (!g) {
    block.append(h("div", { class: "game-note", text: "…" }));
    return block;
  }
  block.append(gameBarRow(d, g));
  // Only offered when the user's own switch could not hold on its own: a UAC prompt nobody asked
  // for is worse than a row that stops one level short and says so.
  if (g.needsAdmin) {
    block.append(
      h(
        "div",
        { class: "game-admin" },
        h("span", { text: t("game.needsAdmin") }),
        h("button", {
          class: "ghost",
          text: d.gameBusy === "admin" ? t("game.adminBusy") : t("game.raiseAdmin"),
          disabled: d.gameBusy !== null,
          onclick: () =>
            void gameAction(
              "admin",
              () => Bridge.gamebarPolicyElevated(!g.capture),
              t("game.adminDone"),
            ),
        }),
      ),
    );
  }
  if (d.gameNote && d.gameNote.text) {
    block.append(
      h("div", { class: d.gameNote.bad ? "game-note bad" : "game-note", text: d.gameNote.text }),
    );
  }
  return block;
}

/** Runs one other control on the card: busy flag up, the answer read back, one line about it. */
async function gameAction(
  which: "capture" | "install" | "admin",
  run: () => Promise<GameBarState>,
  said: string,
) {
  const d = State.debloat;
  if (!d || d.gameBusy) return;
  d.gameBusy = which;
  d.gameNote = null;
  State.notify();
  try {
    d.game = await run();
    d.gameNote = said ? { text: said } : null;
    void Bridge.log(`debloat game bar ${which}: ok`);
  } catch (e) {
    d.gameNote = { text: t("game.failed", e instanceof Error ? e.message : String(e)), bad: true };
    void Bridge.log(`debloat game bar ${which}: failed`);
  }
  d.gameBusy = null;
  State.notify();
}

// ── The Windows settings block ────────────────────────────────────────────────
// One table in Rust, three groups in the window. The labels live here because they are words, the
// paths live there because they are Windows' business; a new setting is one row and two strings.
// Every click is followed by a re-read of the whole table, so a switch that did not land shows as
// the state it stayed in rather than the one that was asked for.

/** The label key for a tweak id. A record rather than a switch so a missing id is a type error. */
const TWEAK_LABELS: Record<string, TextKey> = {
  "context.classic": "tweaks.contextClassic",
  "explorer.recent": "tweaks.explorerRecent",
  "explorer.frequent": "tweaks.explorerFrequent",
  "explorer.ads": "tweaks.explorerAds",
  "taskbar.widgets": "tweaks.taskbarWidgets",
  "taskbar.copilot": "tweaks.taskbarCopilot",
  "taskbar.search": "tweaks.taskbarSearch",
  "privacy.bing": "tweaks.privacyBing",
  "privacy.copilot": "tweaks.privacyCopilot",
  "privacy.silentapps": "tweaks.privacySilentApps",
  "privacy.suggestions": "tweaks.privacySuggestions",
  "privacy.softlanding": "tweaks.privacySoftLanding",
  "privacy.telemetry": "tweaks.privacyTelemetry",
};

const TWEAK_GROUPS: { id: TweakState["group"]; label: TextKey }[] = [
  { id: "explorer", label: "tweaks.groupExplorer" },
  { id: "taskbar", label: "tweaks.groupTaskbar" },
  { id: "privacy", label: "tweaks.groupPrivacy" },
];

function tweakName(id: string): string {
  const key = TWEAK_LABELS[id];
  return key ? t(key) : id;
}

/** Runs one row: busy by id, the table re-read, one line about what happened. */
async function tweakAction(id: string, run: () => Promise<TweakState[]>) {
  const d = State.debloat;
  if (!d || d.tweakBusy) return;
  d.tweakBusy = id;
  d.tweakNote = null;
  State.notify();
  try {
    d.tweaks = await run();
    d.changes = (await Bridge.tweaksHistory()) ?? [];
    void Bridge.log(`tweak ${id}: ok`);
  } catch (e) {
    d.tweakNote = { text: t("game.failed", e instanceof Error ? e.message : String(e)), bad: true };
    void Bridge.log(`tweak ${id}: failed`);
  }
  d.tweakBusy = null;
  State.notify();
}

function tweakRow(d: DebloatState, tweak: TweakState): HTMLElement {
  const busy = d.tweakBusy !== null;
  // The machine level is a promise the window can only keep through one prompt, so the row says so
  // before the click rather than after a refusal.
  const sub = tweak.needsAdmin
    ? t("tweaks.needsAdmin")
    : tweak.admin
      ? t("tweaks.adminRow")
      : tweak.restart
        ? t("tweaks.restartHint")
        : "";
  const sw = h("button", {
    class: tweak.on ? "switch on" : "switch",
    "aria-pressed": tweak.on,
    title: t(tweak.on ? "tweaks.on" : "tweaks.off"),
    disabled: busy,
    onclick: () =>
      void tweakAction(tweak.id, () =>
        tweak.admin
          ? Bridge.tweakElevated(tweak.id, !tweak.on)
          : Bridge.tweakSet(tweak.id, !tweak.on),
      ),
  });
  return h(
    "div",
    { class: "game-row" },
    h(
      "span",
      { class: "game-name" },
      h("strong", { text: tweakName(tweak.id) }),
      sub ? h("span", { class: "pkg", text: sub }) : null,
    ),
    sw,
  );
}

function tweaksBlock(d: DebloatState): HTMLElement {
  const block = h("div", { class: "tweak-group" });
  if (!d.tweaks) {
    block.append(h("div", { class: "game-note", text: "…" }));
    return block;
  }
  const on = d.tweaks.filter((tweak) => tweak.on).length;
  block.append(
    h(
      "div",
      { class: "tweaks-head" },
      h("span", { class: "game-eyebrow", text: t("tweaks.title") }),
      h("span", { class: "count", text: t("tweaks.count", on, d.tweaks.length) }),
    ),
  );
  for (const group of TWEAK_GROUPS) {
    const rows = d.tweaks.filter((tweak) => tweak.group === group.id);
    if (rows.length === 0) continue;
    block.append(
      h("div", { class: "game-eyebrow group", text: t(group.label) }),
      ...rows.map((tweak) => tweakRow(d, tweak)),
    );
  }
  if (d.tweakNote && d.tweakNote.text) {
    block.append(
      h("div", { class: d.tweakNote.bad ? "game-note bad" : "game-note", text: d.tweakNote.text }),
    );
  }
  // Half of these are only read when Explorer starts, so the restart is offered rather than done:
  // it closes every folder window the user has open.
  block.append(
    h(
      "div",
      { class: "tweaks-actions" },
      h("button", {
        class: "ghost",
        text: t("tweaks.restartExplorer"),
        disabled: d.tweakBusy !== null,
        onclick: () => void restartExplorer(),
      }),
      h("button", {
        class: "ghost",
        text: t("tweaks.resetAll"),
        disabled: d.tweakBusy !== null || d.tweaks.every((tweak) => !tweak.on && !tweak.admin),
        onclick: () => void resetAllTweaks(),
      }),
    ),
  );
  // The last three changes, with the value each replaced: the undo is a write, not a guess.
  const recent = d.changes.slice(0, 3);
  if (recent.length > 0) {
    block.append(
      h("div", { class: "game-eyebrow group", text: t("tweaks.history") }),
      ...recent.map((change) =>
        h(
          "div",
          { class: "game-row history" },
          h(
            "span",
            { class: "game-name" },
            h("strong", { text: tweakName(change.id) }),
            h("span", { class: "pkg", text: t(change.on ? "tweaks.wasOn" : "tweaks.wasOff") }),
          ),
          h("button", {
            class: "ghost",
            text: t("tweaks.undo"),
            disabled: d.tweakBusy !== null,
            onclick: () => void tweakAction(change.id, () => Bridge.tweakUndo(change.id)),
          }),
        ),
      ),
    );
  }
  return block;
}

async function restartExplorer() {
  const d = State.debloat;
  if (!d || d.tweakBusy) return;
  d.tweakBusy = "explorer";
  State.notify();
  try {
    await Bridge.restartExplorer();
    d.tweakNote = { text: t("tweaks.restarted") };
  } catch (e) {
    d.tweakNote = { text: t("game.failed", e instanceof Error ? e.message : String(e)), bad: true };
  }
  d.tweakBusy = null;
  State.notify();
}

/** Puts every switch back to Windows' default, one row at a time, and re-reads once at the end. */
async function resetAllTweaks() {
  const d = State.debloat;
  if (!d || !d.tweaks || d.tweakBusy) return;
  d.tweakBusy = "reset";
  d.tweakNote = null;
  State.notify();
  try {
    for (const tweak of d.tweaks.filter((row) => row.on)) {
      d.tweaks = await Bridge.tweakReset(tweak.id);
    }
    d.changes = (await Bridge.tweaksHistory()) ?? [];
    d.tweakNote = { text: t("tweaks.resetDone") };
    void Bridge.log("tweaks: reset to defaults");
  } catch (e) {
    d.tweakNote = { text: t("game.failed", e instanceof Error ? e.message : String(e)), bad: true };
  }
  d.tweakBusy = null;
  State.notify();
}

// ── The autostart tab ─────────────────────────────────────────────────────────
// What Windows starts at logon, switched the way Task Manager switches it: the Run value stays, only
// its approval byte changes, so nothing is deleted and Windows Explorer agrees with the list.

async function startupAction(entry: StartupEntry, on: boolean) {
  const d = State.debloat;
  if (!d || d.startupBusy) return;
  d.startupBusy = entry.name;
  State.notify();
  try {
    d.startup = await Bridge.startupSet(entry.name, on);
    void Bridge.log(`startup ${on ? "on" : "off"}: ${entry.name}`);
  } catch (e) {
    d.tweakNote = { text: t("game.failed", e instanceof Error ? e.message : String(e)), bad: true };
  }
  d.startupBusy = null;
  State.notify();
}

function startupBlock(d: DebloatState): HTMLElement {
  const block = h("div", { class: "tweak-group" });
  if (!d.startup) {
    block.append(h("div", { class: "game-note", text: "…" }));
    return block;
  }
  const on = d.startup.filter((entry) => entry.on).length;
  block.append(
    h(
      "div",
      { class: "tweaks-head" },
      h("span", { class: "game-eyebrow", text: t("startup.title") }),
      h("span", { class: "count", text: t("startup.count", on, d.startup.length) }),
    ),
  );
  if (d.startup.length === 0) {
    block.append(h("div", { class: "game-note", text: t("startup.empty") }));
    return block;
  }
  const list = h("div", { class: "panel-list" });
  for (const entry of d.startup) {
    list.append(
      h(
        "div",
        { class: "game-row" },
        h(
          "span",
          { class: "game-name" },
          h("strong", { text: entry.name }),
          h("span", {
            class: "pkg",
            text: entry.missing ? t("startup.missing") : entry.command || entry.source,
          }),
        ),
        h("button", {
          class: entry.on ? "switch on" : "switch",
          "aria-pressed": entry.on,
          title: t(entry.on ? "startup.on" : "startup.off"),
          disabled: d.startupBusy !== null,
          onclick: () => void startupAction(entry, !entry.on),
        }),
      ),
    );
  }
  block.append(list);
  return block;
}

// ── The cleaning tab ──────────────────────────────────────────────────────────
// Two piles, measured before and after: the button says how much is there, and the answer says how
// much the measurement changed — a file another program still holds is counted as skipped.

/** Bytes as a person reads them: one decimal under ten, whole numbers above it. */
function bytes(value: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let left = value;
  let at = 0;
  while (left >= 1024 && at < units.length - 1) {
    left /= 1024;
    at += 1;
  }
  const shown = at === 0 ? Math.round(left) : left < 10 ? left.toFixed(1) : Math.round(left);
  return `${shown} ${units[at]}`;
}

async function cleanScan() {
  const d = State.debloat;
  if (!d || d.cleanBusy) return;
  d.cleanBusy = true;
  d.cleanNote = null;
  State.notify();
  const started = Date.now();
  try {
    d.clean = await Bridge.cleanScan();
  } catch (e) {
    d.tweakNote = { text: t("game.failed", e instanceof Error ? e.message : String(e)), bad: true };
  }
  // An empty temp folder measures in a few milliseconds, and a button that changes nothing on
  // screen reads as broken. The busy line stays up long enough to be seen, then the answer appears.
  const spent = Date.now() - started;
  if (spent < 450) await new Promise((done) => setTimeout(done, 450 - spent));
  d.cleanBusy = false;
  d.cleanNote = { text: t("clean.measured") };
  State.notify();
}

async function cleanRun() {
  const d = State.debloat;
  if (!d || d.cleanBusy) return;
  d.cleanBusy = true;
  d.cleanFreed = null;
  d.cleanNote = null;
  State.notify();
  try {
    d.cleanFreed = await Bridge.cleanRun();
    d.clean = await Bridge.cleanScan();
    void Bridge.log("clean: swept");
  } catch (e) {
    d.tweakNote = { text: t("game.failed", e instanceof Error ? e.message : String(e)), bad: true };
  }
  d.cleanBusy = false;
  State.notify();
}

function cleanBlock(d: DebloatState): HTMLElement {
  const block = h(
    "div",
    { class: "tweak-group" },
    h("div", { class: "game-eyebrow", text: t("clean.title") }),
    h("div", { class: "game-note", text: t("clean.hint") }),
  );
  const sizes = d.clean;
  if (!sizes) {
    block.append(h("div", { class: "game-note", text: d.cleanBusy ? t("clean.scanning") : "…" }));
    return block;
  }
  block.append(
    h(
      "div",
      { class: "game-row" },
      h(
        "span",
        { class: "game-name" },
        h("strong", { text: t("clean.temp") }),
        h("span", { class: "pkg", text: bytes(sizes.temp) }),
      ),
    ),
    h(
      "div",
      { class: "game-row" },
      h(
        "span",
        { class: "game-name" },
        h("strong", { text: t("clean.recycle") }),
        h("span", {
          class: "pkg",
          text: t("clean.recycleItems", bytes(sizes.recycle), sizes.recycleItems),
        }),
      ),
    ),
  );
  if (d.cleanNote) {
    block.append(h("div", { class: "game-note", text: d.cleanNote.text }));
  }
  if (d.cleanFreed) {
    block.append(
      h("div", {
        class: "game-note",
        text: t(
          "clean.freed",
          bytes(d.cleanFreed.temp + d.cleanFreed.recycle),
          bytes(d.cleanFreed.skipped),
        ),
      }),
    );
  }
  block.append(
    h(
      "div",
      { class: "tweaks-actions" },
      h("button", {
        class: "primary",
        text: d.cleanBusy ? t("clean.busy") : t("clean.run"),
        disabled: d.cleanBusy || sizes.temp + sizes.recycle === 0,
        onclick: () => void cleanRun(),
      }),
      h("button", {
        class: "ghost",
        text: t("clean.rescan"),
        disabled: d.cleanBusy,
        onclick: () => void cleanScan(),
      }),
    ),
  );
  return block;
}

async function removeDebloat() {
  const d = State.debloat;
  if (!d || d.busy || d.pending.length === 0) return;
  d.busy = true;
  State.notify();
  try {
    const outcome = await Bridge.debloatRemove(d.pending);
    d.result = { removed: outcome.removed, failed: outcome.failed };
    d.pending = [];
    void Bridge.log(`debloat: ${outcome.removed.length} removed, ${outcome.failed.length} kept`);
  } catch (e) {
    d.result = { removed: [], failed: [], error: e instanceof Error ? e.message : String(e) };
  }
  d.busy = false;
  await loadDebloat();
}

function buildDebloat(actions: ViewActions): ViewHost {
  const head = h("div", { class: "panel-head" });
  const scroll = h("div", { class: "panel-body" });
  const foot = h("div", { class: "panel-foot" });
  const el = h(
    "div",
    { class: "view" },
    card(null, h("div", { class: "panel-view" }, head, scroll, foot)),
  );
  let key = "";
  let asked = false;

  return {
    el,
    sync() {
      // The first time the view is shown the list is read; after that it is cached until a removal.
      if (!State.debloat && !asked) {
        asked = true;
        void loadDebloat();
      }
      const d = State.debloat;
      const extra = d
        ? [
            d.tab,
            d.game ? `${d.game.installed}:${d.game.capture}:${d.game.needsAdmin}:${d.gameBusy}:${d.gameNote?.text}` : "none",
            d.tweaks ? d.tweaks.map((tweak) => `${tweak.id}:${tweak.on}:${tweak.needsAdmin}`).join(",") : "none",
            d.tweakBusy,
            d.tweakNote?.text,
            d.changes.length,
            d.startup ? d.startup.map((entry) => `${entry.name}:${entry.on}`).join(",") : "none",
            d.startupBusy,
            d.clean ? `${d.clean.temp}:${d.clean.recycle}` : "none",
            d.cleanBusy,
            d.cleanNote?.text ?? null,
            d.cleanFreed ? d.cleanFreed.temp + d.cleanFreed.recycle : "none",
          ].join(",")
        : "none";
      const next = d
        ? [d.loading, d.busy, d.items.length, d.ticked.join(","), d.pending.length, JSON.stringify(d.result), extra].join("|")
        : "none";
      if (next === key) return;
      key = next;
      clear(head);
      clear(scroll);
      clear(foot);

      head.append(
        h(
          "div",
          { class: "grow" },
          h("div", { class: "eyebrow", text: t("debloat.title") }),
          h("div", { class: "sub", text: t("debloat.sub") }),
        ),
        h("button", { class: "ghost close", text: "×", title: t("health.close"), onclick: () => actions.collapse() }),
      );

      // Four halves, one strip: the packages Windows reports, the settings that are not a
      // package, what starts with Windows, and the two piles worth emptying. The strip is drawn
      // before any of them so a tab never looks like a different window.
      const tabButton = (which: DebloatState["tab"], text: string) =>
        h("button", {
          class: State.debloat?.tab === which ? "deb-tab on" : "deb-tab",
          text,
          onclick: () => {
            const open = State.debloat;
            if (!open || open.tab === which) return;
            // A confirmation belongs to the list it names, so opening it never switches tabs.
            open.tab = which;
            State.notify();
          },
        });
      const tabs = h(
        "div",
        { class: "deb-tabs" },
        tabButton("packages", t("debloat.tabPackages")),
        tabButton("tweaks", t("debloat.tabTweaks")),
        tabButton("startup", t("debloat.tabStartup")),
        tabButton("clean", t("debloat.tabClean")),
      );

      if (!d || d.loading) {
        scroll.append(tabs, h("div", { class: "panel-advice", text: "…" }));
        return;
      }

      scroll.append(tabs);

      if (d.pending.length > 0) {
        scroll.append(
          h("div", { class: "panel-advice", text: t("debloat.confirm", d.pending.length) }),
          h("div", { class: "panel-advice", text: d.pending.map(debloatLabel).join(", ") }),
        );
        foot.append(
          h("span", { class: "count", text: t("debloat.ticked", d.ticked.length) }),
          h("button", { class: "primary", text: d.busy ? t("debloat.busy") : t("debloat.confirmYes"), onclick: () => void removeDebloat() }),
          h("button", {
            text: t("debloat.cancel"),
            onclick: () => {
              d.pending = [];
              State.notify();
            },
          }),
        );
        return;
      }

      // The extra settings: what nobody removes and many people turn off. Each block says what it
      // is in an eyebrow, so the next Windows knob has a row to join rather than a new panel.
      if (d.tab === "tweaks") {
        scroll.append(gameBlock(d), tweaksBlock(d));
        return;
      }

      if (d.tab === "startup") {
        scroll.append(startupBlock(d));
        return;
      }

      if (d.tab === "clean") {
        scroll.append(cleanBlock(d));
        return;
      }

      if (d.items.length === 0) {
        scroll.append(h("div", { class: "panel-advice", text: t("debloat.empty") }));
      }
      // One column and a scrolling list: the package id is long enough that two columns clipped the
      // tier word, and a machine nobody has cleaned has far more than fits in a strip this tall.
      const grid = h("div", { class: "panel-list" });
      scroll.append(grid);
      for (const item of d.items) {
        const box = h("input", { type: "checkbox" }) as HTMLInputElement;
        box.checked = d.ticked.includes(item.name);
        box.addEventListener("change", () => {
          d.ticked = box.checked
            ? [...d.ticked, item.name]
            : d.ticked.filter((n) => n !== item.name);
          State.notify();
        });
        grid.append(
          h(
            "label",
            { class: "deb-row" },
            box,
            h(
              "span",
              { class: "name" },
              h("strong", { text: debloatLabel(item.name) }),
              h("span", { class: "pkg", text: item.name }),
            ),
            // Only the careful tier needs saying out loud: everything else in this list is a
            // consumer app nothing depends on.
            item.tier === "careful" ? h("span", { class: "tier", text: t("debloat.careful") }) : null,
          ),
        );
      }

      if (d.result) {
        scroll.append(
          h("div", {
            class: "panel-advice",
            text: [
              d.result.removed.length ? t("debloat.done", d.result.removed.map(debloatLabel).join(", ")) : "",
              d.result.failed.length ? t("debloat.failed", d.result.failed.map(debloatLabel).join(", ")) : "",
              d.result.error ?? "",
            ]
              .filter(Boolean)
              .join(" · "),
          }),
        );
      }

      foot.append(
        h("span", { class: "count", text: t("debloat.ticked", d.ticked.length) }),
        h("button", {
          class: "primary",
          text: d.busy ? t("debloat.busy") : t("debloat.remove", d.ticked.length),
          onclick: () => {
            if (d.ticked.length === 0) return;
            d.pending = [...d.ticked];
            State.notify();
          },
        }),
      );
    },
  };
}

/** One watch, on demand. It runs on a worker thread for as many minutes as it was asked for and
 *  then stops by itself — there is no timer left behind, which is the difference between this and
 *  monitoring the machine all day.
 *
 *  Two things this used to get wrong: the countdown read a start time from a previous run, and a
 *  promise that never settled left "0:00" on screen with the stop button still showing. Both are
 *  handled here — the start is written when the run begins, and the wait is raced against a
 *  deadline so the view cannot sit in "watching" for ever. */
async function runWatch(minutes: number, actions: ViewActions) {
  if (State.watching) return;
  // The arm that opened the explanation is spent the moment the run starts; leaving it set is what
  // put the start screen back on top of a finished report.
  State.watchPending = null;
  State.watching = true;
  State.watchMinutes = minutes;
  State.watchStartAt = Date.now();
  State.notify();
  const deadline = new Promise<null>((resolve) => {
    window.setTimeout(() => resolve(null), (minutes * 60 + 45) * 1000);
  });
  try {
    const result = await Promise.race([Bridge.healthWatch(minutes), deadline]);
    if (result === null) {
      void Bridge.log(`watch ${minutes} min: no answer past its deadline`);
    } else {
      State.watch = result;
      const longest = result.stalls.reduce((max, s) => Math.max(max, s.ms), 0);
      void Bridge.log(
        `watch ${minutes} min: ${result.stalls.length} stalls, longest ${longest} ms${result.cancelled ? ", cancelled" : ""}`,
      );
    }
  } catch (e) {
    void Bridge.log(`watch failed: ${e instanceof Error ? e.message : String(e)}`);
  } finally {
    State.watching = false;
    State.watchPending = null;
    State.notify();
    // A watch runs for minutes, so its result must come to the user rather than wait in a view they
    // have to remember to open.
    if (State.watch) actions.watchFinished();
  }
}

function buildWatch(actions: ViewActions): ViewHost {
  const head = h("div", { class: "panel-head" });
  const body = h("div", { class: "panel-body" });
  const foot = h("div", { class: "panel-foot" });
  const progress = h("div", { class: "sub" });
  const el = h(
    "div",
    { class: "view" },
    card(null, h("div", { class: "panel-view" }, head, body, foot)),
  );
  let key = "";

  return {
    el,
    tick() {
      // The one thing that changes every second: how much of the watch is left. Past zero the
      // samples are still coming back, so the line says that instead of a frozen clock.
      if (!State.watching) {
        // A finished watch keeps its last countdown for ever unless it is taken away.
        if (progress.textContent !== "") progress.textContent = "";
        return;
      }
      const total = State.watchMinutes * 60;
      const done = Math.floor((Date.now() - State.watchStartAt) / 1000);
      const left = Math.max(0, total - done);
      if (left === 0) {
        progress.textContent = t("watch.finishing");
        return;
      }
      const clock = `${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`;
      progress.textContent = t("watch.running", clock);
    },
    sync() {
      const watch = State.watch;
      const next = [
        State.watching,
        State.watchPending ?? 0,
        watch?.started ?? 0,
        watch?.stalls.length ?? -1,
        watch?.minutes ?? 0,
      ].join("|");
      if (next === key) return;
      key = next;
      clear(head);
      clear(body);
      clear(foot);

      head.append(
        h(
          "div",
          { class: "grow" },
          h("div", { class: "eyebrow", text: t("watch.eyebrow") }),
          h("div", {
            class: "title",
            text: State.watching
              ? t("watch.watching")
              : watch
                ? t("watch.stalls", watch.stalls.length, watch.stalls.reduce((max, s) => Math.max(max, s.ms), 0))
                : t("watch.title"),
          }),
          progress,
        ),
        h("button", { class: "ghost close", text: "×", title: t("health.close"), onclick: () => actions.collapse() }),
      );

      // One short paragraph before it starts: what a watch does, and that the machine stays usable.
      // Two sentences and two buttons, so the decision is made without reading a manual.
      if (State.watchPending !== null && !State.watching) {
        body.append(
          h("div", { class: "panel-advice tall", text: t("watch.intro") }),
        );
        // The two lengths are the confirmation: pressing one says "I read this, start it".
        foot.append(
          h("button", {
            class: "primary",
            text: t("watch.minutes", 15),
            onclick: () => {
              State.watchPending = null;
              void runWatch(15, actions);
            },
          }),
          h("button", {
            class: "primary",
            text: t("watch.minutes", 30),
            onclick: () => {
              State.watchPending = null;
              void runWatch(30, actions);
            },
          }),
          h("button", {
            class: "ghost",
            text: t("health.cancel"),
            onclick: () => {
              State.watchPending = null;
              State.notify();
            },
          }),
        );
        return;
      }

      if (!watch) {
        body.append(h("div", { class: "panel-advice", text: State.watching ? "" : t("watch.none") }));
      } else {
        if (watch.stalls.length === 0) {
          body.append(h("div", { class: "panel-advice", text: t("watch.clean") }));
        }
        // What the stops had in common, in one line. Twelve timestamps with the same program under
        // each of them is a wall of text that never says the thing it is trying to say.
        const parts: string[] = [];
        for (const c of watch.culprits ?? []) {
          parts.push(t("watch.part", c.name, c.count, Math.round(c.pct)));
        }
        if (watch.driverStalls) parts.push(t("watch.partDriver", watch.driverStalls));
        if (watch.blindStalls) parts.push(t("watch.partBlind", watch.blindStalls));
        if (parts.length) {
          body.append(
            h("div", { class: "panel-advice", text: t("watch.summary", watch.stalls.length, parts.join(" · ")) }),
          );
        }

        const shownStalls = watch.stalls.slice(0, 5);
        shownStalls.forEach((stall, i) => {
          // The culprit is printed only where it changes: five identical lines say nothing the
          // summary above has not already said.
          const previous = i > 0 ? shownStalls[i - 1] : null;
          const sameCulprit =
            previous !== null &&
            previous.top === stall.top &&
            Math.round(previous.topPct) === Math.round(stall.topPct) &&
            (previous.dpc >= 8) === (stall.dpc >= 8);
          body.append(
            h(
              "div",
              { class: "panel-row warn" },
              h("i", { class: "sev" }),
              h(
                "span",
                { class: "text" },
                t("watch.stallLine", stall.atText, stall.ms),
                // A stop with heavy driver time is a driver's stop, and saying so is the whole point
                // of measuring DPC: audiodg at 4 % is not what holds a machine for three seconds.
                sameCulprit
                  ? null
                  : stall.dpc >= 8
                    ? h("span", { class: "cause", text: t("watch.dpc", Math.round(stall.dpc)) })
                    : stall.top
                      ? h("span", {
                          class: "cause",
                          text:
                            t("watch.culprit", stall.top, Math.round(stall.topPct)) +
                            // The second name is where an audio suite or an overlay appears: the
                            // game is always first, and the thing that actually stalled it second.
                            (stall.tops?.length > 1 && stall.tops[1].pct >= 2
                              ? t("watch.also", stall.tops[1].name, Math.round(stall.tops[1].pct))
                              : ""),
                        })
                      : null,
              ),
            ),
          );
        });
        if (watch.stalls.length > shownStalls.length) {
          body.append(
            h("div", {
              class: "panel-advice",
              text: t("watch.more", watch.stalls.length - shownStalls.length),
            }),
          );
        }
        // The summary line: driver time first when it is high, because that is the answer to "why did
        // everything stop" far more often than any program name.
        if (watch.stalls.length >= 3 && watch.gapMinutes > 0) {
          body.append(
            h("div", {
              class: "panel-advice",
              text: t("watch.cadence", watch.gapMinutes),
            }),
          );
        }
        if (watch.dpcPeak >= 8) {
          body.append(
            h("div", { class: "panel-advice", text: t("watch.dpcPeak", Math.round(watch.dpcPeak)) }),
          );
          // Naming what is installed is the useful half of "it is a driver": the list of driver
          // suites on this machine is short, and one of them was installed recently.
          if (watch.suspects?.length) {
            body.append(
              h("div", {
                class: "panel-advice",
                text: t("watch.suspects", watch.suspects.join(", ")),
              }),
            );
          }
        } else if ((watch.culprits?.length ?? 0) === 1 && (watch.culprits[0].count ?? 0) >= 2) {
          body.append(
            h("div", {
              class: "panel-advice",
              text: t("watch.sameCulprit", watch.culprits[0].name),
            }),
          );
        } else if (watch.busiest) {
          body.append(
            h("div", {
              class: "panel-advice",
              text: t(
                "watch.busiest",
                watch.busiest,
                Math.round(watch.busiestPct),
                watch.cores ?? 1,
                Math.round(watch.cpuAvg),
              ),
            }),
          );
        }
      }

      // While it runs the only useful button is the one that stops it; when it is done the report
      // stays on screen until "Jasne".
      const controls: HTMLElement[] = State.watching
        ? [
            h("button", {
              class: "ghost",
              text: t("watch.cancel"),
              onclick: () => void Bridge.healthWatchCancel(),
            }),
          ]
        : [
            // Two lengths, because a freeze can be hours apart: fifteen minutes for "it happens all
            // the time", half an hour for "I will leave it running while I work".
            // One way in. The length is chosen on the explanation itself, which is where the person
            // has just read what a watch does — not two clicks earlier, guessing.
            h("button", {
              class: "ghost",
              text: t("watch.start"),
              onclick: () => {
                State.watchPending = 15;
                State.notify();
              },
            }),
          ];
      if (!State.watching && watch && watch.stalls.length > 0) {
        controls.push(
          h("button", {
            class: "primary",
            text: t("health.repair"),
            onclick: () => {
              const list = watch.stalls
                .slice(0, 3)
                .map((stall) => t("watch.stallLine", stall.atText, stall.ms))
                .join("; ");
              actions.setView("prompt");
              window.setTimeout(() => {
                const field = document.querySelector(".chat-input") as HTMLInputElement | null;
                if (!field) return;
                field.value = t("watch.repairAsk", list);
                field.focus();
                field.setSelectionRange(field.value.length, field.value.length);
                State.notify();
              }, 160);
            },
          }),
        );
      }
      if (!State.watching) {
        controls.push(
          h("button", { class: "primary", text: t("watch.ok"), onclick: () => actions.collapse() }),
        );
      }
      foot.append(
        h("span", { class: "count", text: watch ? watch.startedText : "" }),
        ...controls,
      );
    },
  };
}

// ── Prices ────────────────────────────────────────────────────────────────────

/** A game's price card: the title in, the numbers from gg.deals out. The cover is Steam's own
 *  header for the App ID the lookup settled on, so a wrong match is visible at a glance — and it
 *  is a plain remote image (the CSP lets that host through), hidden until it has decoded so a
 *  cover that never arrives leaves no empty frame behind. */
function buildPrices(actions: ViewActions): ViewHost {
  const input = h("input", {
    class: "pr-input",
    type: "text",
    placeholder: t("prices.placeholder"),
    spellcheck: "false",
  }) as HTMLInputElement;
  const go = h("button", { class: "pr-go" }, t("prices.go"));

  const cover = h("img", { class: "pr-cover", alt: "" }) as HTMLImageElement;
  cover.style.display = "none";
  cover.addEventListener("load", () => {
    cover.style.display = "";
  });
  cover.addEventListener("error", () => {
    cover.style.display = "none";
  });

  const title = h("div", { class: "title" });
  // Two numbers side by side, because the whole point is the comparison: what a key costs today
  // against what Steam charges for it. The record lows sit underneath, once each.
  const keyNow = h("div", { class: "pr-big" });
  const steamNow = h("div", { class: "pr-big" });
  const keyLow = h("div", { class: "pr-low" });
  const steamLow = h("div", { class: "pr-low" });
  const note = h("div", { class: "pr-note" });
  const body = h(
    "div",
    { class: "pr-body" },
    cover,
    h(
      "div",
      { class: "pr-main" },
      title,
      h(
        "div",
        { class: "pr-bigs" },
        h("div", { class: "pr-bigwrap" }, keyNow, h("div", { class: "pr-cap" }, t("prices.keyshops"))),
        h("div", { class: "pr-bigwrap" }, steamNow, h("div", { class: "pr-cap" }, t("prices.shops"))),
      ),
      h("div", { class: "pr-lows" }, keyLow, steamLow),
    ),
  );
  const openPage = btn(t("prices.open"), "primary", () => {
    const url = State.prices?.url;
    if (url) actions.openUrl(url);
  });
  const openSteam = btn(t("prices.steam"), "secondary", () => {
    const id = State.prices?.appId;
    if (id) actions.openUrl(`https://store.steampowered.com/app/${id}/`);
  });
  const actionsRow = h("div", { class: "pr-actions" }, openPage, openSteam);

  /** A price with the currency the key reports — never a converted one. */
  const money = (value: string | null | undefined): string =>
    value ? `${value.replace(".", ",")} ${State.prices?.currency ?? ""}`.trim() : "—";

  async function search() {
    const text = input.value.trim();
    void Bridge.log(`prices search: „${text}”`);
    if (!text) {
      State.pricesError = t("prices.empty");
      paint();
      return;
    }
    if (State.pricesBusy) return;
    State.pricesBusy = true;
    State.pricesError = null;
    // A lookup is a request: the island has to stay out while it runs, or the answer lands on a
    // card nobody can see.
    actions.holdOpen(0);
    paint();
    try {
      State.prices = await Bridge.pricesLookup(text);
    } catch (e) {
      State.pricesError = e instanceof Error ? e.message : String(e);
    } finally {
      State.pricesBusy = false;
      paint();
      // Long enough to read four numbers, not so long the card squats on the screen.
      actions.holdOpen(45);
    }
  }

  function paint() {
    const offer = State.prices;
    const has = Boolean(offer);
    body.style.display = has ? "" : "none";
    actionsRow.style.display = has ? "" : "none";
    go.disabled = State.pricesBusy;

    // The loudest line wins: an error, then the lookup running, then what the card is for.
    note.textContent = State.pricesError
      ? State.pricesError
      : State.pricesBusy
        ? t("prices.busy")
        : has
          ? ""
          : t("prices.hint");
    note.classList.toggle("bad", Boolean(State.pricesError));
    note.style.display = note.textContent ? "" : "none";

    if (!offer) return;
    title.textContent = offer.title;
    keyNow.textContent = money(offer.keyshops);
    steamNow.textContent = money(offer.retail);
    keyLow.textContent = `${t("prices.lowKeyshops")} ${money(offer.historicalKeyshops)}`;
    steamLow.textContent = `${t("prices.lowShops")} ${money(offer.historicalRetail)}`;
    if (offer.image) {
      if (cover.dataset.for !== offer.image) {
        cover.dataset.for = offer.image;
        cover.style.display = "none";
        cover.src = offer.image;
      }
    } else {
      cover.style.display = "none";
    }
  }

  go.addEventListener("click", () => void search());
  input.addEventListener("keydown", (e) => {
    if ((e as KeyboardEvent).key === "Enter") void search();
  });

  const el = h("div", { class: "view" }, card("amber", h("div", { class: "pr" },
    h("div", { class: "pr-top" }, input, go), body, note, actionsRow)));

  return {
    el,
    sync: paint,
    focus() {
      input.focus();
      input.select();
    },
  };
}

export function buildViews(
  actions: ViewActions,
  onChatHeightChange: () => void,
): Map<IslandViewName, ViewHost> {
  const map = new Map<IslandViewName, ViewHost>();
  map.set("home", buildHome(actions));
  map.set("overview", buildOverview(actions));
  map.set("empty", buildEmpty(actions));
  map.set("confused", buildConfused());
  map.set("note", buildNote());
  map.set("listening", buildListening());
  map.set("reminder", buildReminder(actions));
  map.set("routine", buildRoutine(actions, false));
  map.set("routineFull", buildRoutine(actions, true));
  map.set("whatsnew", buildWhatsNew(actions));
  map.set("settings", buildSettings(actions));
  map.set("prompt", buildPrompt(actions, onChatHeightChange));
  map.set("upload", buildUpload());
  map.set("uploading", buildUploading());
  map.set("choose", buildChoose(actions));
  map.set("prices", buildPrices(actions));
  map.set("pc", buildPc(actions));
  map.set("health", buildHealth(actions));
  map.set("report", buildReport(actions));
  map.set("debloat", buildDebloat(actions));
  map.set("watch", buildWatch(actions));
  // Not in the Windows v1: sending a file by email, window attach + web result.
  map.set("mail", buildPlaceholder(t("empty.noEmail"), ""));
  map.set("searching", buildPlaceholder(t("view.searching"), ""));
  map.set("result", buildPlaceholder(t("view.result"), ""));
  return map;
}

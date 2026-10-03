// DOM ports of the Swift island views: paddings, font sizes, colours and wording follow them.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { Bridge } from "../core/bridge";
import { State, type AgentTask } from "../core/state";
import { washRGBA, type IslandViewName, type Wash } from "../core/layout";
import { t } from "../core/i18n";
import { createMiniBot, pruneMiniBots } from "../bot/minibots";
import { buildPrompt } from "./chat";
import { buildChoose, buildUpload, buildUploading } from "./upload";
import { renderIntegrationCard, type IntegrationCardHooks } from "./integrations";

export interface ViewActions {
  setView(v: IslandViewName): void;
  collapse(): void;
  setFocus(id: string): void;
  /** The eye: snip a region of the screen and ask about it. */
  snip(): void;
  holdOpen(seconds: number): void;
  /** The ↗ button: opens whatever the focused pill points at. */
  openTarget(): void;
  openUrl(url: string): void;
  setAutoClose(seconds: number): void;
  openSettingsWindow(): void;
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
    },
  };
}

// ── Home ──────────────────────────────────────────────────────────────────────

function buildHome(actions: ViewActions): ViewHost {
  const pills = h("div", { class: "pills" });
  // The eye is the one thing here that must not open the chat: it works on the
  // screen, not in the conversation.
  const eye = h(
    "button",
    { class: "snip-btn", title: t("chat.snipTip") },
    svg(ICONS.eye, 13),
  );
  eye.addEventListener("mousedown", (e) => {
    e.stopPropagation();
    actions.snip();
  });
  const fresh = h(
    "button",
    { class: "snip-btn", title: t("chat.newTip") },
    svg(ICONS.plus, 12),
  );
  fresh.addEventListener("mousedown", (e) => {
    e.stopPropagation();
    State.chatHistory = [];
    State.snip = null;
    State.droppedFile = null;
    State.chatLastActivity = Date.now();
    void Bridge.chatReset();
    actions.blip();
    State.notify();
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

  const shortcuts = h(
    "div",
    { class: "home-shortcuts" },
    h("span", { text: t("chat.snipHint") }),
  );

  const el = h(
    "div",
    { class: "view home" },
    h("div", { class: "card wash home-card" },
      h("div", { class: "home-body" },
        // Empty on purpose: Iskra is drawn here by the canvas, not by the DOM.
        h("div", { class: "home-left" }),
        h("div", { class: "home-right" }, pills, shortcuts, bar),
      ),
    ),
  );
  (el.querySelector(".card") as HTMLElement).style.setProperty("--wash", "rgba(99,102,241,0.5)");

  let pillKey = "";
  return {
    el,
    sync() {
      // The plus only means anything once there is a conversation to clear.
      const hasSession =
        State.chatHistory.length > 0 || State.snip != null || State.droppedFile != null;
      fresh.style.opacity = hasSession ? "1" : "0";
      fresh.style.pointerEvents = hasSession ? "auto" : "none";

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
  const right = card(null, pills);

  const el = h("div", { class: "view overview" },
    h("div", { class: "left" }, left),
    h("div", { class: "right" }, right),
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

      jump.style.display = detailOpen ? "none" : "";

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
      // service and show its card.
      onclick: () => {
        actions.setFocus(task.id);
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
    btn(t("chat.askButton"), "primary", () => actions.setView("prompt")),
  );
  return { el: h("div", { class: "view" }, card(null, body)), sync() {} };
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

// ── In-island settings ────────────────────────────────────────────────────────

function buildSettings(actions: ViewActions): ViewHost {
  const autoLabel = h("span", {});
  const segButtons = [10, 15, 30].map((s) =>
    h("button", { onclick: () => actions.setAutoClose(s) }, `${s}s`),
  );
  // The island may only ask whether the key exists, never read it, so the
  // badge is refreshed from Rust instead of from State.
  const keyBadge = h("span", { class: "status-badge" });
  let hasKey = false;
  let lastKeyCheck = -Infinity;
  async function refreshKey() {
    lastKeyCheck = performance.now();
    hasKey = (await Bridge.secretPresent("deepseek-api-key")) ?? false;
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
      { class: "settings-row", style: "gap:14px" },
      keyBadge,
      h("div", { class: "grow" }),
      h("button", {
        class: "link-btn",
        style: "color:#8e939c;font-size:11.5px",
        text: t("int.settings"),
        onclick: () => actions.openSettingsWindow(),
      }),
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
      // Re-check while the view is open, so saving a key in the settings window
      // shows up without a restart.
      if (performance.now() - lastKeyCheck > 1500) void refreshKey();
      clear(keyBadge);
      keyBadge.append(dot(hasKey ? "#22C55E" : "#F4505E", 6), h("span", { text: "DeepSeek" }));
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
  map.set("settings", buildSettings(actions));
  map.set("prompt", buildPrompt(actions, onChatHeightChange));
  map.set("upload", buildUpload());
  map.set("uploading", buildUploading());
  map.set("choose", buildChoose(actions));
  // Not in the Windows v1: sending a file by email, window attach + web result.
  map.set("mail", buildPlaceholder(t("empty.noEmail"), ""));
  map.set("searching", buildPlaceholder("Szukam…", ""));
  map.set("result", buildPlaceholder("Wynik", ""));
  return map;
}

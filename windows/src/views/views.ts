// DOM ports of the Swift island views: paddings, font sizes, colours and wording follow them.

import { h, svg, clear, dot } from "./dom";
import { ICONS } from "./icons";
import { Bridge } from "../core/bridge";
import { State, type AgentTask } from "../core/state";
import { washRGBA, type IslandViewName, type Wash } from "../core/layout";
import { t } from "../core/i18n";
import { createMiniBot, pruneMiniBots } from "../bot/minibots";
import { buildPrompt, bubble } from "./chat";
import { buildChoose, buildUpload, buildUploading } from "./upload";
import { renderIntegrationCard, type IntegrationCardHooks } from "./integrations";

export interface ViewActions {
  setView(v: IslandViewName): void;
  collapse(): void;
  setFocus(id: string): void;
  /** The eye: snip a region of the screen and ask about it. */
  snip(): void;
  /** Keep the island open for `seconds` (0 = until released): a chat turn takes as
   *  long as it takes, and the answer must stay readable. */
  holdOpen(seconds: number): void;
  /** The ↗ button: opens whatever the focused pill points at. */
  openTarget(): void;
  openUrl(url: string): void;
  setAutoClose(seconds: number): void;
  setNotchHide(seconds: number): void;
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
  const fresh = h(
    "button",
    { class: "new-chat-btn", title: t("chat.newTip") },
    svg(ICONS.plus, 11),
    h("span", { text: t("chat.new") }),
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

/** The island's home screen: Iskra, the integration pills above the bar, and the bar
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
        // Empty on purpose: Iskra is drawn here by the canvas, not by the DOM.
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
  map.set("whatsnew", buildWhatsNew(actions));
  map.set("settings", buildSettings(actions));
  map.set("prompt", buildPrompt(actions, onChatHeightChange));
  map.set("upload", buildUpload());
  map.set("uploading", buildUploading());
  map.set("choose", buildChoose(actions));
  // Not in the Windows v1: sending a file by email, window attach + web result.
  map.set("mail", buildPlaceholder(t("empty.noEmail"), ""));
  map.set("searching", buildPlaceholder(t("view.searching"), ""));
  map.set("result", buildPlaceholder(t("view.result"), ""));
  return map;
}

// The debloat window: what Windows brought with it, and taking it away — one ticked box at a time.
//
// Nothing here decides anything by itself. The list is what the catalogue is willing to offer
// (src-tauri/src/debloat.rs), safe packages are pre-ticked and careful ones are not, and the last
// step is always a panel naming exactly what will disappear. The answer afterwards comes from
// Windows, not from the script: a package still installed is reported as kept.

import "../pc/pc.css";
import { Bridge, onEvent } from "../core/bridge";
import { setLang, t } from "../core/i18n";
import { h, clear } from "../views/dom";
import { debloatLabel as label } from "../core/debloat-labels";
import type { DebloatItem, DebloatRemoved, GameBarState } from "../core/bridge";

const root = document.getElementById("pc-root")!;

/** The one package the gaming card owns by name; the catalogue still lists it below. */
const GAME_BAR = "Microsoft.XboxGamingOverlay";

let items: DebloatItem[] = [];
let history: DebloatRemoved[] = [];
let ticked = new Set<string>();
let busy = false;
let loading = false;
/** The names waiting for a confirmation, and the outcome of the last removal. */
let pending: string[] = [];
let result: { removed: string[]; failed: string[]; error?: string } | null = null;
/** Game Bar and Game DVR as Windows reports them, which button is running, and what it said. */
let game: GameBarState | null = null;
let gameBusy: "capture" | "install" | "remove" | "admin" | null = null;
let gameNote: { text: string; bad?: boolean } | null = null;

function sectionFor(tier: "safe" | "careful"): HTMLElement | null {
  const list = items.filter((i) => i.tier === tier);
  if (list.length === 0) return null;
  const rows = list.map((item) => {
    const box = h("input", { type: "checkbox" }) as HTMLInputElement;
    box.checked = tier === "safe";
    box.addEventListener("change", () => {
      if (box.checked) ticked.add(item.name);
      else ticked.delete(item.name);
      // The footer follows the boxes without a repaint, which keeps the scroll where it is: the
      // count says how many are ticked, the button says what pressing it does — with the number
      // written into it, because that button is the only place the action is named.
      const count = document.querySelector(".footer .count");
      if (count) count.textContent = t("debloat.ticked", ticked.size);
      const remove = document.getElementById("do-remove") as HTMLButtonElement | null;
      if (remove) {
        remove.textContent = t("debloat.remove", ticked.size);
        remove.disabled = ticked.size === 0 || busy;
      }
    });
    if (box.checked) ticked.add(item.name);
    return h(
      "label",
      { class: "app" },
      box,
      // The readable name first, the package id underneath it: the id is context, and putting it
      // in a column of its own left a ragged gap under every short name.
      h(
        "span",
        { class: "name" },
        h("strong", { text: label(item.name) }),
        h("span", { class: "pkg", text: item.name }),
      ),
    );
  });
  return h(
    "section",
    {},
    h("h2", { text: t(tier === "safe" ? "debloat.safe" : "debloat.careful") }),
    h("div", { class: "hint", text: t(tier === "safe" ? "debloat.safeHint" : "debloat.carefulHint") }),
    ...rows,
  );
}

// ── The gaming card ───────────────────────────────────────────────────────────
// Two things Windows turns on by itself and a person may not want: the Win+G overlay and the
// capture that records behind a game. Both are read back after every change, so the card shows
// what Windows has — including the machine level, which an ordinary account cannot write and which
// therefore says so instead of pretending.

function barRow(state: GameBarState): HTMLElement {
  const pkg = h("span", {
    class: "pkg",
    text: state.version ? `${GAME_BAR} · ${state.version}` : GAME_BAR,
  });
  const badge = h("span", {
    class: state.installed ? "pill ok" : "pill off",
    text: t(state.installed ? "game.present" : "game.absent"),
  });
  const action = state.installed
    ? h("button", {
        class: "ghost danger",
        text: gameBusy === "remove" ? t("game.removing") : t("game.uninstall"),
        disabled: gameBusy !== null,
        onclick: () => void removeBar(),
      })
    : h("button", {
        class: "primary",
        text: gameBusy === "install" ? t("game.installing") : t("game.install"),
        disabled: gameBusy !== null,
        onclick: () => void installBar(),
      });
  return h(
    "div",
    { class: "tweak" },
    h(
      "div",
      { class: "head" },
      h("span", { class: "title" }, h("strong", { text: t("game.bar") }), pkg),
      badge,
      action,
    ),
    h("div", { class: "body", text: t("game.barHint") }),
  );
}

function captureRow(state: GameBarState): HTMLElement {
  const sw = h("button", {
    class: state.capture ? "switch on" : "switch",
    "aria-pressed": state.capture,
    title: t("game.capture"),
    disabled: gameBusy !== null,
    onclick: () => void setCapture(!state.capture),
  });
  const row = h(
    "div",
    { class: "tweak" },
    h(
      "div",
      { class: "head" },
      h(
        "span",
        { class: "title" },
        h("strong", { text: t("game.capture") }),
        h("span", { class: "pkg", text: t(state.capture ? "game.dvrOn" : "game.dvrOff") }),
      ),
      sw,
    ),
    h("div", {
      class: state.capture ? "body on" : "body",
      text: t(state.capture ? "game.captureOn" : "game.captureOff"),
    }),
    h("div", { class: "hint", text: t("game.captureHint") }),
  );
  // Only offered when the machine level really refused the write, and only for the state the
  // toggle is now in: a UAC prompt nobody can explain is worse than a switch that stops one level
  // short and says so.
  if (state.needsAdmin) {
    row.append(
      h(
        "div",
        { class: "admin" },
        h("span", { text: t("game.needsAdmin") }),
        h("button", {
          class: "ghost",
          text: gameBusy === "admin" ? t("game.adminBusy") : t("game.raiseAdmin"),
          disabled: gameBusy !== null,
          onclick: () => void raiseAdmin(!state.capture),
        }),
      ),
    );
  }
  return row;
}

function gamesSection(): HTMLElement {
  const card = h(
    "section",
    { class: "games" },
    h("h2", { text: t("debloat.games") }),
    h("div", { class: "hint", text: t("debloat.gamesHint") }),
  );
  if (!game) {
    card.append(h("div", { class: "hint", text: "…" }));
    return card;
  }
  card.append(barRow(game), captureRow(game));
  if (gameNote && gameNote.text) {
    card.append(h("div", { class: gameNote.bad ? "result bad" : "result", text: gameNote.text }));
  }
  return card;
}

/** Runs one action on the card: busy flag on, the answer read back, one line about what happened. */
async function gameAction(
  which: "capture" | "install" | "remove" | "admin",
  run: () => Promise<GameBarState>,
  said: string,
) {
  if (gameBusy) return;
  gameBusy = which;
  gameNote = null;
  paint();
  try {
    game = await run();
    gameNote = { text: said };
    void Bridge.log(`debloat window: game bar ${which} ok`);
  } catch (e) {
    gameNote = { text: t("game.failed", e instanceof Error ? e.message : String(e)), bad: true };
  } finally {
    gameBusy = null;
  }
  paint();
}

function setCapture(on: boolean) {
  return gameAction(
    "capture",
    () => Bridge.gamebarCapture(on),
    // Nothing to announce: the switch and its status line already carry the new state, and a note
    // repeating it would be the third place saying the same thing.
    "",
  );
}

function installBar() {
  return gameAction("install", () => Bridge.gamebarInstall(), t("game.back"));
}

function raiseAdmin(on: boolean) {
  return gameAction("admin", () => Bridge.gamebarPolicyElevated(on), t("game.adminDone"));
}

/** Removal goes through the same command the package list uses; afterwards the list is re-read,
 *  because Game Bar is in it and a package that is gone cannot stay ticked. */
async function removeBar() {
  if (gameBusy) return;
  gameBusy = "remove";
  gameNote = null;
  paint();
  try {
    const outcome = await Bridge.debloatRemove([GAME_BAR]);
    gameNote =
      outcome.removed.length > 0
        ? { text: t("game.gone") }
        : { text: t("game.failed", outcome.failed.join(", ")), bad: true };
  } catch (e) {
    gameNote = { text: t("game.failed", e instanceof Error ? e.message : String(e)), bad: true };
  } finally {
    gameBusy = null;
  }
  await load();
}

function paint() {
  clear(root);

  const refresh = h("button", {
    class: "ghost",
    text: t("debloat.reload"),
    onclick: () => void load(),
  }) as HTMLButtonElement;
  refresh.disabled = loading || busy;
  root.append(
    h(
      "header",
      {},
      h(
        "div",
        { class: "grow" },
        h("h1", { text: t("debloat.title") }),
        h("div", { class: "sub", text: t("debloat.sub") }),
      ),
      refresh,
    ),
  );

  if (loading) {
    root.append(gamesSection());
    root.append(h("section", {}, h("div", { class: "hint", text: "…" })));
    return;
  }

  const safe = sectionFor("safe");
  const careful = sectionFor("careful");
  root.append(gamesSection());
  if (!safe && !careful) {
    root.append(h("section", {}, h("div", { class: "hint", text: t("debloat.empty") })));
  } else {
    if (safe) root.append(safe);
    if (careful) root.append(careful);
  }

  // The confirmation names what will go, because a count is not something anyone can check.
  if (pending.length > 0) {
    root.append(
      h(
        "div",
        { class: "confirm" },
        h("div", { text: t("debloat.confirm", pending.length) }),
        h("div", { class: "names", text: pending.map(label).join("\n") }),
        h(
          "div",
          { class: "row" },
          h("button", {
            class: "danger",
            text: busy ? t("debloat.busy") : t("debloat.confirmYes"),
            onclick: () => void remove(),
          }),
          h("button", {
            class: "ghost",
            text: t("debloat.cancel"),
            onclick: () => {
              pending = [];
              paint();
            },
          }),
        ),
      ),
    );
  } else {
    root.append(
      h(
        "div",
        { class: "footer" },
        h("div", { class: "count", text: t("debloat.ticked", ticked.size) }),
        h("button", {
          id: "do-remove",
          class: "primary",
          text: busy ? t("debloat.busy") : t("debloat.remove", ticked.size),
          disabled: ticked.size === 0 || busy,
          onclick: () => {
            pending = [...ticked];
            paint();
          },
        }) as HTMLButtonElement,
      ),
    );
  }

  if (result) {
    root.append(
      h(
        "section",
        {},
        h("h2", { text: t("debloat.remove", result.removed.length + result.failed.length) }),
        result.removed.length > 0
          ? h("div", { class: "result", text: t("debloat.done", result.removed.map(label).join(", ")) })
          : "",
        result.failed.length > 0
          ? h("div", {
              class: "result bad",
              text: t("debloat.failed", result.failed.map(label).join(", ")),
            })
          : "",
        result.error ? h("div", { class: "result bad", text: result.error }) : "",
      ),
    );
  }

  root.append(
    h(
      "section",
      {},
      h("h2", { text: t("debloat.history") }),
      history.length === 0
        ? h("div", { class: "hint", text: t("debloat.noHistory") })
        : h(
            "div",
            { class: "history" },
            ...history.map((row) =>
              h(
                "div",
                { class: "row" },
                h("span", { class: "name", text: label(row.name) }),
                // A removed package comes back from the Store, which is the only honest way to
                // offer it: the provisioned copy is not something this app should promise.
                h("button", {
                  class: "ghost",
                  text: t("debloat.restore"),
                  onclick: () =>
                    void Bridge.openUrl(
                      `ms-windows-store://search/?query=${encodeURIComponent(label(row.name))}`,
                    ),
                }),
              ),
            ),
          ),
    ),
  );
}

async function load() {
  loading = true;
  paint();
  try {
    items = await Bridge.debloatList();
    history = (await Bridge.debloatHistory()) ?? [];
    // A package already gone cannot stay ticked: the list is what Windows reports right now.
    ticked = new Set(items.filter((i) => i.tier === "safe").map((i) => i.name));
    void Bridge.log(`debloat window: ${items.length} offerable packages`);
  } catch (e) {
    result = {
      removed: [],
      failed: [],
      error: e instanceof Error ? e.message : String(e),
    };
  }
  // The gaming card is its own reading and its own failure: a registry or Appx answer that comes
  // back empty leaves the package list above untouched rather than blanking the window.
  try {
    game = await Bridge.gamebarStatus();
  } catch (e) {
    game = null;
    gameNote = { text: t("game.failed", e instanceof Error ? e.message : String(e)), bad: true };
  }
  loading = false;
  paint();
}

async function remove() {
  if (busy || pending.length === 0) return;
  busy = true;
  paint();
  try {
    const outcome = await Bridge.debloatRemove(pending);
    result = { removed: outcome.removed, failed: outcome.failed };
    void Bridge.log(
      `debloat window remove: ${outcome.removed.length} removed, ${outcome.failed.length} kept`,
    );
    pending = [];
  } catch (e) {
    result = { removed: [], failed: [], error: e instanceof Error ? e.message : String(e) };
  } finally {
    busy = false;
  }
  await load();
}

async function boot() {
  const info = await Bridge.boot();
  if (info) setLang(info.settings.language);
  await load();
  await onEvent("pc-shown", () => {
    void load();
  });
}

void boot();

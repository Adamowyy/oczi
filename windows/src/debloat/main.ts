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
import type { DebloatItem, DebloatRemoved } from "../core/bridge";

const root = document.getElementById("pc-root")!;

let items: DebloatItem[] = [];
let history: DebloatRemoved[] = [];
let ticked = new Set<string>();
let busy = false;
let loading = false;
/** The names waiting for a confirmation, and the outcome of the last removal. */
let pending: string[] = [];
let result: { removed: string[]; failed: string[]; error?: string } | null = null;

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
    root.append(h("section", {}, h("div", { class: "hint", text: "…" })));
    return;
  }

  const safe = sectionFor("safe");
  const careful = sectionFor("careful");
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
  } finally {
    loading = false;
    paint();
  }
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

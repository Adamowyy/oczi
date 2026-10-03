// Settings window — the place where anything that writes to disk is confirmed.
// Covers the general preferences, the DeepSeek chat key and the integrations.

import "./settings.css";
import { Bridge, onEvent } from "../core/bridge";
import { DEFAULT_SETTINGS, type Settings } from "../core/state";
import { LANGUAGES, isLang, setLang, t, type TextKey } from "../core/i18n";
import { h, clear } from "../views/dom";

let settings: Settings = { ...DEFAULT_SETTINGS };
let version = "";

const root = document.getElementById("settings-root")!;

async function save() {
  await Bridge.saveSettings(settings);
}

// ── Reusable bits ─────────────────────────────────────────────────────────────

function toggle(on: boolean, onChange: (v: boolean) => void): HTMLElement {
  const el = h("button", { class: on ? "switch on" : "switch", "aria-pressed": on });
  el.addEventListener("click", () => {
    const next = !el.classList.contains("on");
    el.classList.toggle("on", next);
    onChange(next);
  });
  return el;
}

function statusDot(ok: boolean): HTMLElement {
  return h("i", { class: "dot", style: `background:${ok ? "#22c55e" : "#f4505e"}` });
}

// ── DeepSeek API section ──────────────────────────────────────────────────────

const MODELS: [string, string][] = [
  ["deepseek-flash", "DeepSeek Flash"],
  ["deepseek-v4-pro", "DeepSeek V4 Pro"],
];

function apiSection(hasKey: boolean): HTMLElement {
  const dot = statusDot(hasKey);
  const state = h("span", { class: "hint", text: hasKey ? t("set.keyStored") : t("set.keyMissing") });

  const field = h("input", {
    type: "password",
    placeholder: hasKey ? t("set.keyPlaceholderSaved") : "sk-…",
    style: "flex:1 1 auto;min-width:0",
    autocomplete: "off",
    spellcheck: "false",
  }) as HTMLInputElement;

  const saveBtn = h("button", { class: "primary", text: t("set.keySave") });
  const clearBtn = h("button", { class: "danger", text: t("set.keyRemove") });
  const feedback = h("div", {});

  async function refresh() {
    const present = (await Bridge.secretPresent("deepseek-api-key")) ?? false;
    dot.style.background = present ? "#22c55e" : "#f4505e";
    state.textContent = present ? t("set.keyStored") : t("set.keyMissing");
    field.placeholder = present ? t("set.keyPlaceholderSaved") : "sk-…";
    clearBtn.style.display = present ? "" : "none";
  }

  saveBtn.addEventListener("click", async () => {
    const value = field.value.trim();
    if (!value) return;
    clear(feedback);
    try {
      await Bridge.secretSet("deepseek-api-key", value);
      field.value = "";
      feedback.append(h("div", { class: "notice ok", text: t("set.keySaved") }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: t("set.keySaveFailed", String(err)) }));
    }
  });

  clearBtn.addEventListener("click", async () => {
    clear(feedback);
    try {
      await Bridge.secretClear("deepseek-api-key");
      feedback.append(h("div", { class: "notice ok", text: t("set.keyRemoved") }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: t("set.keyRemoveFailed", String(err)) }));
    }
  });

  const model = h("select", {}) as HTMLSelectElement;
  for (const [id, label] of MODELS) model.append(h("option", { value: id, text: label }));
  model.value = settings.model;
  model.addEventListener("change", () => {
    settings.model = model.value;
    void save();
  });

  clearBtn.style.display = hasKey ? "" : "none";

  return h(
    "section",
    {},
    h("h2", {}, dot, h("span", { text: t("set.deepseek") })),
    state,
    h("div", { class: "row" }, h("label", { text: t("set.apiKey") }), field, saveBtn, clearBtn),
    h("div", { class: "row" }, h("label", { text: t("set.model") }), model),
    h("div", { class: "row" },
      h("label", { text: t("set.thinking") }),
      toggle(settings.thinking, (v) => { settings.thinking = v; void save(); }),
      h("span", { class: "hint", text: t("set.thinkingHint") }),
    ),
    feedback,
  );
}

// ── Web access section ────────────────────────────────────────────────────────

const SEARCH_PROVIDERS: [string, string, TextKey][] = [
  ["duckduckgo", "DuckDuckGo", "set.noKeyNeeded"],
  ["brave", "Brave Search", "set.needsKey"],
  ["tavily", "Tavily", "set.needsKey"],
];

interface SecretField {
  key: string;
  labelKey: TextKey;
  placeholder: string;
  secret: boolean;
}

/** Keys only the keyed backends use; DuckDuckGo needs nothing configured. */
const SEARCH_KEYS: SecretField[] = [
  { key: "brave-api-key", labelKey: "set.braveKey", placeholder: "BSA…", secret: true },
  { key: "tavily-api-key", labelKey: "set.tavilyKey", placeholder: "tvly-…", secret: true },
];

/** One credential row: input, save button, status dot. Shared by the
 * integrations and the search backends — same rules for every secret. */
function secretRow(field: SecretField, present: Record<string, boolean>): HTMLElement {
  const input = h("input", {
    type: field.secret ? "password" : "text",
    placeholder: present[field.key] ? t("set.stored") : field.placeholder,
    autocomplete: "off",
    spellcheck: "false",
    style: "flex:1 1 auto;min-width:0",
  }) as HTMLInputElement;
  const saveBtn = h("button", { text: t("set.save") });
  const dotEl = statusDot(present[field.key] ?? false);
  saveBtn.addEventListener("click", async () => {
    const value = input.value.trim();
    try {
      await Bridge.secretSet(field.key, value);
      present[field.key] = value.length > 0;
      input.value = "";
      input.placeholder = value ? t("set.stored") : field.placeholder;
      dotEl.style.background = value ? "#22c55e" : "#f4505e";
    } catch {
      dotEl.style.background = "#f5a524";
    }
  });
  return h("div", { class: "row" },
    h("label", { style: "min-width:104px", text: t(field.labelKey) }),
    input, saveBtn, dotEl,
  );
}

function webSection(present: Record<string, boolean>): HTMLElement {
  const provider = h("select", {}) as HTMLSelectElement;
  for (const [id, label, noteKey] of SEARCH_PROVIDERS) {
    provider.append(h("option", { value: id, text: `${label} — ${t(noteKey)}` }));
  }
  provider.value = settings.searchProvider;
  provider.addEventListener("change", () => {
    settings.searchProvider = provider.value;
    void save();
  });

  const rows = h("div", { style: "display:flex;flex-direction:column;gap:6px;flex:1 1 auto;min-width:0" });
  for (const field of SEARCH_KEYS) rows.append(secretRow(field, present));

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("set.internet") })),
    h("div", { class: "row" },
      h("label", { text: t("set.webSearch") }),
      toggle(settings.webSearch, (v) => { settings.webSearch = v; void save(); }),
      h("span", { class: "hint", text: t("set.webSearchHint") }),
    ),
    h("div", { class: "row" },
      h("label", { text: t("set.searchEngine") }),
      provider,
      h("span", { class: "hint", text: t("set.searchKeysHint") }),
    ),
    rows,
  );
}

// ── Terminal section ──────────────────────────────────────────────────────────

/** Off by default. Turning it on takes two clicks: the warning is the gate. */
function terminalSection(): HTMLElement {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:8px" });

  const warning = () =>
    h("div", { class: "notice err", style: "padding:10px 12px;line-height:1.45;white-space:normal" },
      h("b", { text: t("set.terminalWarnTitle") }),
      h("div", { style: "margin-top:4px", text: t("set.terminalWarn") }),
      h("div", { style: "margin-top:6px;opacity:.8", text: t("set.terminalLog") }),
    );

  function render(confirming: boolean) {
    clear(body);
    const enabled = settings.terminalEnabled;

    body.append(
      h("div", { class: "row" },
        h("label", { text: t("set.terminal") }),
        toggle(enabled, () => (enabled ? render(false) : render(true))),
        h("span", { class: "hint", text: enabled ? t("set.terminalOn") : t("set.terminalOff") }),
      ),
      h("div", { class: "hint", text: t("set.terminalHint") }),
    );

    if (enabled) {
      body.append(
        warning(),
        h("div", { class: "row" },
          h("button", {
            class: "danger",
            text: t("set.terminalDisable"),
            onclick: () => {
              settings.terminalEnabled = false;
              void save();
              render(false);
            },
          }),
        ),
      );
      return;
    }

    if (confirming) {
      body.append(
        warning(),
        h("div", { class: "row" },
          h("button", {
            class: "danger",
            text: t("set.terminalEnable"),
            onclick: () => {
              settings.terminalEnabled = true;
              void save();
              render(false);
            },
          }),
          h("button", { text: t("set.terminalCancel"), onclick: () => render(false) }),
        ),
      );
    }
  }

  render(false);
  return h("section", {}, h("h2", {}, h("span", { text: t("set.terminal") })), body);
}

// ── Integrations section ──────────────────────────────────────────────────────

interface IntegrationDef {
  id: string;
  name: string;
  color: string;
  /** Credential Manager keys, in the order they are shown. */
  fields: SecretField[];
}

const INTEGRATIONS: IntegrationDef[] = [
  { id: "integration_stripe", name: "Stripe", color: "#0570DE",
    fields: [{ key: "stripe-api-key", labelKey: "set.secretKey", placeholder: "sk_live_…", secret: true }] },
  { id: "integration_github", name: "GitHub", color: "#F4505E",
    fields: [{ key: "github-token", labelKey: "set.token", placeholder: "ghp_…", secret: true }] },
  { id: "integration_vercel", name: "Vercel", color: "#7C5CFF",
    fields: [{ key: "vercel-token", labelKey: "set.token", placeholder: "…", secret: true }] },
  { id: "integration_n8n", name: "n8n", color: "#F29B38",
    fields: [
      { key: "n8n-url", labelKey: "set.instanceUrl", placeholder: "https://n8n.example.com", secret: false },
      { key: "n8n-api-key", labelKey: "set.apiKey", placeholder: "…", secret: true },
    ] },
  { id: "integration_resend", name: "Resend", color: "#22C55E",
    fields: [{ key: "resend-api-key", labelKey: "set.apiKey", placeholder: "re_…", secret: true }] },
  { id: "integration_notion", name: "Notion", color: "#8C8C8C",
    fields: [{ key: "notion-api-key", labelKey: "set.integrationToken", placeholder: "ntn_…", secret: true }] },
  { id: "integration_calcom", name: "Cal.com", color: "#C9956A",
    fields: [{ key: "calcom-api-key", labelKey: "set.apiKey", placeholder: "cal_…", secret: true }] },
];

const MAX_ACTIVE = 4;

function integrationsSection(present: Record<string, boolean>): HTMLElement {
  const note = h("div", { class: "hint" });
  const list = h("div", { style: "display:flex;flex-direction:column;gap:14px" });

  function updateNote() {
    const used = settings.activeIntegrations.length;
    note.textContent = t("set.integrationsNote", MAX_ACTIVE, used, MAX_ACTIVE);
  }

  for (const def of INTEGRATIONS) {
    const active = settings.activeIntegrations.includes(def.id);
    const sw = h("button", { class: active ? "switch on" : "switch" });
    sw.addEventListener("click", () => {
      const on = settings.activeIntegrations.includes(def.id);
      if (on) {
        settings.activeIntegrations = settings.activeIntegrations.filter((x) => x !== def.id);
      } else {
        if (settings.activeIntegrations.length >= MAX_ACTIVE) return;
        settings.activeIntegrations = [...settings.activeIntegrations, def.id];
      }
      sw.classList.toggle("on", !on);
      updateNote();
      void save();
    });

    const rows = h("div", { style: "display:flex;flex-direction:column;gap:6px;flex:1 1 auto;min-width:0" });
    for (const field of def.fields) rows.append(secretRow(field, present));

    list.append(
      h("div", { style: "display:flex;gap:12px;align-items:flex-start" },
        h("div", { style: "display:flex;align-items:center;gap:8px;min-width:132px;padding-top:4px" },
          sw,
          h("i", { class: "dot", style: `background:${def.color}` }),
          h("span", { style: "font-size:12.5px", text: def.name }),
        ),
        rows,
      ),
    );
  }

  updateNote();
  return h("section", {}, h("h2", {}, h("span", { text: t("set.integrations") })), note, list);
}

// ── General section ───────────────────────────────────────────────────────────

/** The chords the island can be summoned with. Polish layout is the constraint:
 * Ctrl+Alt is AltGr, so only keys with no AltGr diacritic are offered. */
const HOTKEY_OPTIONS = ["Ctrl+Alt+M", "Ctrl+Alt+Shift+M", "Ctrl+Shift+M", "Ctrl+Alt+G", "Ctrl+Alt+Space"];

function generalSection(): HTMLElement {
  const language = h("select", {}) as HTMLSelectElement;
  for (const { tag, label } of LANGUAGES) language.append(h("option", { value: tag, text: label }));
  language.value = settings.language;
  language.addEventListener("change", () => {
    if (!isLang(language.value)) return;
    settings.language = language.value;
    void save();
    // The texts live in the widgets, so a language change rebuilds the window.
    location.reload();
  });

  const autoClose = h("input", {
    type: "number", min: "5", max: "120", step: "1",
    value: String(Math.round(settings.autoCloseInterval)),
    style: "width:72px",
  }) as HTMLInputElement;
  autoClose.addEventListener("change", () => {
    settings.autoCloseInterval = Math.max(5, Math.min(120, Number(autoClose.value) || 15));
    autoClose.value = String(settings.autoCloseInterval);
    void save();
  });

  const notchHide = h("input", {
    type: "number", min: "5", max: "600", step: "5",
    value: String(Math.round(settings.notchHideInterval)),
    style: "width:72px",
  }) as HTMLInputElement;
  notchHide.addEventListener("change", () => {
    settings.notchHideInterval = Math.max(5, Math.min(600, Number(notchHide.value) || 60));
    notchHide.value = String(settings.notchHideInterval);
    void save();
  });

  const screen = h("select", {}) as HTMLSelectElement;
  screen.append(
    h("option", { value: "primary", text: t("set.screenPrimary") }),
    h("option", { value: "cursor", text: t("set.screenCursor") }),
  );
  screen.value = settings.screen;
  screen.addEventListener("change", () => {
    settings.screen = screen.value as Settings["screen"];
    void save();
  });

  const hotkey = h("select", {}) as HTMLSelectElement;
  for (const opt of HOTKEY_OPTIONS) hotkey.append(h("option", { value: opt, text: opt }));
  hotkey.value = HOTKEY_OPTIONS.includes(settings.hotkey) ? settings.hotkey : HOTKEY_OPTIONS[0];
  hotkey.addEventListener("change", () => {
    settings.hotkey = hotkey.value;
    void save();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("set.general") })),
    h("div", { class: "row" }, h("label", { text: t("set.language") }), language),
    h("div", { class: "row" },
      h("label", { text: t("set.autoClose") }),
      autoClose,
      h("span", { class: "hint", text: t("set.autoCloseHint") }),
    ),
    h("div", { class: "row" },
      h("label", { text: t("set.notchHide") }),
      notchHide,
      h("span", { class: "hint", text: t("set.notchHideHint") }),
    ),
    h("div", { class: "row" }, h("label", { text: t("set.islandScreen") }), screen),
    h("div", { class: "row" }, h("label", { text: t("set.hotkey") }), hotkey),
    h("div", { class: "row" },
      h("label", { text: t("set.autostart") }),
      toggle(settings.autostart, (v) => { settings.autostart = v; void save(); }),
    ),
  );
}

// ── Boot ──────────────────────────────────────────────────────────────────────

async function main() {
  const boot = await Bridge.boot();
  if (boot) {
    settings = { ...settings, ...boot.settings };
    version = boot.version;
  }

  setLang(settings.language);
  document.title = t("set.title");

  const hasKey = (await Bridge.secretPresent("deepseek-api-key")) ?? false;

  const keys = [
    "stripe-api-key", "github-token", "vercel-token",
    "n8n-url", "n8n-api-key", "resend-api-key", "notion-api-key", "calcom-api-key",
    "brave-api-key", "tavily-api-key",
  ];
  const present: Record<string, boolean> = {};
  for (const k of keys) present[k] = (await Bridge.secretPresent(k)) ?? false;

  clear(root);
  root.append(
    h("h1", {}, h("span", { text: "Oczi" }), h("span", { class: "version", text: version })),
    apiSection(hasKey),
    webSection(present),
    terminalSection(),
    integrationsSection(present),
    generalSection(),
    h("div", { class: "hint", text: t("set.privacy") }),
  );

  void onEvent<Settings>("settings-changed", (s) => {
    settings = { ...settings, ...s };
  });
}

void main();

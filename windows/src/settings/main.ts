// Settings window — the place where anything that writes to disk is confirmed.
// Covers the general preferences, the DeepSeek chat key and the integrations.

import "./settings.css";
import { Bridge, onEvent } from "../core/bridge";
import { DEFAULT_SETTINGS, type Settings } from "../core/state";
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
  const state = h("span", { class: "hint", text: hasKey ? "Klucz zapisany w menedżerze poświadczeń Windows." : "Brak klucza — czat go potrzebuje." });

  const field = h("input", {
    type: "password",
    placeholder: hasKey ? "••••••••••••  (zapisany)" : "sk-…",
    style: "flex:1 1 auto;min-width:0",
    autocomplete: "off",
    spellcheck: "false",
  }) as HTMLInputElement;

  const saveBtn = h("button", { class: "primary", text: "Zapisz klucz" });
  const clearBtn = h("button", { class: "danger", text: "Usuń" });
  const feedback = h("div", {});

  async function refresh() {
    const present = (await Bridge.secretPresent("deepseek-api-key")) ?? false;
    dot.style.background = present ? "#22c55e" : "#f4505e";
    state.textContent = present
      ? "Klucz zapisany w menedżerze poświadczeń Windows."
      : "Brak klucza — czat go potrzebuje.";
    field.placeholder = present ? "••••••••••••  (zapisany)" : "sk-…";
    clearBtn.style.display = present ? "" : "none";
  }

  saveBtn.addEventListener("click", async () => {
    const value = field.value.trim();
    if (!value) return;
    clear(feedback);
    try {
      await Bridge.secretSet("deepseek-api-key", value);
      field.value = "";
      feedback.append(h("div", { class: "notice ok", text: "Zapisano. Nigdy nie trafia na dysk." }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: `Nie udało się zapisać: ${String(err)}` }));
    }
  });

  clearBtn.addEventListener("click", async () => {
    clear(feedback);
    try {
      await Bridge.secretClear("deepseek-api-key");
      feedback.append(h("div", { class: "notice ok", text: "Klucz usunięty." }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: `Nie udało się usunąć: ${String(err)}` }));
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
    h("h2", {}, dot, h("span", { text: "DeepSeek" })),
    state,
    h("div", { class: "row" }, h("label", { text: "Klucz API" }), field, saveBtn, clearBtn),
    h("div", { class: "row" }, h("label", { text: "Model" }), model),
    h("div", { class: "row" },
      h("label", { text: "Myślenie" }),
      toggle(settings.thinking, (v) => { settings.thinking = v; void save(); }),
      h("span", { class: "hint", text: "wolniej, ale najpierw myśli" }),
    ),
    feedback,
  );
}

// ── Web access section ────────────────────────────────────────────────────────

const SEARCH_PROVIDERS: [string, string, string][] = [
  ["duckduckgo", "DuckDuckGo", "bez klucza"],
  ["brave", "Brave Search", "wymaga klucza"],
  ["tavily", "Tavily", "wymaga klucza"],
];

/** Keys only the keyed backends use; DuckDuckGo needs nothing configured. */
const SEARCH_KEYS = [
  { key: "brave-api-key", label: "Klucz Brave", placeholder: "BSA…", secret: true },
  { key: "tavily-api-key", label: "Klucz Tavily", placeholder: "tvly-…", secret: true },
];

interface SecretField {
  key: string;
  label: string;
  placeholder: string;
  secret: boolean;
}

/** One credential row: input, save button, status dot. Shared by the
 * integrations and the search backends — same rules for every secret. */
function secretRow(field: SecretField, present: Record<string, boolean>): HTMLElement {
  const input = h("input", {
    type: field.secret ? "password" : "text",
    placeholder: present[field.key] ? "••••••••  (stored)" : field.placeholder,
    autocomplete: "off",
    spellcheck: "false",
    style: "flex:1 1 auto;min-width:0",
  }) as HTMLInputElement;
  const saveBtn = h("button", { text: "Zapisz" });
  const dotEl = statusDot(present[field.key] ?? false);
  saveBtn.addEventListener("click", async () => {
    const value = input.value.trim();
    try {
      await Bridge.secretSet(field.key, value);
      present[field.key] = value.length > 0;
      input.value = "";
      input.placeholder = value ? "••••••••  (stored)" : field.placeholder;
      dotEl.style.background = value ? "#22c55e" : "#f4505e";
    } catch {
      dotEl.style.background = "#f5a524";
    }
  });
  return h("div", { class: "row" },
    h("label", { style: "min-width:104px", text: field.label }),
    input, saveBtn, dotEl,
  );
}

function webSection(present: Record<string, boolean>): HTMLElement {
  const provider = h("select", {}) as HTMLSelectElement;
  for (const [id, label, note] of SEARCH_PROVIDERS) {
    provider.append(h("option", { value: id, text: `${label} — ${note}` }));
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
    h("h2", {}, h("span", { text: "Internet" })),
    h("div", { class: "row" },
      h("label", { text: "Szukanie w sieci" }),
      toggle(settings.webSearch, (v) => { settings.webSearch = v; void save(); }),
      h("span", { class: "hint", text: "świeże dane, linki, pogoda, ceny" }),
    ),
    h("div", { class: "row" },
      h("label", { text: "Wyszukiwarka" }),
      provider,
      h("span", { class: "hint", text: "klucze tylko dla Brave / Tavily — DuckDuckGo działa od razu" }),
    ),
    rows,
  );
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
    fields: [{ key: "stripe-api-key", label: "Klucz tajny", placeholder: "sk_live_…", secret: true }] },
  { id: "integration_github", name: "GitHub", color: "#F4505E",
    fields: [{ key: "github-token", label: "Token", placeholder: "ghp_…", secret: true }] },
  { id: "integration_vercel", name: "Vercel", color: "#7C5CFF",
    fields: [{ key: "vercel-token", label: "Token", placeholder: "…", secret: true }] },
  { id: "integration_n8n", name: "n8n", color: "#F29B38",
    fields: [
      { key: "n8n-url", label: "URL instancji", placeholder: "https://n8n.example.com", secret: false },
      { key: "n8n-api-key", label: "Klucz API", placeholder: "…", secret: true },
    ] },
  { id: "integration_resend", name: "Resend", color: "#22C55E",
    fields: [{ key: "resend-api-key", label: "Klucz API", placeholder: "re_…", secret: true }] },
  { id: "integration_notion", name: "Notion", color: "#8C8C8C",
    fields: [{ key: "notion-api-key", label: "Token integracji", placeholder: "ntn_…", secret: true }] },
  { id: "integration_calcom", name: "Cal.com", color: "#C9956A",
    fields: [{ key: "calcom-api-key", label: "Klucz API", placeholder: "cal_…", secret: true }] },
];

const MAX_ACTIVE = 4;

function integrationsSection(present: Record<string, boolean>): HTMLElement {
  const note = h("div", { class: "hint" });
  const list = h("div", { style: "display:flex;flex-direction:column;gap:14px" });

  function updateNote() {
    const used = settings.activeIntegrations.length;
    note.textContent = `Wybierz maksymalnie ${MAX_ACTIVE} pigułki obok Iskra — użyto ${used}/${MAX_ACTIVE}. Klucze są przechowywane w menedżerze poświadczeń Windows, nigdy na dysku.`;
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
  return h("section", {}, h("h2", {}, h("span", { text: "Integracje" })), note, list);
}

// ── General section ───────────────────────────────────────────────────────────

/** The chords the island can be summoned with. Polish layout is the constraint:
 * Ctrl+Alt is AltGr, so only keys with no AltGr diacritic are offered. */
const HOTKEY_OPTIONS = ["Ctrl+Alt+M", "Ctrl+Alt+Shift+M", "Ctrl+Shift+M", "Ctrl+Alt+G", "Ctrl+Alt+Space"];

function generalSection(): HTMLElement {
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

  const screen = h("select", {}) as HTMLSelectElement;
  screen.append(
    h("option", { value: "primary", text: "Monitor główny" }),
    h("option", { value: "cursor", text: "Monitor pod kursorem" }),
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
    h("h2", {}, h("span", { text: "Ogólne" })),
    h("div", { class: "row" },
      h("label", { text: "Auto-zamykanie" }),
      autoClose,
      h("span", { class: "hint", text: "sekund po opuszczeniu wyspy" }),
    ),
    h("div", { class: "row" },
      h("label", { text: "Wyspa mieszka na" }),
      screen,
    ),
    h("div", { class: "row" },
      h("label", { text: "Skrót otwierający" }),
      hotkey,
    ),
    h("div", { class: "row" },
      h("label", { text: "Uruchamiaj przy starcie" }),
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
    integrationsSection(present),
    generalSection(),
    h("div", {
      class: "hint",
      text: "Bez telemetrii. Zapytania sieciowe trafiają wyłącznie do usług, które sam konfigurujesz.",
    }),
  );

  void onEvent<Settings>("settings-changed", (s) => {
    settings = { ...settings, ...s };
  });
}

void main();

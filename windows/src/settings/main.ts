// Settings window, the place where anything that writes to disk is confirmed.
// Covers the general preferences, the DeepSeek chat key and the integrations.

import "./settings.css";
import { Bridge, onEvent } from "../core/bridge";
import { DEFAULT_SETTINGS, type Settings } from "../core/state";
import { CHAT_PROVIDERS, chatProvider, chatKeyFor } from "../core/providers";
import { LANGUAGES, isLang, setLang, t, type TextKey } from "../core/i18n";
import { h, clear } from "../views/dom";

let settings: Settings = { ...DEFAULT_SETTINGS };
let version = "";
/** The monitor picker, kept so it can be re-read when the window is reopened. */
let screenSelect: HTMLSelectElement | null = null;

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

// ── Chat API section ──────────────────────────────────────────────────────────
// Two providers behind one switch. Which one is on decides the credentials entry, the
// model list and the reasoning field, so everything provider-shaped is redrawn when the
// switch moves — and the model follows the provider, because a DeepSeek id under Gemini
// is a 400 on every chat with nothing on screen to explain it.

function apiSection(present: Record<string, boolean>): HTMLElement {
  const providerSelect = h("select", {}) as HTMLSelectElement;
  for (const provider of CHAT_PROVIDERS) {
    // "(free)" belongs in the picker alone: the key badge and the ask button name the provider,
    // and a price note there would be noise.
    const text = provider.note ? `${provider.label} (${t(provider.note)})` : provider.label;
    providerSelect.append(h("option", { value: provider.id, text }));
  }
  providerSelect.value = chatProvider(settings.provider).id;

  const dot = statusDot(present[chatKeyFor(settings.provider)] ?? false);
  const heading = h("span", { text: chatProvider(settings.provider).label });
  const detail = h("div", { style: "display:flex;flex-direction:column;gap:6px" });
  const feedback = h("div", {});

  function draw() {
    const provider = chatProvider(settings.provider);
    const hasKey = present[provider.key] ?? false;
    heading.textContent = provider.label;
    dot.style.background = hasKey ? "#22c55e" : "#f4505e";
    clear(detail);
    clear(feedback);

    const state = h("span", {
      class: "hint",
      text: hasKey ? t("set.keyStored") : t("set.keyMissing"),
    });
    const field = h("input", {
      type: "password",
      placeholder: hasKey ? t("set.keyPlaceholderSaved") : provider.placeholder,
      style: "flex:1 1 auto;min-width:0",
      autocomplete: "off",
      spellcheck: "false",
    }) as HTMLInputElement;
    const saveBtn = h("button", { class: "primary", text: t("set.keySave") });
    const clearBtn = h("button", { class: "danger", text: t("set.keyRemove") });
    clearBtn.style.display = hasKey ? "" : "none";

    saveBtn.addEventListener("click", async () => {
      const value = field.value.trim();
      if (!value) return;
      try {
        await Bridge.secretSet(provider.key, value);
        present[provider.key] = true;
        draw();
        feedback.append(h("div", { class: "notice ok", text: t("set.keySaved") }));
      } catch (err) {
        feedback.append(h("div", { class: "notice err", text: t("set.keySaveFailed", String(err)) }));
      }
    });

    clearBtn.addEventListener("click", async () => {
      try {
        await Bridge.secretClear(provider.key);
        present[provider.key] = false;
        draw();
        feedback.append(h("div", { class: "notice ok", text: t("set.keyRemoved") }));
      } catch (err) {
        feedback.append(h("div", { class: "notice err", text: t("set.keyRemoveFailed", String(err)) }));
      }
    });

    const model = h("select", {}) as HTMLSelectElement;
    for (const [id, label] of provider.models) model.append(h("option", { value: id, text: label }));
    // A stored model from the other provider is not offered here at all, so the field shows
    // what a chat would actually use rather than a value that cannot be sent.
    model.value = provider.models.some(([id]) => id === settings.model)
      ? settings.model
      : provider.models[0][0];
    model.addEventListener("change", () => {
      settings.model = model.value;
      void save();
    });

    detail.append(
      state,
      h("div", { class: "row" }, h("label", { text: t("set.apiKey") }), field, saveBtn, clearBtn),
      h("div", { class: "row" }, h("label", { text: t("set.model") }), model),
      h("div", { class: "row" },
        h("label", { text: t("set.thinking") }),
        toggle(settings.thinking, (v) => { settings.thinking = v; void save(); }),
        h("span", { class: "hint", text: t("set.thinkingHint") }),
      ),
      h("div", { class: "row" },
        h("label", { text: t("set.getKey") }),
        h("button", { text: provider.keyUrl, onclick: () => void Bridge.openUrl(provider.keyUrl) }),
      ),
      h("div", { class: "row" }, h("span", { class: "hint", text: t("set.providerHint") })),
    );
  }

  providerSelect.addEventListener("change", () => {
    const next = chatProvider(providerSelect.value);
    settings.provider = next.id;
    if (!next.models.some(([id]) => id === settings.model)) settings.model = next.models[0][0];
    void save();
    draw();
  });

  draw();

  return h(
    "section",
    {},
    h("h2", {}, dot, heading),
    h("div", { class: "row" }, h("label", { text: t("set.provider") }), providerSelect),
    detail,
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
 * integrations and the search backends, same rules for every secret. */
function secretRow(field: SecretField, present: Record<string, boolean>): HTMLElement {
  const stored = present[field.key] ?? false;
  const input = h("input", {
    type: field.secret ? "password" : "text",
    placeholder: field.secret && stored ? t("set.stored") : field.placeholder,
    autocomplete: "off",
    spellcheck: "false",
    style: "flex:1 1 auto;min-width:0",
  }) as HTMLInputElement;
  // A region code or an instance URL is a setting, so it is shown as it is.
  if (!field.secret && stored) {
    void Bridge.secretValue(field.key).then((value) => {
      if (value && !input.value) input.value = value;
    });
  }
  const saveBtn = h("button", { text: t("set.save") });
  const dotEl = statusDot(stored);
  saveBtn.addEventListener("click", async () => {
    const value = input.value.trim();
    try {
      await Bridge.secretSet(field.key, value);
      present[field.key] = value.length > 0;
      if (field.secret) {
        input.value = "";
        input.placeholder = value ? t("set.stored") : field.placeholder;
      }
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

// Two groups, in this order: the pills that need no key or account first, because those are the
// ones that work the moment Oczi is installed, then the keyed ones with Games at the top — the
// list is read top to bottom, so a user who has no key at all should not have to scroll past six
// services he cannot use yet to find the two that already work.
const INTEGRATIONS: IntegrationDef[] = [
  // No key, no account: this one reads the machine Oczi runs on.
  { id: "integration_pc", name: "PC", color: "#4CC2FF", fields: [] },
  // Nor this one: Windows itself reports what is playing, whatever the player.
  { id: "integration_music", name: "Music", color: "#F472B6", fields: [] },
  // Prices: gg.deals' own API with the key the user generates in their account (Settings → Integrations).
  // Steam does the title lookup, which is why no other field is needed.
  { id: "integration_prices", name: "Games", color: "#E4A11B",
    fields: [
      { key: "ggdeals-api-key", labelKey: "set.ggdealsKey", placeholder: "…", secret: true },
      // The region decides the currency: `pl` gives PLN, and leaving it empty follows the
      // interface language.
      { key: "ggdeals-region", labelKey: "set.ggdealsRegion", placeholder: "pl", secret: false },
    ] },
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
    // An integration with nothing to fill in says so rather than leaving a gap.
    if (def.fields.length === 0) rows.append(h("div", { class: "hint", text: t("set.noKeyNeeded") }));

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

/** The named keys Rust understands, `key_code` in island.rs is the other half. */
const NAMED_KEYS: Record<string, string> = {
  " ": "Space",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  ArrowUp: "Up",
  ArrowDown: "Down",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Insert: "Insert",
  Delete: "Delete",
  Backspace: "Backspace",
};

const MODIFIER_KEYS = ["Control", "Alt", "Shift", "Meta", "AltGraph"];

type ChordResult = { ok: true; chord: string } | { ok: false; why: "modifier" | "key" | "layout" };

/** What a keydown means as a chord: the modifiers first, then the key under them. */
function chordFor(e: KeyboardEvent): ChordResult {
  // A modifier on its own is half a chord, wait for the key it modifies.
  if (MODIFIER_KEYS.includes(e.key)) return { ok: false, why: "key" };
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Win");

  let key = NAMED_KEYS[e.key];
  if (!key && /^[a-z0-9]$/i.test(e.key)) key = e.key.toUpperCase();
  if (!key && /^F([1-9]|1\d|2[0-4])$/i.test(e.key)) key = e.key.toUpperCase();
  if (!key) return { ok: false, why: "key" };
  // A bare key would be swallowed everywhere on the desktop, so one modifier is
  // the least a chord may have.
  if (mods.length === 0) return { ok: false, why: "modifier" };
  // Alt+Shift on its own is the keyboard-layout switch: Windows registers such a chord
  // and then hands the presses to the layout switcher, so it would look set up and never
  // fire. Any third modifier takes it out of that case, so only the bare pair is refused.
  if (mods.length === 2 && mods.includes("Alt") && mods.includes("Shift")) {
    return { ok: false, why: "layout" };
  }
  return { ok: true, chord: [...mods, key].join("+") };
}

function hotkeyField(
  initial: string,
  onChord: (chord: string) => void,
  onProblem: (why: "modifier" | "key" | "layout") => void,
): HTMLButtonElement {
  const field = h("button", { class: "hotkey-field", text: initial }) as HTMLButtonElement;
  let chord = initial;
  let recording = false;

  const stop = () => {
    recording = false;
    field.classList.remove("recording");
    field.textContent = chord;
  };

  field.addEventListener("click", () => {
    if (recording) return stop();
    recording = true;
    field.classList.add("recording");
    field.textContent = t("set.hotkeyRecording");
  });

  // A click anywhere else is "never mind", not a chord.
  document.addEventListener("pointerdown", (e) => {
    if (recording && !field.contains(e.target as Node)) stop();
  });

  document.addEventListener(
    "keydown",
    (e) => {
      if (!recording) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") return stop();
      const result = chordFor(e);
      if (!result.ok) return onProblem(result.why);
      chord = result.chord;
      stop();
      onChord(chord);
    },
    true,
  );

  return field;
}

async function listMonitors(select: HTMLSelectElement) {
  const monitors = (await Bridge.listMonitors()) ?? [];
  // Read after the await: a pick made while the list was on its way wins.
  const wanted = settings.screen;
  // Drop what a previous pass added, every monitor entry, plus the placeholder.
  for (const opt of Array.from(select.options)) {
    if (opt.value.startsWith("monitor:")) opt.remove();
  }
  monitors.forEach((m, i) => {
    const parts = [t("set.screenMonitor", m.number ?? i + 1), `${m.width}×${m.height}`];
    if (m.primary) parts.push(t("set.screenMain"));
    const option = h("option", { value: `monitor:${m.key}`, text: parts.join(" · ") });
    if (m.name) option.title = m.name;
    select.append(option);
  });
  if (wanted.startsWith("monitor:") && !monitors.some((m) => `monitor:${m.key}` === wanted)) {
    select.append(h("option", { value: wanted, text: t("set.screenMissing") }));
  }
  select.value = wanted;
}

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
  // Set what we can before the list arrives; `listMonitors` settles a pinned
  // display once it knows the monitors.
  screen.value = settings.screen;
  screen.addEventListener("change", () => {
    if (!screen.value) return;
    settings.screen = screen.value;
    void save();
  });
  screenSelect = screen;
  void listMonitors(screen);

  // Both chords are recorded the same way, and the note under them says what the
  // pair costs: the same chord twice, or Ctrl+Alt on a Polish layout.
  const hotkeyNote = h("div", { class: "hint" });
  const message = h("div", { class: "hint strongly" });
  let messageTimer = 0;

  function refreshNote() {
    const chords = [settings.hotkey, settings.snipHotkey, settings.pinHotkey];
    const same = new Set(chords).size !== chords.length;
    // Ctrl+Alt is AltGr, so a chord built on it takes a diacritic away from
    // whichever app has the focus, worth saying out loud, not worth a warning.
    const altGr = chords.some((c) => c.includes("Ctrl+Alt"));
    hotkeyNote.textContent = same
      ? t("set.hotkeyConflict")
      : altGr
        ? t("set.hotkeyAltGr")
        : t("set.hotkeyHint");
    hotkeyNote.className = same ? "hint strongly" : "hint";
  }

  /** The recorder's own refusal, shown where the chord is and taken back again. */
  function showProblem(why: "modifier" | "key" | "layout") {
      const text =
        why === "modifier"
          ? "set.hotkeyNeedsMod"
          : why === "layout"
            ? "set.hotkeyLayout"
            : "set.hotkeyUnknownKey";
      message.textContent = t(text);
    window.clearTimeout(messageTimer);
    messageTimer = window.setTimeout(() => {
      message.textContent = "";
    }, 4000);
  }

  const hotkey = hotkeyField(
    settings.hotkey,
    (chord) => {
      settings.hotkey = chord;
      void save();
      refreshNote();
    },
    showProblem,
  );
  const snipHotkey = hotkeyField(
    settings.snipHotkey,
    (chord) => {
      settings.snipHotkey = chord;
      void save();
      refreshNote();
    },
    showProblem,
  );
  // The dictation chord is only useful in the build that has a speech engine; the
  // ordinary one stores it and registers nothing, which the note under the field says.
  const voiceHotkey = hotkeyField(
    settings.voiceHotkey,
    (chord) => {
      settings.voiceHotkey = chord;
      void save();
      refreshNote();
    },
    showProblem,
  );
  // The pin chord: the same recorder, its own field, so it is his to move like the rest.
  const pinHotkey = hotkeyField(
    settings.pinHotkey,
    (chord) => {
      settings.pinHotkey = chord;
      void save();
      refreshNote();
    },
    showProblem,
  );
  // What the engine listens for. Auto is the default and costs a second pass over the
  // audio; pinning the language is faster and better on names, and wrong if he switches
  // language mid-sentence — so it is his choice, with the trade in the hint.
  const voiceLanguage = h("select", {}) as HTMLSelectElement;
  for (const value of ["auto", "pl", "en"] as const) {
    voiceLanguage.append(
      h("option", { value, text: t(("set.voiceLang." + value) as TextKey) }),
    );
  }
  voiceLanguage.value = settings.voiceLanguage;
  voiceLanguage.addEventListener("change", () => {
    const value = voiceLanguage.value as "auto" | "pl" | "en";
    if (!["auto", "pl", "en"].includes(value)) return;
    settings.voiceLanguage = value;
    void save();
  });
  refreshNote();

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
    h("div", { class: "row" }, h("label", { text: t("set.hotkeySnip") }), snipHotkey),
    h("div", { class: "row" }, h("label", { text: t("set.hotkeyVoice") }), voiceHotkey),
    h("div", { class: "row" }, h("label", { text: t("set.hotkeyPin") }), pinHotkey),
    h("div", { class: "row" },
      h("label", { text: t("set.voiceLanguage") }),
      voiceLanguage,
      h("span", { class: "hint", text: t("set.voiceLanguageHint") }),
    ),
    hotkeyNote,
    message,
    h("div", { class: "row" },
      h("label", { text: t("set.autostart") }),
      toggle(settings.autostart, (v) => { settings.autostart = v; void save(); }),
    ),
    h("div", { class: "row" },
      h("label", { text: t("set.gameMode") }),
      toggle(settings.gameMode, (v) => { settings.gameMode = v; void save(); }),
      h("span", { class: "hint", text: t("set.gameModeHint") }),
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

  // Read from the tables that build the rows: the hand-kept copy here missed the Games keys.
  const keys = [
    ...CHAT_PROVIDERS.map((provider) => provider.key),
    ...SEARCH_KEYS.map((f) => f.key),
    ...INTEGRATIONS.flatMap((def) => def.fields.map((f) => f.key)),
  ];
  const present: Record<string, boolean> = {};
  for (const k of keys) present[k] = (await Bridge.secretPresent(k)) ?? false;

  clear(root);
  root.append(
    h("h1", {}, h("span", { text: "Oczi" }), h("span", { class: "version", text: version })),
    apiSection(present),
    webSection(present),
    terminalSection(),
    integrationsSection(present),
    generalSection(),
    h("div", { class: "hint", text: t("set.privacy") }),
  );

  void onEvent<Settings>("settings-changed", (s) => {
    settings = { ...settings, ...s };
  });

  // The window is built once at launch and only shown/hidden afterwards, so the
  // monitor list is re-read each time it comes back to the screen.
  void onEvent<null>("settings-shown", () => {
    if (screenSelect) void listMonitors(screenSelect);
  });
}

void main();

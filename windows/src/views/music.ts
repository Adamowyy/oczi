// The now-playing card, one pill for whatever Windows says is playing.

import { h, svg, dot } from "./dom";
import { ICONS } from "./icons";
import { Bridge } from "../core/bridge";
import { State } from "../core/state";
import { t } from "../core/i18n";

const ID = "integration_music";

function data(): Record<string, unknown> {
  return (State.integrations[ID]?.data ?? {}) as Record<string, unknown>;
}

/** The pill is switched on in settings, nothing polls this otherwise. */
export function musicEnabled(): boolean {
  return State.settings.activeIntegrations.includes(ID);
}

/** Something is playing right now. */
export function musicPlaying(): boolean {
  return musicEnabled() && data().playing === true;
}

/** Everything the card draws except the position: this is what decides when the
 *  card is rebuilt, so a moving position never rebuilds it. */
export function musicKey(): string {
  const d = data();
  return [d.playing, d.status, d.title, d.artist, d.app, d.durationSecs, d.canPrev, d.canNext, d.canToggle].join("|");
}

function fmt(secs: number): string {
  if (!Number.isFinite(secs) || secs <= 0) return "0:00";
  const total = Math.floor(secs);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

// ── The running clock ─────────────────────────────────────────────────────────

interface Live {
  el: HTMLElement;
  /** `performance.now()` when the sample behind this card arrived. */
  at: number;
  position: number;
  duration: number;
  playing: boolean;
}

const live = new Set<Live>();
let timer: number | null = null;

function paint(entry: Live, position: number) {
  const fill = entry.el.querySelector<HTMLElement>(".music-fill");
  const time = entry.el.querySelector<HTMLElement>(".music-time");
  if (fill) {
    const ratio = entry.duration > 0 ? Math.min(1, position / entry.duration) : 0;
    fill.style.width = `${(ratio * 100).toFixed(1)}%`;
  }
  if (time) {
    time.textContent =
      entry.duration > 0 ? `${fmt(position)} / ${fmt(entry.duration)}` : fmt(position);
  }
}

/** Advances every card on screen half a second at a time, and stops itself when
 *  the last one goes away, the overlay is hidden most of the day. */
function pump() {
  const now = performance.now();
  for (const entry of [...live]) {
    if (!entry.el.isConnected) {
      live.delete(entry);
      continue;
    }
    const elapsed = entry.playing ? (now - entry.at) / 1000 : 0;
    paint(entry, entry.position + elapsed);
  }
  if (live.size === 0 && timer != null) {
    window.clearInterval(timer);
    timer = null;
  }
}

// ── The card ──────────────────────────────────────────────────────────────────

export function buildMusicCard(): HTMLElement {
  const d = data();
  const playing = d.playing === true;
  const title = String(d.title ?? "") || t("int.nothingPlaying");
  const artist = String(d.artist ?? "");
  const app = String(d.app ?? "");
  const position = Number(d.positionSecs ?? 0);
  const duration = Number(d.durationSecs ?? 0);

  const button = (
    icon: string,
    action: "prev" | "toggle" | "next",
    label: string,
    enabled: boolean,
  ) => {
    const el = h(
      "button",
      { class: "music-btn", title: label, onclick: () => void Bridge.mediaControl(action) },
      svg(icon, 11),
    ) as HTMLButtonElement;
    // Disabled rather than hidden: the row keeps its shape, and a player that
    // cannot skip says so instead of pretending the button is not there.
    el.disabled = !enabled;
    return el;
  };

  const time = h("span", { class: "music-time" });
  const fill = h("i", { class: "music-fill" });
  const toggle = h(
    "button",
    {
      class: "music-btn primary",
      title: playing ? t("int.pause") : t("int.play"),
      onclick: () => void Bridge.mediaControl("toggle"),
    },
    svg(playing ? ICONS.pause : ICONS.play, 11),
  ) as HTMLButtonElement;
  toggle.disabled = d.canToggle !== true;

  // One fact per line: the track on its own, then where it comes from and who it
  // is by. Sharing a line is what cut both of them off.
  const meta = h("div", { class: "music-meta" });
  if (app) meta.append(h("span", { class: "music-app", text: app }));
  if (app && artist) meta.append(h("span", { class: "music-sep", text: "·" }));
  if (artist) meta.append(h("span", { class: "music-artist", text: artist }));

  const card = h(
    "div",
    { class: playing ? "music-card playing" : "music-card" },
    h("div", { class: "music-head" }, dot("#F472B6", 7), h("b", { text: "Music" }), time),
    h("div", { class: "music-title", text: title }),
    app || artist ? meta : null,
    h("div", { class: "music-track" }, fill),
    h("div", { class: "music-controls" }, button(ICONS.prev, "prev", t("int.prev"), d.canPrev === true), toggle, button(ICONS.next, "next", t("int.next"), d.canNext === true)),
  );

  const entry: Live = { el: card, at: performance.now(), position, duration, playing };
  live.add(entry);
  paint(entry, position);
  if (timer == null) timer = window.setInterval(pump, 500);
  return card;
}

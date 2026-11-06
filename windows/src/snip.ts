// The screen-selection overlay — the "eye". All pixels live in Rust (`snip.rs`), which
// froze the desktop and does the crop; this file only turns a drag into a window rect.

import { invoke } from "@tauri-apps/api/core";
import { setLang, storedLang, t } from "./core/i18n";

const band = document.getElementById("snip-band") as HTMLDivElement;
const body = document.body;
const hint = document.getElementById("snip-hint") as HTMLDivElement;

setLang(storedLang());
hint.textContent = t("snip.hint");

let origin: { x: number; y: number } | null = null;
/** Set when the crop was refused: the overlay stops taking selections and says why. */
let refusing = false;

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

function rectFrom(event: MouseEvent): Rect | null {
  if (!origin) return null;
  return {
    x: Math.min(origin.x, event.clientX),
    y: Math.min(origin.y, event.clientY),
    width: Math.abs(event.clientX - origin.x),
    height: Math.abs(event.clientY - origin.y),
  };
}

function paint(rect: Rect | null) {
  if (!rect || rect.width < 1 || rect.height < 1) {
    band.hidden = true;
    body.classList.remove("selecting");
    return;
  }
  body.classList.add("selecting");
  band.hidden = false;
  band.style.left = `${rect.x}px`;
  band.style.top = `${rect.y}px`;
  band.style.width = `${rect.width}px`;
  band.style.height = `${rect.height}px`;
}

function cancel() {
  origin = null;
  paint(null);
  void invoke("cancel_snip");
}

window.addEventListener("mousedown", (event) => {
  if (event.button !== 0 || refusing) return;
  origin = { x: event.clientX, y: event.clientY };
  paint({ x: origin.x, y: origin.y, width: 1, height: 1 });
});

window.addEventListener("mousemove", (event) => {
  if (origin) paint(rectFrom(event));
});

window.addEventListener("mouseup", (event) => {
  if (!origin) return;
  const rect = rectFrom(event);
  origin = null;
  // A click is not a selection: it means "never mind".
  if (!rect || rect.width < 6 || rect.height < 6) {
    cancel();
    return;
  }
  paint(rect);
  void invoke("finish_snip", {
    x: rect.x,
    y: rect.y,
    width: rect.width,
    height: rect.height,
  }).catch((err) => refuse(String(err)));
});

/** The crop did not happen. The overlay stays up for a moment with the reason on it — a person who
 *  just dragged a region has to learn why nothing appeared, and the pins that are already on the
 *  desk are the reason. */
function refuse(error: string) {
  refusing = true;
  paint(null);
  body.classList.add("refusing");
  hint.textContent = error.includes("pins-full") ? t("snip.pinsFull") : error.replace(/^Error:\s*/, "");
  window.setTimeout(() => {
    body.classList.remove("refusing");
    void invoke("cancel_snip");
  }, 2600);
}

window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") cancel();
});

// Right-click also backs out, like every other snipping tool.
window.addEventListener("contextmenu", (event) => {
  event.preventDefault();
  cancel();
});

// A new capture must always start from a clean slate.
paint(null);

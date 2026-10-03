// The screen-selection overlay, the "eye".

import { invoke } from "@tauri-apps/api/core";

const band = document.getElementById("snip-band") as HTMLDivElement;
const body = document.body;

let origin: { x: number; y: number } | null = null;

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
  if (event.button !== 0) return;
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
  });
});

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

// One pinned crop: draws the picture Rust hands over and asks Rust for the moves.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { setLang, storedLang, t } from "./core/i18n";

const shot = document.getElementById("pin-shot") as HTMLImageElement;
const close = document.getElementById("pin-close") as HTMLButtonElement;
const keep = document.getElementById("pin-keep") as HTMLButtonElement;
const menu = document.getElementById("pin-menu") as HTMLDivElement;
const toast = document.getElementById("pin-toast") as HTMLDivElement;
const opacityRow = document.getElementById("pin-opacity") as HTMLDivElement;

/** Which window this is: every command is about one pin. */
const windowLabel = getCurrentWindow().label;

setLang(storedLang());

/** One line into the app log, so a window that shows nothing says where it stopped. */
function note(message: string) {
  void invoke("log_line", { message: `pin  ${message}` }).catch(() => {});
}

/** Feedback inside the pin: the text goes to a clipboard nobody is watching. */
let toastTimer: number | null = null;
function flash(message: string) {
  toast.textContent = message;
  toast.classList.add("on");
  if (toastTimer !== null) window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => toast.classList.remove("on"), 2200);
}

let onTop = true;

/** The pin toggle: above everything, or an ordinary window. */
function showKeep() {
  keep.classList.toggle("off", !onTop);
  keep.title = t(onTop ? "pin.keepOn" : "pin.keepOff");
  keep.setAttribute("aria-label", keep.title);
}

function label(): void {
  (menu.querySelector('[data-action="ask"]') as HTMLElement).textContent = t("pin.ask");
  (menu.querySelector('[data-action="copy-text"]') as HTMLElement).textContent = t("pin.copyText");
  (menu.querySelector('[data-action="copy-image"]') as HTMLElement).textContent = t("pin.copyImage");
  (menu.querySelector('[data-action="save"]') as HTMLElement).textContent = t("pin.save");
  (menu.querySelector('[data-action="close"]') as HTMLElement).textContent = t("pin.close");
  (menu.querySelector('[data-action="close-all"]') as HTMLElement).textContent = t("pin.closeAll");
  (opacityRow.querySelector("span") as HTMLElement).textContent = t("pin.opacity");
  close.title = t("pin.close");
}

function setOpacity(percent: number) {
  const value = Math.min(100, Math.max(15, Math.round(percent)));
  shot.style.opacity = String(value / 100);
  for (const chip of opacityRow.querySelectorAll("button")) {
    chip.classList.toggle("on", Number(chip.dataset.opacity) === value);
  }
  note(`opacity ${value}%`);
}

/** Puts the picture back to filling its own window. */
function releaseRoom() {
  shot.style.position = "";
  shot.style.left = "";
  shot.style.top = "";
  shot.style.width = "";
  shot.style.height = "";
}

async function closeMenu() {
  if (menu.hidden) return;
  menu.hidden = true;
  document.body.classList.remove("menu");
  releaseRoom();
  try {
    await invoke("pin_menu", { label: windowLabel, open: false, needW: 0, needH: 0 });
  } catch (err) {
    note(`could not give the room back: ${err}`);
  }
}

/** The window is made bigger for as long as the menu is open; the picture keeps its place. */
async function openMenu(x: number, y: number) {
  if (!menu.hidden) return;
  note("menu open");
  document.body.classList.add("menu");
  menu.hidden = false;
  menu.style.visibility = "hidden";
  const box = menu.getBoundingClientRect();
  // The window is the picture until the menu asks for room.
  const picW = window.innerWidth;
  const picH = window.innerHeight;

  let offsetX = 0;
  let offsetY = 0;
  try {
    const room = await invoke<[number, number]>("pin_menu", {
      label: windowLabel,
      open: true,
      needW: box.width + 8,
      needH: box.height + 8,
    });
    [offsetX, offsetY] = room;
  } catch (err) {
    note(`could not make room for the menu: ${err}`);
  }

  shot.style.position = "absolute";
  shot.style.left = `${offsetX}px`;
  shot.style.top = `${offsetY}px`;
  shot.style.width = `${picW}px`;
  shot.style.height = `${picH}px`;

  const left = offsetX > 0 ? offsetX - box.width - 6 : picW + 6;
  const top = offsetY > 0 ? offsetY - box.height - 6 : picH + 6;
  menu.style.left = `${Math.max(2, Math.min(left, window.innerWidth - box.width - 2))}px`;
  menu.style.top = `${Math.max(2, Math.min(top, window.innerHeight - box.height - 2))}px`;
  menu.style.visibility = "";
  if (x < 0 || y < 0) note("menu placed");
}

note("page ready");
label();
showKeep();

// A new crop for this window: the picture replaces the last one in place.
void listen<string>(`pin-image-${windowLabel}`, (event) => {
  note(`image ${event.payload.length} chars`);
  void closeMenu();
  releaseRoom();
  shot.src = event.payload;
  shot.hidden = false;
  // A new crop is always a pin proper, whatever the last one was set to.
  onTop = true;
  showKeep();
  setOpacity(100);
}).catch((err) => note(`could not listen: ${err}`));

// The picture's own verdict, so "drawn" is a fact in the log rather than an assumption.
shot.addEventListener("load", () => note(`drawn ${shot.naturalWidth}x${shot.naturalHeight}`));
shot.addEventListener("error", () => note("the picture would not load"));

// ── Actions ──────────────────────────────────────────────────────────────────

async function run(action: string) {
  void closeMenu();
  note(`action ${action}`);
  try {
    switch (action) {
      case "ask":
        // The pin stays: asking about a reference is not a reason to lose the reference.
        await invoke("pin_ask", { label: windowLabel });
        break;
      case "copy-text": {
        const text = await invoke<string>("pin_copy_text", { label: windowLabel });
        flash(text.trim().length > 0 ? t("pin.textCopied") : t("pin.textNone"));
        break;
      }
      case "copy-image":
        await invoke("pin_copy_image", { label: windowLabel });
        flash(t("pin.imageCopied"));
        break;
      case "save": {
        const saved = await invoke<string | null>("pin_save_as", { label: windowLabel });
        if (saved) flash(t("pin.saved", saved.split(/[\\/]/).pop() ?? saved));
        break;
      }
      case "close":
        await invoke("pin_close", { label: windowLabel });
        break;
      case "close-all":
        await invoke("pin_close_all");
        break;
    }
  } catch (err) {
    flash(String(err).replace(/^Error:\s*/, ""));
  }
}

menu.addEventListener("click", (event) => {
  const target = event.target as HTMLElement;
  const opacity = target.closest("[data-opacity]") as HTMLElement | null;
  if (opacity) {
    setOpacity(Number(opacity.dataset.opacity));
    return;
  }
  const action = target.closest("[data-action]") as HTMLElement | null;
  if (action?.dataset.action) void run(action.dataset.action);
});

// ── Mouse ────────────────────────────────────────────────────────────────────

document.addEventListener("mousedown", (event) => {
  if (event.button !== 0) return;
  if ((event.target as HTMLElement).closest("#pin-menu, #pin-close")) return;
  // A press on the picture puts the menu away first, since a press usually means a drag.
  if (!menu.hidden) {
    void closeMenu();
    return;
  }
  void invoke("pin_drag_start", { label: windowLabel });
});

document.addEventListener("contextmenu", (event) => {
  event.preventDefault();
  if (menu.hidden) void openMenu(event.clientX, event.clientY);
  else void closeMenu();
});

close.addEventListener("click", () => void run("close"));

keep.addEventListener("click", async () => {
  const wanted = !onTop;
  try {
    await invoke("pin_topmost", { label: windowLabel, on: wanted });
    onTop = wanted;
    showKeep();
    flash(t(onTop ? "pin.keepOn" : "pin.keepOff"));
  } catch (err) {
    flash(String(err).replace(/^Error:\s*/, ""));
  }
});

// The wheel zooms, Shift fades. Neither applies while the menu has the window bigger.
document.addEventListener(
  "wheel",
  (event) => {
    event.preventDefault();
    if (!menu.hidden) {
      void closeMenu();
      return;
    }
    if (event.shiftKey) {
      const current = Math.round(Number(shot.style.opacity || "1") * 100);
      setOpacity(current + (event.deltaY < 0 ? 10 : -10));
      return;
    }
    void invoke("pin_scale", { label: windowLabel, step: event.deltaY < 0 ? 1 : -1 });
  },
  { passive: false },
);

document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  if (!menu.hidden) void closeMenu();
  else void run("close");
});

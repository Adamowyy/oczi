// One pinned crop: draws the picture Rust hands over, lets it be marked up, and asks Rust for the
// moves.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { setLang, storedLang, t } from "./core/i18n";

const shot = document.getElementById("pin-shot") as HTMLImageElement;
const frame = document.getElementById("pin-frame") as HTMLDivElement;
const close = document.getElementById("pin-close") as HTMLButtonElement;
const keep = document.getElementById("pin-keep") as HTMLButtonElement;
const menu = document.getElementById("pin-menu") as HTMLDivElement;
const toast = document.getElementById("pin-toast") as HTMLDivElement;
const opacityRow = document.getElementById("pin-opacity") as HTMLDivElement;
const tools = document.getElementById("pin-tools") as HTMLDivElement;
const toolsToggle = document.getElementById("pin-tools-toggle") as HTMLButtonElement;
const ink = document.getElementById("pin-ink") as HTMLCanvasElement;
const textInput = document.getElementById("pin-text-input") as HTMLInputElement;

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

/** Above everything, or an ordinary window. */
function showKeep() {
  keep.classList.toggle("off", !onTop);
  keep.title = t(onTop ? "pin.keepOn" : "pin.keepOff");
  keep.setAttribute("aria-label", keep.title);
}

function showTools(open: boolean) {
  tools.hidden = !open;
  toolsToggle.classList.toggle("on", open);
  note(`tools ${open}`);
  // No palette, no tool: a pin with a hidden panel that still draws over the picture is a pin that
  // cannot be dragged.
  if (!open && tool !== "move") setTool("move");
}

function label(): void {
  toolsToggle.title = t("pin.tools");
  (menu.querySelector('[data-action="ask"]') as HTMLElement).textContent = t("pin.ask");
  (menu.querySelector('[data-action="copy-text"]') as HTMLElement).textContent = t("pin.copyText");
  (menu.querySelector('[data-action="copy-image"]') as HTMLElement).textContent = t("pin.copyImage");
  (menu.querySelector('[data-action="save"]') as HTMLElement).textContent = t("pin.save");
  (menu.querySelector('[data-action="close"]') as HTMLElement).textContent = t("pin.close");
  (menu.querySelector('[data-action="close-all"]') as HTMLElement).textContent = t("pin.closeAll");
  (opacityRow.querySelector("span") as HTMLElement).textContent = t("pin.opacity");
  close.title = t("pin.close");

  (tools.querySelector('[data-tool="move"]') as HTMLElement).title = t("pin.toolMove");
  (tools.querySelector('[data-tool="pen"]') as HTMLElement).title = t("pin.toolPen");
  (tools.querySelector('[data-tool="text"]') as HTMLElement).title = t("pin.toolText");
  (tools.querySelector('[data-action="undo"]') as HTMLElement).title = t("pin.undo");
  (tools.querySelector('[data-action="clear"]') as HTMLElement).title = t("pin.clearDrawing");
}

// ── Marking the picture up ───────────────────────────────────────────────────

type Stroke = {
  kind: "stroke";
  color: string;
  width: number;
  points: [number, number][];
};
type Caption = {
  kind: "text";
  color: string;
  size: number;
  x: number;
  y: number;
  text: string;
};
type Mark = Stroke | Caption;

let marks: Mark[] = [];
let penning: Stroke | null = null;
let tool: "move" | "pen" | "text" = "move";
let inkColor = "#f4505e";

/** Pen and letter sizes follow the picture, so the same gesture looks the same on a big crop and a
 *  thumbnail. */
function penWidth(): number {
  return Math.max(2, Math.round(ink.width / 240));
}
function textSize(): number {
  return Math.max(13, Math.round(ink.width / 38));
}

/** A pointer in window coordinates as a point on the picture. */
function toPicture(event: PointerEvent): [number, number] {
  const box = ink.getBoundingClientRect();
  const x = ((event.clientX - box.left) / Math.max(1, box.width)) * ink.width;
  const y = ((event.clientY - box.top) / Math.max(1, box.height)) * ink.height;
  return [x, y];
}

function captionFont(size: number): string {
  return `600 ${size}px "Segoe UI", system-ui, sans-serif`;
}

/** A caption is broken at word boundaries so it never runs off the picture: the note is pinned to a
 *  region, and words outside it are lost when the pin is zoomed or copied. */
function wrapCaption(text: string, maxWidth: number, size: number): string[] {
  const measure = document.createElement("canvas").getContext("2d");
  if (measure) measure.font = captionFont(size);
  const widthOf = (line: string) => (measure ? measure.measureText(line).width : line.length * size * 0.55);

  const words = text.split(/\s+/).filter((word) => word.length > 0);
  const lines: string[] = [];
  let line = "";
  for (const word of words) {
    const candidate = line ? `${line} ${word}` : word;
    if (!line || widthOf(candidate) <= maxWidth) line = candidate;
    else {
      lines.push(line);
      line = word;
    }
  }
  if (line) lines.push(line);
  return lines;
}

function captionLayout(mark: Caption) {
  const maxWidth = Math.max(40, ink.width - mark.x - 8);
  const lines = wrapCaption(mark.text, maxWidth, mark.size);
  const lineHeight = mark.size * 1.25;
  return { lines, lineHeight, height: lineHeight * lines.length };
}

/** Where a caption's letters sit, for hit testing and for keeping it inside the picture. */
function captionBox(mark: Caption) {
  const { lines, lineHeight, height } = captionLayout(mark);
  const measure = document.createElement("canvas").getContext("2d");
  if (measure) measure.font = captionFont(mark.size);
  const width = Math.max(
    ...lines.map((line) => (measure ? measure.measureText(line).width : line.length * mark.size * 0.55)),
  );
  return { x: mark.x, y: mark.y, width, height, lineHeight, lines };
}

function paint(context: CanvasRenderingContext2D, mark: Mark) {
  context.lineJoin = "round";
  context.lineCap = "round";
  if (mark.kind === "stroke") {
    context.strokeStyle = mark.color;
    context.lineWidth = mark.width;
    context.beginPath();
    mark.points.forEach(([x, y], index) => {
      if (index === 0) context.moveTo(x, y);
      else context.lineTo(x, y);
    });
    // A tap with the pen still leaves a dot.
    if (mark.points.length === 1) context.lineTo(mark.points[0][0] + 0.1, mark.points[0][1]);
    context.stroke();
    return;
  }
  context.font = captionFont(mark.size);
  context.textBaseline = "top";
  const { lines, lineHeight, height } = captionLayout(mark);
  const x = Math.max(2, Math.min(mark.x, ink.width - 4));
  // A caption that would run past the bottom edge climbs instead of being cut off.
  const top = Math.max(2, Math.min(mark.y, ink.height - height - 2));
  context.fillStyle = mark.color;
  lines.forEach((line, index) => context.fillText(line, x, top + index * lineHeight));
}

function redraw() {
  const context = ink.getContext("2d");
  if (!context) return;
  context.clearRect(0, 0, ink.width, ink.height);
  for (const mark of marks) paint(context, mark);
  if (penning) paint(context, penning);
}

/** The picture plus the marks, as one PNG. Everything that leaves the pin takes this, so what is
 *  copied, saved or asked about is what is on screen. */
function composite(): string {
  const canvas = document.createElement("canvas");
  canvas.width = ink.width;
  canvas.height = ink.height;
  const context = canvas.getContext("2d");
  if (!context) return "";
  if (shot.naturalWidth > 0) context.drawImage(shot, 0, 0, canvas.width, canvas.height);
  context.drawImage(ink, 0, 0);
  return canvas.toDataURL("image/png");
}

async function pushMarks() {
  try {
    await invoke("pin_mark", { label: windowLabel, data: marks.length > 0 ? composite() : "" });
  } catch (err) {
    note(`could not store the drawing: ${err}`);
  }
}

function setTool(next: "move" | "pen" | "text") {
  tool = next;
  note(`tool ${next}`);
  if (next === "move" && !textInput.hidden) commitText();
  for (const button of tools.querySelectorAll("[data-tool]")) {
    button.classList.toggle("on", (button as HTMLElement).dataset.tool === next);
  }
  // The canvas always shows the marks; only the pointer decides whether a press draws or moves.
  ink.style.pointerEvents = next === "move" ? "none" : "auto";
  ink.style.cursor = next === "text" ? "text" : "crosshair";
  document.body.classList.toggle("tooling", next !== "move");
}

// ── Text ─────────────────────────────────────────────────────────────────────

let pendingAt: { x: number; y: number } | null = null;
let captionSize = 0;
let moving: { mark: Caption; dx: number; dy: number } | null = null;

/** The caption under a point, topmost first: what a press lands on when the text tool is in hand. */
function captionAt(x: number, y: number): Caption | null {
  for (let index = marks.length - 1; index >= 0; index -= 1) {
    const mark = marks[index];
    if (mark.kind !== "text") continue;
    const box = captionBox(mark);
    if (x >= box.x - 4 && x <= box.x + box.width + 4 && y >= box.y - 4 && y <= box.y + box.height + 4) {
      return mark;
    }
  }
  return null;
}

const MIN_CAPTION = 10;
const MAX_CAPTION = 120;

/** The field is its own preview: its letters are the size the caption will be, and the arrows
 *  change that while the words are being typed. */
function showCaptionSize() {
  textInput.style.fontSize = `${captionSize}px`;
  textInput.style.height = `${Math.round(captionSize * 1.7)}px`;
  textInput.title = `${captionSize}px`;
}

function placeText(event: PointerEvent) {
  const [x, y] = toPicture(event);
  pendingAt = { x, y };
  captionSize = textSize();
  textInput.value = "";
  // Kept inside the pin: a field hanging over the edge of a small crop would be clipped by the
  // window, and the picture must not move to make room for it.
  const width = Math.min(220, Math.max(70, window.innerWidth - 24));
  const height = Math.round(captionSize * 1.7);
  const left = Math.max(4, Math.min(event.clientX, window.innerWidth - width - 4));
  const top = Math.max(4, Math.min(event.clientY, window.innerHeight - height - 4));
  textInput.style.left = `${left}px`;
  textInput.style.top = `${top}px`;
  textInput.style.width = `${width}px`;
  showCaptionSize();
  textInput.hidden = false;
  textInput.focus();
  // A focused field is scrolled into view by the engine, which slides the picture under it.
  window.scrollTo(0, 0);
}

function commitText() {
  if (!pendingAt) return;
  const value = textInput.value.trim();
  const at = pendingAt;
  pendingAt = null;
  textInput.hidden = true;
  if (!value) return;
  marks.push({ kind: "text", color: inkColor, size: captionSize, x: at.x, y: at.y, text: value });
  redraw();
  void pushMarks();
}

textInput.addEventListener("keydown", (event) => {
  if (event.key === "ArrowUp" || event.key === "ArrowDown") {
    event.preventDefault();
    const step = event.key === "ArrowUp" ? 4 : -4;
    captionSize = Math.min(MAX_CAPTION, Math.max(MIN_CAPTION, captionSize + step));
    showCaptionSize();
    window.scrollTo(0, 0);
    return;
  }
  if (event.key === "Enter") {
    event.preventDefault();
    commitText();
  } else if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    pendingAt = null;
    textInput.hidden = true;
  }
});
textInput.addEventListener("blur", () => commitText());

// ── Tools ────────────────────────────────────────────────────────────────────

toolsToggle.addEventListener("click", () => {
  showTools(tools.hidden);
});

tools.addEventListener("click", (event) => {
  const target = event.target as HTMLElement;
  const swatch = target.closest("[data-color]") as HTMLElement | null;
  if (swatch) {
    inkColor = swatch.dataset.color ?? inkColor;
    for (const chip of tools.querySelectorAll("[data-color]")) {
      chip.classList.toggle("on", chip === swatch);
    }
    // Picking a colour means wanting to draw with it.
    if (tool === "move") setTool("pen");
    return;
  }
  const pick = target.closest("[data-tool]") as HTMLElement | null;
  if (pick?.dataset.tool) {
    setTool(pick.dataset.tool as "move" | "pen" | "text");
    return;
  }
  const action = target.closest("[data-action]") as HTMLElement | null;
  if (action?.dataset.action === "undo") {
    marks.pop();
    redraw();
    void pushMarks();
  } else if (action?.dataset.action === "clear") {
    marks = [];
    redraw();
    void pushMarks();
  }
});

ink.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  if (tool === "pen") {
    ink.setPointerCapture(event.pointerId);
    penning = { kind: "stroke", color: inkColor, width: penWidth(), points: [toPicture(event)] };
    redraw();
    event.preventDefault();
    return;
  }
  if (tool === "text") {
    event.preventDefault();
    const [x, y] = toPicture(event);
    const grabbed = captionAt(x, y);
    if (grabbed) {
      ink.setPointerCapture(event.pointerId);
      moving = { mark: grabbed, dx: x - grabbed.x, dy: y - grabbed.y };
      ink.style.cursor = "move";
      return;
    }
    placeText(event);
  }
});

ink.addEventListener("pointermove", (event) => {
  if (moving) {
    const [x, y] = toPicture(event);
    const box = captionBox(moving.mark);
    moving.mark.x = Math.max(2, Math.min(x - moving.dx, ink.width - box.width - 2));
    moving.mark.y = Math.max(2, Math.min(y - moving.dy, ink.height - box.height - 2));
    redraw();
    return;
  }
  if (!penning) return;
  penning.points.push(toPicture(event));
  redraw();
});

for (const done of ["pointerup", "pointercancel"]) {
  ink.addEventListener(done, () => {
    if (moving) {
      moving = null;
      ink.style.cursor = "text";
      redraw();
      note(`caption moved`);
      void pushMarks();
      return;
    }
    if (!penning) return;
    marks.push(penning);
    penning = null;
    redraw();
    note(`marks ${marks.length}`);
    void pushMarks();
  });
}

// ── Opacity, menu, picture ───────────────────────────────────────────────────

function setOpacity(percent: number) {
  const value = Math.min(100, Math.max(15, Math.round(percent)));
  shot.style.opacity = String(value / 100);
  ink.style.opacity = String(value / 100);
  for (const chip of opacityRow.querySelectorAll("button")) {
    chip.classList.toggle("on", Number(chip.dataset.opacity) === value);
  }
  note(`opacity ${value}%`);
}

function closeMenu() {
  if (menu.hidden) return;
  showTools(false);
  menu.hidden = true;
  releaseRoom();
  void invoke("pin_menu", { label: windowLabel, open: false, needW: 0, needH: 0 }).catch((err) =>
    note(`could not give the room back: ${err}`),
  );
}

function releaseRoom() {
  frame.style.inset = "";
  frame.style.left = "";
  frame.style.top = "";
  frame.style.width = "";
  frame.style.height = "";
}

/** The window is made bigger for as long as the menu is open; the picture keeps its place. */
async function openMenu(x: number, y: number) {
  if (!menu.hidden) return;
  // One popup at a time: the tools and the menu would fight for the same corner.
  showTools(false);
  note("menu open");
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

  // One box holds the picture and everything drawn on it, so the whole lot shifts together and
  // the crop stays where the window used to be.
  frame.style.inset = "auto";
  frame.style.left = `${offsetX}px`;
  frame.style.top = `${offsetY}px`;
  frame.style.width = `${picW}px`;
  frame.style.height = `${picH}px`;

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
showTools(false);
setTool("move");
// The ink sits over the picture and is scaled with it, so it needs no other coordination.
ink.style.pointerEvents = "none";

// A new crop for this window: the picture replaces the last one, and its marks go with it.
void listen<string>(`pin-image-${windowLabel}`, (event) => {
  note(`image ${event.payload.length} chars`);
  void closeMenu();
  releaseRoom();
  if (!textInput.hidden) {
    pendingAt = null;
    textInput.hidden = true;
  }
  marks = [];
  showTools(false);
  shot.src = event.payload;
  shot.hidden = false;
  setOpacity(100);
  setTool("move");
  void invoke("pin_mark", { label: windowLabel, data: "" }).catch(() => {});
}).catch((err) => note(`could not listen: ${err}`));

shot.addEventListener("load", () => {
  note(`drawn ${shot.naturalWidth}x${shot.naturalHeight}`);
  // The canvas keeps the picture's own pixel grid: a zoom scales it with the window and the
  // strokes stay where they were drawn.
  ink.width = Math.max(1, shot.naturalWidth);
  ink.height = Math.max(1, shot.naturalHeight);
  redraw();
});
shot.addEventListener("error", () => note("the picture would not load"));

// ── Actions ──────────────────────────────────────────────────────────────────

async function run(action: string) {
  void closeMenu();
  note(`action ${action}`);
  try {
    switch (action) {
      case "ask":
        // The pin stays: asking about a reference is not a reason to lose the reference.
        await pushMarks();
        await invoke("pin_ask", { label: windowLabel });
        break;
      case "copy-text": {
        const text = await invoke<string>("pin_copy_text", { label: windowLabel });
        flash(text.trim().length > 0 ? t("pin.textCopied") : t("pin.textNone"));
        break;
      }
      case "copy-image":
        await pushMarks();
        await invoke("pin_copy_image", { label: windowLabel });
        flash(t("pin.imageCopied"));
        break;
      case "save": {
        await pushMarks();
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

// Any click outside the popup puts it away, on the capture pass so the canvas drawing a stroke or
// the field taking focus cannot swallow it first.
document.addEventListener(
  "click",
  (event) => {
    const target = event.target as HTMLElement;
    // While a tool is in hand the popup stays: a stroke is a click too, and closing the palette on
    // every line would make drawing a chore.
    if (tool !== "move") return;
    if (!target.closest("#pin-tools, #pin-tools-toggle")) showTools(false);
  },
  true,
);

document.addEventListener("mousedown", (event) => {
  if (event.button !== 0) return;
  const inside = (event.target as HTMLElement).closest(
    "#pin-menu, #pin-close, #pin-keep, #pin-tools, #pin-tools-toggle, #pin-text-input",
  );
  if (inside) return;
  // A press on the picture puts the menu away first, since a press usually means a drag.
  if (!menu.hidden) {
    void closeMenu();
    return;
  }
  if (tool !== "move") return;
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
  if (!textInput.hidden) return;
  if (event.key === "Escape") {
    if (!menu.hidden) void closeMenu();
    else void run("close");
    return;
  }
  // M, P, T switch the tool, the way a drawing app does.
  const key = event.key.toLowerCase();
  if (key === "p") setTool("pen");
  else if (key === "t") setTool("text");
  else if (key === "m") setTool("move");
});

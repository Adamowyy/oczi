// The island: DOM shell, sizing animation, Iskra placement, mouse handling.
// Mirrors IslandRootView.swift + IslandWindowController.swift.

import { Tracked, Spring, clamp } from "../core/anim";
import { Bridge, IS_TAURI, onDragDrop } from "../core/bridge";
import {
  EXPANDED_CORNER, EXPANDED_W, NOTCH_H, NOTCH_W, PANEL_H, PANEL_W,
  ROUNDED_CORNER, VIEW_LAYOUTS, botGlowColor, botGlowOpacity, botPosition, chatPromptHeight,
  islandSize,
  type IslandMode, type IslandViewName,
} from "../core/layout";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import { BotEngine, hexToRGB } from "../bot/engine";
import { Greeting } from "../bot/greeting";
import { createMiniBot, miniBotCount, pruneMiniBots, syncMiniBotStates, tickMiniBots } from "../bot/minibots";
import { UploadCanvas } from "../upload/canvas";
import { USC, UploadSeq } from "../upload/sequence";
import { buildHeader, buildViews, type ViewActions, type ViewHost } from "../views/views";

/** What the selection overlay reports back. Zeros mean the user cancelled. */
type SnipBounds = { width: number; height: number; bytes: number };
import { h } from "../views/dom";
import { IslandStateMachine } from "./fsm";

const BOT_OVERHANG = 40;
/** Same margin as the Rust hit test (src-tauri/src/island.rs). */
const HIT_MARGIN = 14;
/** Frames a second on an untouched island; the engine is clock-driven, so the
 * same character at any rate. Anything the user touches runs full speed. */
const RESTING_FPS = 15;
/** Frames a second for an open card nobody is touching; 15 was visibly jerky there. */
const OPEN_FPS = 30;

/** The three views the drop sequence owns; leaving them stops the engine. */
const UPLOAD_VIEWS: ReadonlySet<IslandViewName> = new Set(["upload", "uploading", "choose"]);

/** Seconds between the drop and the moment the progress bar starts filling. */
const PRE_PROGRESS = USC.T_PROG_START - USC.T_DROP;

const modeOrder = (m: IslandMode) => (m === "hidden" ? 0 : m === "compact" ? 1 : 2);

export class Island {
  readonly fsm = new IslandStateMachine();

  private root: HTMLElement;
  private islandEl!: HTMLElement;
  private clipEl!: HTMLElement;
  private contentEl!: HTMLElement;
  private viewsEl!: HTMLElement;
  private botCanvas!: HTMLCanvasElement;
  private botGlow!: HTMLElement;
  /** Last glow values written, see updateBotTargets. A blurred rewrite is not free. */
  private awake = true;
  /** True while the card is growing or shrinking (see `#island.settling`). */
  private settling = false;
  /** The greeting is worth exactly one run per waking: the settle it waits for can
   *  happen twice, and the second call used to start it all over again. */
  private greetingStarted = false;
  private glowColor = "";
  private glowSize = -1;
  private glowPos = { x: -1, y: -1 };
  private glowOpacity = "";
  /** Rounded geometry last written; see applyGeometry. */
  private geometryKey = "";
  /** Last canvas position written; see drawBot. */
  private botPos = { x: -1, y: -1 };
  private greetingCanvas!: HTMLCanvasElement;
  private miniGrid!: HTMLElement;
  private countdown!: HTMLElement;
  private wakeStrip!: HTMLElement;

  private header!: ViewHost;
  private views!: Map<IslandViewName, ViewHost>;
  private uploadCanvas!: UploadCanvas;

  private width = new Tracked(NOTCH_W);
  private height = new Tracked(0);
  private radius = new Tracked(ROUNDED_CORNER);
  private botCx = new Spring(46);
  private botCy = new Spring(16);
  private botSize = new Spring(10);

  private engine = new BotEngine();
  private greeting = new Greeting();

  private running = false;
  private lastFrame = 0;
  private dirty = true;
  private canvasPx = 0;

  // Rust starts the window at full size so the launch greeting has room.
  private collapsed = false;
  private collapseTimer: number | null = null;
  private wasInIsland = false;
  /** Last shape handed to Rust for the click-through test. */
  private pushedRect = { x: -1, y: -1, w: -1, h: -1 };
  private homeCollapseAt: number | null = null;

  // Bot hover → love (IslandWindowController.botHoverIn)
  private botHovering = false;
  /** True once the character has been drawn this frame; see the frame loop. */
  private botShown = false;
  /** Cursor is on the island. Worth full frame rate: it is about to be used. */
  private hovered = false;
  /** The timer that holds a resting island at RESTING_FPS. */
  private restTimer: number | null = null;
  private botHoverTimer: number | null = null;
  private lastLoveTime = 0;
  private botHoverStart = { x: 0, y: 0 };

  private confusedRecovery: number | null = null;
  private prevViewBeforeConfused: IslandViewName = "overview";
  private lastSyncedView: IslandViewName | null = null;

  /** Drop sequence bookkeeping: last tick played, and whether the ✓ has fired. */
  private uploadTens = 0;
  private uploadDone = false;

  constructor(root: HTMLElement) {
    this.root = root;
    this.build();
    this.wireFsm();
    this.wireInput();
    this.engine.onDizzy = () => this.handleDizzy();
    this.greeting.onComplete = () => {
      this.greetingStarted = false;
      this.fsm.greetComplete();
    };
    State.subscribe(() => {
      this.dirty = true;
      this.ensureRunning();
    });
  }

  // ── DOM ─────────────────────────────────────────────────────────────────────

  private build() {
    const actions: ViewActions = {
      setView: (v) => this.setView(v),
      snip: () => void this.snipRegion(),
      // Asked for by the chat: hold the island open while a question is in flight and
      // while the answer is being read, even with the mouse nowhere near it.
      holdOpen: (seconds) => this.holdOpen(seconds),
      collapse: () => this.collapse(),
      setFocus: (id) => {
        State.setFocus(id);
        Sound.play("blip");
      },
      // The ↗ button, same targets as openAgentTarget() on macOS.
      openTarget: () => {
        const task = State.focusTask;
        if (!task) return;
        const urls: Record<string, string> = {
          integration_resend: "https://resend.com/emails",
          integration_vercel: "https://vercel.com/dashboard",
          integration_github: "https://github.com",
          integration_stripe: "https://dashboard.stripe.com/payments",
          integration_notion: "https://notion.so",
          integration_calcom: "https://app.cal.com/bookings",
        };
        if (task.id === "integration_n8n") void Bridge.openN8n();
        else if (urls[task.id]) void Bridge.openUrl(urls[task.id]);
      },
      openUrl: (url) => {
        if (url) void Bridge.openUrl(url);
      },
      setAutoClose: (s) => {
        State.settings.autoCloseInterval = s;
        this.fsm.homeToPetitDelay = s;
        void Bridge.saveSettings(State.settings);
        State.notify();
      },
      setNotchHide: (s) => {
        State.settings.notchHideInterval = s;
        this.fsm.petitToHiddenDelay = s;
        void Bridge.saveSettings(State.settings);
        State.notify();
      },
      openSettingsWindow: () => void Bridge.openSettingsWindow(),
      blip: () => Sound.play("blip"),
    };

    this.wakeStrip = h("div", { id: "wake-strip" });
    this.botGlow = h("div", { id: "bot-glow" });
    this.botCanvas = h("canvas", { id: "bot-canvas" });
    this.greetingCanvas = h("canvas", { id: "greeting-canvas" });
    this.miniGrid = h("div", { id: "mini-grid" });
    this.countdown = h("div", { id: "countdown" });

    this.header = buildHeader(actions);
    this.views = buildViews(actions, () => this.animateGeometry(false));
    this.viewsEl = h("div", { id: "views" });
    for (const v of this.views.values()) this.viewsEl.append(v.el);
    this.contentEl = h("div", { id: "content" }, this.header.el, this.viewsEl);

    // The drop sequence draws the card, the bar and its own Iskra. It sits under
    // the header, which stays visible on top of it exactly as on macOS.
    this.uploadCanvas = new UploadCanvas({
      ask: () => {
        State.promptContext = State.droppedFile
          ? { kind: "file", name: State.droppedFile.name, path: State.droppedFile.path }
          : null;
        this.setView("prompt");
      },
      cancel: () => this.setView(State.defaultView()),
    });

    this.clipEl = h(
      "div",
      { id: "island-clip" },
      this.greetingCanvas,
      this.uploadCanvas.el,
      this.contentEl,
    );
    this.islandEl = h(
      "div",
      { id: "island" },
      this.clipEl,
      this.botGlow,
      this.botCanvas,
      this.miniGrid,
      this.countdown,
    );

    const dpr = Math.min(2, window.devicePixelRatio || 1);
    this.greetingCanvas.width = Math.round(EXPANDED_W * dpr);
    this.greetingCanvas.height = Math.round(150 * dpr);
    this.greetingCanvas.style.width = `${EXPANDED_W}px`;
    this.greetingCanvas.style.height = "150px";

    this.root.append(this.wakeStrip, this.islandEl);
    this.bindDrag();
    this.applyGeometry();
  }

  // ── FSM ─────────────────────────────────────────────────────────────────────

  private wireFsm() {
    this.fsm.homeToPetitDelay = State.settings.autoCloseInterval;
    this.fsm.petitToHiddenDelay = State.settings.notchHideInterval;
    this.fsm.onTransition = (from, to) => {
      switch (to) {
        case "hidden":
          this.setMode("hidden");
          break;
        case "petit":
          if (from === "greeting") this.greeting.interrupt();
          else if (from === "hidden") Sound.play("peek");
          this.setMode("compact");
          if (from === "greeting") State.view = State.defaultView();
          if (!this.wasInIsland) this.fsm.mouseLeft();
          break;
        case "home":
          this.expand(State.defaultView());
          if (!this.wasInIsland) this.fsm.mouseLeft();
          break;
        case "greeting":
          this.expand("greeting");
          void Bridge.log("fsm -> greeting");
          this.startGreetingWhenSettled();
          break;
      }
      State.notify();
    };
  }

  launch() {
    this.fsm.launch();
  }

  // ── Mode / view ─────────────────────────────────────────────────────────────

  private setMode(mode: IslandMode) {
    const prev = State.mode;
    if (mode === prev) return;
    State.mode = mode;
    if (mode === "expanded") Sound.play("open");
    if (prev === "expanded") {
      Sound.play("close");
      State.isPinned = false;
      this.fsm.pinned = false;
      this.sticky = false;
      void Bridge.focusWindow(false);
    }
    if (mode !== "expanded") {
      this.engine.resetMorph();
      UploadSeq.deactivate();
      // A fold takes the island out of the upload flow no matter what view it
      // was on, so Rust must stop offering the copy cursor.
      this.uploadPin = false;
      void Bridge.setAcceptDrops(false);
    }
    this.updateWindowCollapsed();
    this.animateGeometry(modeOrder(mode) < modeOrder(prev));
    State.notify();
  }

  /** True while the drop sequence owns the island body. */
  private get uploadActive(): boolean {
    return State.mode === "expanded" && UploadSeq.isActive && UPLOAD_VIEWS.has(State.view);
  }

  /** Navigating out of the drop flow ends the sequence, as on macOS. */
  private stopSequenceIfLeaving(view: IslandViewName) {
    if (UploadSeq.isActive && !UPLOAD_VIEWS.has(view)) UploadSeq.deactivate();
  }

  expand(view: IslandViewName) {
    this.stopSequenceIfLeaving(view);
    State.view = view;
    this.syncDropPin(view);
    if (State.mode !== "expanded") this.setMode("expanded");
    else this.animateGeometry(false);
    State.lastActivity = performance.now();
    this.homeCollapseAt = null;
    State.notify();
  }

  setView(view: IslandViewName) {
    this.stopSequenceIfLeaving(view);
    if (State.mode !== "expanded") {
      this.fsm.forceHome();
      State.view = view;
      this.syncDropPin(view);
      this.animateGeometry(false);
      State.notify();
      return;
    }
    const grew = VIEW_LAYOUTS[view].height >= VIEW_LAYOUTS[State.view].height;
    State.view = view;
    this.syncDropPin(view);
    State.lastActivity = performance.now();
    this.animateGeometry(!grew);
    State.notify();
  }

  collapse() {
    if (State.view === "note") {
      State.noteMessage = null;
      State.view = State.defaultView();
    }
    State.isPinned = false;
    this.fsm.pinned = false;
    // Drive the state machine, not the mode: setting the mode behind its back desyncs it.
    this.fsm.forcePetit();
  }

  /** Holds the island open for `seconds`, or indefinitely when 0. Sending a question
   *  means the mouse is away, so the usual countdown would fold it away mid-answer. */
  holdOpen(seconds: number) {
    State.isPinned = true;
    this.fsm.pinned = true;
    if (this.holdTimer !== null) {
      window.clearTimeout(this.holdTimer);
      this.holdTimer = null;
    }
    if (seconds <= 0) return;
    this.holdTimer = window.setTimeout(() => {
      this.holdTimer = null;
      State.isPinned = false;
      this.fsm.pinned = false;
      // The island stays for the full interval from now, and only after a real mouse leave.
      this.homeCollapseAt = performance.now() + State.settings.autoCloseInterval * 1000;
      if (this.fsm.state === "home" && !this.wasInIsland) this.fsm.mouseLeft();
    }, seconds * 1000);
  }

  /** Alert from the hook server: open on this view. Pinned alerts never auto-close. */
  alert(view: IslandViewName) {
    this.fsm.pinned = State.isPinned;
    this.fsm.forceHome();
    this.expand(view);
  }

  reveal() {
    this.fsm.reveal();
  }

  /** The summon hotkey asks for the chat with the caret in the field, the island opens on the view it shows.
   */
  revealOrOpen() {
    if (State.mode === "expanded") {
      // Already open: show the chat and take the keyboard. forceHome also settles
      // the FSM out of "greeting", whose timer would fold the island mid-question.
      this.fsm.forceHome();
      this.expand("prompt");
      this.focusChat();
      return;
    }
    // A transition into the state the machine already believes it is in is ignored,
    // so take the long way round; updateWindowCollapsed clears the hidden step.
    this.fsm.forceHidden();
    this.fsm.forceHome();
    this.expand("prompt");
    this.focusChat();
  }

  /** Hands the keyboard to the chat field. Reopening on the same view is not a view
   *  *change*, so clearing `lastSyncedView` makes the sync path take focus again. */
  private focusChat() {
    this.lastSyncedView = null;
    this.takeKeyboardFocus();
  }

  /** Activates the window (WS_EX_NOACTIVATE is off in the chat) and focuses the input. */
  private takeKeyboardFocus() {
    void Bridge.focusWindow(true);
    window.setTimeout(() => this.views.get("prompt")?.focus?.(), 120);
  }

  private snipping = false;
  private snipWaiter: ((info: SnipBounds) => void) | null = null;
  private holdTimer: number | null = null;
  private uploadPin = false;
  /** True while the view on screen is one the user works in rather than looks at. */
  private sticky = false;

  /** The eye. Freezes the screen and attaches the dragged region to the chat, waiting
   *  for the user's own question. Rust hides the island so it cannot shoot itself. */
  private async snipRegion() {
    if (this.snipping) return;
    this.snipping = true;
    try {
      // The overlay answers with an event rather than with the command's result: the
      // user takes as long as they take to drag the box.
      const answered = new Promise<SnipBounds>((resolve) => {
        this.snipWaiter = resolve;
      });
      await Bridge.beginSnip();
      const info = await answered;
      if (info.width === 0) return; // they backed out, nothing to say
      // Pinned, not sent: the input takes the caret and the picture waits there until
      // the user asks something about it.
      State.snip = { width: info.width, height: info.height };
      this.setView("prompt");
      window.setTimeout(() => this.views.get("prompt")?.focus?.(), 140);
    } catch (err) {
      State.noteMessage = String(err).replace(/^Error:\s*/, "");
      State.stateOverride = null;
      this.setView("note");
      Sound.play("error");
    } finally {
      this.snipping = false;
      this.snipWaiter = null;
    }
  }

  /** The overlay finished, with a region, or with nothing at all (Esc). */
  snipDone(info: SnipBounds) {
    const waiter = this.snipWaiter;
    this.snipWaiter = null;
    waiter?.(info);
  }

  /** Ctrl+Alt+S, and the eye in the chat bar. */
  snipStart() {
    void this.snipRegion();
  }

  /** The greeting opens with a shape that rushes into Iskra, so it starts after the first frame. */
  private startGreetingWhenSettled() {
    const started = performance.now();
    const tick = window.setInterval(() => {
      // The greeting view has to be up before the one-run guard latches, not just the shape still.
      const ready = this.greetingViewUp() && this.height.value > NOTCH_H * 1.25;
      if (!ready && performance.now() - started < 1500) return;
      window.clearInterval(tick);
      // Someone answered already: the greeting is not worth interrupting them for.
      if (this.greetingViewUp() && !this.greetingStarted) {
        this.greetingStarted = true;
        this.greeting.start();
        void Bridge.log(`greeting start @${Math.round(performance.now() - started)}ms`);
      } else {
        void Bridge.log(`greeting skipped view=${State.view} mode=${State.mode} started=${this.greetingStarted}`);
      }
    }, 30);
  }

  /** True while the greeting has the island: its canvas is only drawn then. */
  private greetingViewUp(): boolean {
    return State.mode === "expanded" && State.view === "greeting";
  }

  /** A click elsewhere in Windows. The island takes the hint, but not while a question
   *  is in flight, that click is the user going back to work, not "done reading". */
  clickOutside() {
    if (UPLOAD_VIEWS.has(State.view)) return;
    if (State.stateOverride === "thinking") return;
    if (State.mode === "expanded") this.collapse();
  }

  /** The drop prompt holds the island open, and is the one place it becomes a drop
   *  target. The countdown is armed and dropped here: only a view change moves it. */
  private syncDropPin(view: IslandViewName) {
    const wanted = UPLOAD_VIEWS.has(view);
    void Bridge.setAcceptDrops(wanted);
    if (wanted !== this.uploadPin) {
      this.uploadPin = wanted;
      this.holdOpen(wanted ? 0 : State.settings.autoCloseInterval);
    }

    // The chat is typed in, so it stays pinned until dismissed by click-outside or
    // Esc. Every other view keeps the popover's own countdown.
    const sticky = view === "prompt";
    if (sticky === this.sticky) return;
    this.sticky = sticky;
    if (sticky) {
      this.holdOpen(0);
    } else {
      // Back to a view the user is only looking at: unpin and arm the countdown the
      // way the cursor leaving the island does.
      State.isPinned = false;
      this.fsm.pinned = false;
      this.homeCollapseAt = performance.now() + State.settings.autoCloseInterval * 1000;
      this.fsm.mouseLeft();
    }
  }

  /** An alert stopped waiting for an answer: let the island auto-close again. */
  dropPin() {
    this.fsm.pinned = false;
  }

  // ── File drop ───────────────────────────────────────────────────────────────

  private onDragDrop(e: { type: string; paths?: string[] }) {
    if (e.type !== "over") void Bridge.log(`drag ${e.type} ${e.paths?.length ?? 0} file(s)`);
    if (State.paused) return;
    // Files are only taken in the explicit upload flow (the plus tab). A stray drag
    // elsewhere must never flip views out from under the user.
    const inFlow = State.mode === "expanded" && UPLOAD_VIEWS.has(State.view);
    switch (e.type) {
      case "enter":
      case "over": {
        if (!inFlow || State.fileDragOver) return;
        State.fileDragOver = true;
        this.engine.animateMorph(1);
        UploadSeq.enterZone(State.mouseInIsland.x, State.mouseInIsland.y);
        break;
      }
      case "leave": {
        if (!State.fileDragOver) return;
        State.fileDragOver = false;
        this.engine.animateMorph(0);
        // The island deliberately stays open: the drag session is still alive.
        UploadSeq.exitZone();
        State.notify();
        break;
      }
      case "drop": {
        if (!inFlow) return;
        State.fileDragOver = false;
        const path = e.paths?.[0];
        if (!path) {
          this.engine.animateMorph(0);
          this.setView(State.defaultView());
          return;
        }
        this.swallow(path);
        break;
      }
    }
  }

  /** Iskra eats the file. The inbox copy runs in the background, so a slow disk
   *  cannot stall the animation, same as FileDropHandler on macOS. */
  private swallow(path: string) {
    const name = path.split(/[\\/]/).pop() || "file";
    State.droppedFile = { name, path };
    State.promptContext = { kind: "file", name, path };
    State.chatHistory = [];
    void Bridge.chatReset();

    if (!UploadSeq.isActive) {
      UploadSeq.enterZone(State.mouseInIsland.x, State.mouseInIsland.y);
    }
    UploadSeq.performDrop(State.uploadDuration);
    this.uploadTens = 0;
    this.uploadDone = false;

    this.engine.gulp();
    Sound.play("approve");
    this.engine.triggerEmote("happy");
    this.engine.animateMorph(0);

    State.uploadProgress = 0;
    this.setView("uploading");
    this.ensureRunning();

    void Bridge.ingestFile(path)
      .then((file) => {
        State.droppedFile = { name: file.name, path: file.path };
        State.promptContext = { kind: "file", name: file.name, path: file.path };
        State.notify();
      })
      .catch((err) => {
        UploadSeq.deactivate();
        State.noteMessage = String(err).replace(/^Error:\s*/, "");
        this.engine.animateMorph(0);
        this.setView("note");
        Sound.play("error");
        window.setTimeout(() => this.setView(State.defaultView()), 2400);
      });
  }

  /** Sounds and view changes off the canvas timeline: a `tick` every 10 %, then
   *  `choose` once Iskra has grown back. */
  private stepSequence() {
    const since = UploadSeq.sinceDrop();
    if (since == null) return;
    const dur = State.uploadDuration;
    const p = Math.max(0, Math.min(1, (since - PRE_PROGRESS) / dur));

    const tens = Math.floor(p * 10);
    if (tens > this.uploadTens && tens < 10) {
      this.uploadTens = tens;
      Sound.play("tick");
    }

    // The DOM uploading view reads this; the canvas draws its own copy.
    State.uploadProgress = Math.max(0, Math.min(1, p));

    if (!this.uploadDone && since >= PRE_PROGRESS + dur) {
      this.uploadDone = true;
      Sound.play("approve");
      this.engine.triggerEmote("happy");
    }
    // The extra second is the grow-back, after which the choose card is up.
    if (since >= PRE_PROGRESS + dur + 1 && State.view === "uploading") {
      this.setView("choose");
    }
  }

  // ── Geometry ────────────────────────────────────────────────────────────────

  private targetSize(): { w: number; h: number; r: number } {
    const { w, h } = islandSize(State.mode, State.view, State.chatHistory.length);
    const r = State.mode === "expanded" ? EXPANDED_CORNER : ROUNDED_CORNER;
    return { w, h, r };
  }

  private animateGeometry(shrinking: boolean) {
    const { w, h, r } = this.targetSize();
    if (shrinking) {
      this.width.curveTowards(w);
      this.height.curveTowards(h);
      this.radius.curveTowards(r);
    } else {
      this.width.springTo(w);
      this.height.springTo(h);
      this.radius.springTo(r);
    }
    this.ensureRunning();
  }

  private applyGeometry() {
    const w = this.width.value;
    const hh = this.height.value;
    const r = this.radius.value;
    // Written only when the rounded values move: the island is settled for most
    // of its life, and a style write is never quite free.
    const key = `${Math.round(w * 2)}|${Math.round(hh * 2)}|${Math.round(r * 2)}`;
    if (key !== this.geometryKey) {
      this.geometryKey = key;
      this.islandEl.style.width = `${w}px`;
      this.islandEl.style.height = `${hh}px`;
      this.islandEl.style.borderRadius = `0 0 ${r}px ${r}px`;
      this.islandEl.style.transform = `translateX(-50%)`;
      // These follow the island as it resizes, so they belong here rather than
      // in the state-driven DOM sync.
      this.miniGrid.style.right = "14px";
      this.miniGrid.style.top = `${hh / 2}px`;
      this.greetingCanvas.style.left = `${(w - EXPANDED_W) / 2}px`;
      this.uploadCanvas.el.style.left = `${(w - EXPANDED_W) / 2}px`;
    }

    const settled = this.targetSize();
    const settling = this.width.animating || this.height.animating;
    if (settling !== this.settling) {
      this.settling = settling;
      this.islandEl.classList.toggle("settling", settling);
    }

    const rect = { x: (PANEL_W - settled.w) / 2, y: 0, w: settled.w, h: settled.h };
    const p = this.pushedRect;
    if (Math.abs(p.x - rect.x) > 0.5 || Math.abs(p.w - rect.w) > 0.5 || Math.abs(p.h - rect.h) > 0.5) {
      this.pushedRect = rect;
      void Bridge.setIslandRect(rect.x, rect.y, rect.w, rect.h);
    }
  }

  /** Island rect in window coordinates (origin top-left of the 720×320 window). */
  private islandRect(): { x: number; y: number; w: number; h: number } {
    const w = this.width.value;
    const hh = this.height.value;
    return { x: (PANEL_W - w) / 2, y: 0, w, h: hh };
  }

  // ── Window collapse (hidden → tiny wake strip, zero polling) ────────────────

  private updateWindowCollapsed() {
    if (this.collapseTimer != null) {
      window.clearTimeout(this.collapseTimer);
      this.collapseTimer = null;
    }
    if (State.mode === "hidden") {
      // Let the island finish retracting, then drop the window to the wake strip:
      // from there the OS delivers no cursor events, so nothing polls at all.
      this.collapseTimer = window.setTimeout(() => {
        this.collapseTimer = null;
        if (State.mode !== "hidden") return;
        this.collapsed = true;
        void Bridge.setCollapsed(true);
      }, 420);
    } else if (this.collapsed) {
      // Grow the window back before the island animates open.
      this.collapsed = false;
      void Bridge.setCollapsed(false);
    }
  }

  // ── Input ───────────────────────────────────────────────────────────────────

  private wireInput() {
    // The wake strip is the only thing the OS can hit while the island is hidden.
    this.wakeStrip.addEventListener("mouseenter", () => {
      Sound.resume();
      if (State.mode === "hidden") this.fsm.mouseEntered();
    });

    this.islandEl.addEventListener("mousedown", (e) => {
      Sound.resume();
      State.lastActivity = performance.now();
      if (State.mode !== "expanded") {
        // A click on the compact island is an intent to type, so it opens the chat
        // with the caret already in the field.
        this.revealOrOpen();
        return;
      }
      if (this.isBotHit(e.clientX, e.clientY)) {
        this.cancelBotHover();
        this.engine.slap();
      }
    });

    window.addEventListener("keydown", (e) => {
      // Escape always closes, even while a question is in flight: it is an explicit
      // instruction, and collapse() unpins on the way out.
      if (e.key === "Escape" && State.mode === "expanded") this.collapse();
      State.lastActivity = performance.now();
    });

    void onDragDrop((e) => this.onDragDrop(e));

    // Outside Tauri (plain browser) drive the cursor from DOM events so the
    // island can be inspected with `npm run dev`.
    if (!IS_TAURI) {
      window.addEventListener("mousemove", (e) => this.onCursor(e.clientX, e.clientY));
    }
  }

  /** Cursor in window-logical coordinates. */
  onCursor(x: number, y: number) {
    State.mouse = { x, y };
    const rect = this.islandRect();
    State.mouseInIsland = { x: x - rect.x, y: y - rect.y };

    // Windows sends no cursor position with an OLE drag, so the drop sequence is
    // fed from the Win32 cursor poll instead, it runs throughout the drag.
    if (UploadSeq.isActive && !UploadSeq.dropped) {
      UploadSeq.updateCursor(State.mouseInIsland.x, State.mouseInIsland.y);
    }

    const inIsland =
      x >= rect.x - HIT_MARGIN && x <= rect.x + rect.w + HIT_MARGIN &&
      y >= rect.y - HIT_MARGIN && y <= rect.y + rect.h + HIT_MARGIN;

    const wasHovered = this.hovered;
    this.hovered = inIsland;
    if (wasHovered !== this.hovered) this.syncAsleep();
    if (inIsland && !this.wasInIsland) {
      if (this.fsm.state === "greeting") this.greeting.hover();
      this.fsm.mouseEntered();
      this.homeCollapseAt = null;
    }
    if (!inIsland && this.wasInIsland) {
      this.fsm.mouseLeft();
      if (this.fsm.state === "home" && !State.isPinned) {
        this.homeCollapseAt = performance.now() + State.settings.autoCloseInterval * 1000;
      }
    }
    this.wasInIsland = inIsland;

    // Bot hover → love
    const overBot = State.mode === "expanded" && State.stateOverride == null && this.isBotHit(x, y);
    if (overBot && !this.botHovering) this.botHoverIn(x, y);
    if (!overBot && this.botHovering) this.cancelBotHover();
    this.botHovering = overBot;
    if (this.botHovering) {
      const d = Math.hypot(x - this.botHoverStart.x, y - this.botHoverStart.y);
      if (d > 40) {
        this.botHoverStart = { x, y };
        this.scheduleLove();
      }
    }

    this.ensureRunning();
  }

  private isBotHit(x: number, y: number): boolean {
    const rect = this.islandRect();
    const cx = rect.x + this.botCx.value;
    const cy = rect.y + this.botCy.value;
    const radius = this.botSize.value / 2;
    return (x - cx) ** 2 + (y - cy) ** 2 <= radius * radius;
  }

  private botHoverIn(x: number, y: number) {
    if (performance.now() / 1000 - this.lastLoveTime < 6) return;
    this.botHoverStart = { x, y };
    this.engine.blink();
    this.engine.tgEs = 1.08;
    Sound.play("hover");
    this.scheduleLove();
  }

  private scheduleLove() {
    if (this.botHoverTimer != null) window.clearTimeout(this.botHoverTimer);
    this.botHoverTimer = window.setTimeout(() => {
      this.botHoverTimer = null;
      if (!this.botHovering || State.stateOverride != null) return;
      if (performance.now() / 1000 - this.lastLoveTime < 6) return;
      this.lastLoveTime = performance.now() / 1000;
      this.engine.triggerEmote("love");
      Sound.play("love");
    }, 1900);
  }

  private cancelBotHover() {
    if (this.botHoverTimer != null) window.clearTimeout(this.botHoverTimer);
    this.botHoverTimer = null;
    this.engine.tgEs = 1;
  }

  /** Three slaps → dizzy + confused view for 3.3 s, then back. */
  private handleDizzy() {
    this.prevViewBeforeConfused = State.view;
    State.stateOverride = "dizzy";
    this.engine.setState("dizzy");
    Sound.play("dizzy");
    this.alert("confused");
    if (this.confusedRecovery != null) window.clearTimeout(this.confusedRecovery);
    this.confusedRecovery = window.setTimeout(() => {
      this.confusedRecovery = null;
      State.stateOverride = null;
      this.engine.setState(State.effectiveState);
      if (State.view === "confused") {
        const fallback = State.defaultView();
        this.setView(this.prevViewBeforeConfused === "confused" ? fallback : this.prevViewBeforeConfused);
      }
      this.engine.triggerEmote("happy");
    }, 3300);
  }

  // ── Frame loop ──────────────────────────────────────────────────────────────

  ensureRunning() {
    if (this.running) return;
    this.running = true;
    this.lastFrame = performance.now();
    requestAnimationFrame(this.frame);
  }

  private frame = (nowMs: number) => {
    const dt = Math.min(0.05, (nowMs - this.lastFrame) / 1000);
    this.lastFrame = nowMs;

    this.width.step(dt, nowMs);
    this.height.step(dt, nowMs);
    this.radius.step(dt, nowMs);
    this.applyGeometry();

    if (this.dirty) {
      this.dirty = false;
      this.syncDom();
    }

    this.updateBotTargets();
    this.botCx.step(dt);
    this.botCy.step(dt);
    this.botSize.step(dt);

    const greetingActive = this.greetingViewUp() && this.greetingStarted;
    if (greetingActive) {
      const gctx = this.greetingCanvas.getContext("2d");
      if (gctx) {
        const dpr = Math.min(2, window.devicePixelRatio || 1);
        gctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        this.greeting.draw(gctx);
      }
    } else {
      // Kept running even while the drop canvas is up, so the island's own Iskra
      // is already in the right place the moment the canvas fades out.
      this.drawBot(dt);
    }

    const uploadActive = this.uploadActive;
    if (uploadActive) this.uploadCanvas.draw(UploadSeq.frame(), nowMs / 1000);
    this.uploadCanvas.el.classList.toggle("on", uploadActive);
    this.viewsEl.classList.toggle("hidden-by-upload", uploadActive);

    tickMiniBots(dt);
    this.views.get(State.view)?.tick?.(nowMs);
    if (UploadSeq.isActive) this.stepSequence();
    this.updateCountdown(nowMs);

    // Nothing is drawn while hidden, so nothing may keep the loop alive either, 
    // engine.busy is permanently true for any looping animation. Geometry still retracts.
    const settling =
      this.width.animating || this.height.animating || this.radius.animating;
    // Everything on screen that moves keeps the loop alive: the character (shards,
    // blinks, eye drift) and the mini characters, which run engines of their own.
    const alive =
      this.botShown ||
      miniBotCount() > 0 ||
      greetingActive ||
      this.engine.busy ||
      UploadSeq.isActive;
    const busy = State.mode === "hidden"
      ? settling
      : settling ||
        !this.botCx.settled || !this.botCy.settled || !this.botSize.settled ||
        alive;

    if (!busy) {
      this.running = false;
      Sound.idle();
      return;
    }

    // Nothing on screen but a living character: everything else drops to RESTING_FPS.
    // The wait is scheduled, not frames skipped, so the browser can actually sleep.
    const still =
      !settling &&
      !this.hovered &&
      !greetingActive &&
      !this.uploadActive &&
      !UploadSeq.isActive &&
      // The chat is the one card where the extra frames are felt: reading,
      // scrolling and the caret.
      State.view !== "prompt";

    const fps = !still ? 0 : State.mode === "expanded" ? OPEN_FPS : RESTING_FPS;

    if (fps > 0) {
      if (this.restTimer !== null) window.clearTimeout(this.restTimer);
      this.restTimer = window.setTimeout(() => {
        this.restTimer = null;
        requestAnimationFrame(this.frame);
      }, 1000 / fps - 4);
    } else {
      if (this.restTimer !== null) {
        window.clearTimeout(this.restTimer);
        this.restTimer = null;
      }
      requestAnimationFrame(this.frame);
    }
  };

  /** Decorative card animations keep compositing at zero opacity, so they run only
   *  while the card is up or the cursor is on the island, see the `.asleep` rule. */
  private syncAsleep() {
    const awake = State.mode === "expanded" || this.hovered;
    if (!awake) this.awake = false;
    else if (!this.awake) {
      this.awake = true;
      this.forgetLaidOutSizes();
      this.reportWake();
    }
    document.documentElement.classList.toggle("asleep", !awake);
  }

  private reportWake() {
    const shot = (tag: string) => {
      const canvas = (id: string) => {
        const el = document.getElementById(id) as HTMLCanvasElement | null;
        if (!el) return `${id}=none`;
        const r = el.getBoundingClientRect();
        const off = getComputedStyle(el).display === "none" ? "!" : "";
        return `${id}=${Math.round(r.width)}x${Math.round(r.height)}/${el.width}x${el.height}${off}`;
      };
      const glow = `glow=${this.botGlow.style.display || "-"} ${this.botGlow.style.width || "-"} op=${this.botGlow.style.opacity || "-"}`;
      return `${tag} mode=${State.mode} view=${State.view} asleep=${document.documentElement.classList.contains("asleep")} upload=${this.uploadActive} ${canvas("bot-canvas")} ${canvas("greeting-canvas")} ${canvas("upload-canvas")} ${glow}`;
    };
    for (const delay of [120, 600]) {
      window.setTimeout(() => void Bridge.log(shot(`wake@${delay}`)), delay);
    }
  }

  private forgetLaidOutSizes() {
    this.glowSize = -1;
    this.glowPos = { x: -1, y: -1 };
    this.glowColor = "";
    this.glowOpacity = "";
    this.canvasPx = 0;
    this.botPos = { x: -1, y: -1 };
    this.geometryKey = "";
  }

  private bindDrag() {
    let dragging = false;
    let moved = false;
    let grabX = 0;
    let startAnchor = 0.5;

    const isControl = (target: EventTarget | null) =>
      target instanceof Element &&
      !!target.closest("button, input, select, textarea, a, .mini, .pill, .upload-hit");

    const down = (e: PointerEvent) => {
      if (e.button !== 0 || isControl(e.target)) return;
      // The drop card is a place where a press means something else.
      if (UPLOAD_VIEWS.has(State.view)) return;
      dragging = true;
      grabX = e.screenX;
      startAnchor = State.settings.islandAnchor;
      // Capture keeps the moves coming once the cursor is past the window edge, which
      // a fast drag does. Not worth failing the drag over.
      try {
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      } catch {
        /* no capture: the drag still works while the cursor stays over us */
      }
    };

    const move = (e: PointerEvent) => {
      if (!dragging) return;
      // A fraction of the room the island has to slide in, so the same number means
      // the same place on another screen or at another resolution.
      const span = Math.max(1, window.screen.availWidth - this.panelSize.w);
      const anchor = clamp(startAnchor + (e.screenX - grabX) / span, 0, 1);
      State.settings.islandAnchor = anchor;
      moved = true;
      void Bridge.setIslandAnchor(anchor, false);
    };

    const up = () => {
      if (!dragging) return;
      dragging = false;
      // A plain click on her is not a drag: no reason to write the settings for it.
      if (moved) void Bridge.setIslandAnchor(State.settings.islandAnchor, true);
      moved = false;
    };

    for (const el of [this.islandEl, this.wakeStrip]) {
      el.addEventListener("pointerdown", down);
      el.addEventListener("pointermove", move);
      el.addEventListener("pointerup", up);
      el.addEventListener("pointercancel", up);
    }
  }

  private updateBotTargets() {
    const p = botPosition(State.mode, State.view, this.height.value, State.uploadProgress);
    this.botCx.target = p.cx;
    this.botCy.target = p.cy;
    this.botSize.target = p.diameter / 0.6;

    const greetingActive = State.mode === "expanded" && State.view === "greeting";
    const grown = clamp(this.height.value / NOTCH_H, 0, 1);
    // She waits until the bar is a third of the way out, so the order on screen
    // is always the bar first, then her fading in.
    const shown = clamp((grown - 0.35) / 0.65, 0, 1);
    // The drop canvas draws its own Iskra; two of them would overlap.
    const visible = p.opacity > 0 && shown > 0.01 && !greetingActive && !this.uploadActive;
    this.botShown = visible;
    this.botCanvas.style.opacity = visible ? String(shown) : "0";

    if (State.mode === "expanded" && State.view !== "uploading" && !greetingActive && !this.uploadActive) {
      if (this.width.animating || this.height.animating) {
        this.botGlow.style.display = "none";
        return;
      }
      const d = p.diameter;
      const color = botGlowColor(State.effectiveState);
      if (this.botGlow.style.display !== "block") this.botGlow.style.display = "block";
      // Only what changed is written: re-writing the same gradient every frame makes
      // the browser re-rasterise the whole blurred square, the costliest card op.
      if (this.glowColor !== color) {
        this.glowColor = color;
        this.botGlow.style.background = `radial-gradient(circle, ${color} 0%, transparent 62%)`;
      }
      const size = d * 2.2;
      if (Math.abs(this.glowSize - size) > 0.5) {
        this.glowSize = size;
        this.botGlow.style.width = `${size}px`;
        this.botGlow.style.height = `${size}px`;
      }
      const left = this.botCx.value - d * 1.1;
      const top = this.botCy.value - d * 1.1;
      if (Math.abs(this.glowPos.x - left) > 0.5 || Math.abs(this.glowPos.y - top) > 0.5) {
        this.glowPos = { x: left, y: top };
        this.botGlow.style.transform = `translate3d(${left}px, ${top}px, 0)`;
      }
      const opacity = String(botGlowOpacity(State.effectiveState));
      if (this.glowOpacity !== opacity) {
        this.glowOpacity = opacity;
        this.botGlow.style.opacity = opacity;
      }
    } else if (this.botGlow.style.display !== "none") {
      this.botGlow.style.display = "none";
      this.glowColor = "";
    }
  }

  private drawBot(dt: number) {
    const size = this.botSize.value;
    const w = Math.max(1, Math.round(size));
    const hCss = w + BOT_OVERHANG;
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    if (this.canvasPx !== w) {
      this.canvasPx = w;
      this.botCanvas.width = Math.round(w * dpr);
      this.botCanvas.height = Math.round(hCss * dpr);
      this.botCanvas.style.width = `${w}px`;
      this.botCanvas.style.height = `${hCss}px`;
    }
    // transform, not left/top: the canvas follows the character every frame, and
    // a layout move invalidates the card around it.
    const bcx = this.botCx.value - w / 2;
    const bcy = this.botCy.value - BOT_OVERHANG / 2 - hCss / 2;
    if (Math.abs(this.botPos.x - bcx) > 0.5 || Math.abs(this.botPos.y - bcy) > 0.5) {
      this.botPos = { x: bcx, y: bcy };
      this.botCanvas.style.transform = `translate3d(${bcx}px, ${bcy}px, 0)`;
    }

    const ctx = this.botCanvas.getContext("2d");
    if (!ctx) return;

    const focus = State.focusTask;
    // A tint means "this colour belongs to the service you are looking at", so it
    // only applies on the Services view; elsewhere Iskra stays neutral.
    const tinted = State.view === "overview" && focus?.isIntegration === true;
    this.engine.bodyColor = tinted && focus ? hexToRGB(focus.color) : null;
    this.engine.particleOverhang = BOT_OVERHANG;
    this.engine.lookX = this.lookX();
    this.engine.lookY = this.lookY();
    if (this.engine.morph > 0.3) {
      this.engine.slotHTarget = State.fileDragOver ? 0.2 : 0;
    } else {
      this.engine.slotHTarget = 0;
      if (this.engine.morph < 0.05) {
        this.engine.slotH = 0;
        this.engine.slotHVel = 0;
      }
    }
    this.engine.update(dt);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, hCss);
    this.engine.draw(ctx, w, hCss);
  }

  /** BotCanvasView.lookX / lookY, tanh of the distance to the bot. */
  private lookX(): number {
    const rect = this.islandRect();
    const botScreenX = rect.x + this.botCx.value;
    return Math.tanh((State.mouse.x - botScreenX) / 260);
  }

  private lookY(): number {
    return -Math.tanh((State.mouse.y - this.botCy.value) / 200);
  }

  private updateCountdown(nowMs: number) {
    if (State.mode !== "expanded" || State.isPinned || this.homeCollapseAt == null) {
      this.countdown.style.width = "0px";
      return;
    }
    const autoClose = State.settings.autoCloseInterval;
    const windowS = Math.min(10, autoClose * 0.6);
    const remaining = (this.homeCollapseAt - nowMs) / 1000;
    this.countdown.style.width =
      remaining < windowS ? `${Math.max(0, clamp(remaining / windowS, 0, 1) * 160)}px` : "0px";
  }

  // ── DOM sync ────────────────────────────────────────────────────────────────

  private syncDom() {
    const expanded = State.mode === "expanded";
    this.syncAsleep();
    const greetingActive = expanded && State.view === "greeting";

    this.contentEl.style.opacity = expanded && !greetingActive ? "1" : "0";
    this.contentEl.style.pointerEvents =
      expanded && !greetingActive && !this.uploadActive ? "auto" : "none";
    this.greetingCanvas.style.display = greetingActive ? "block" : "none";

    this.header.sync();
    for (const [name, view] of this.views) {
      const on = name === State.view;
      view.el.classList.toggle("on", on);
      if (on) view.sync();
    }

    // The chat is the only view with a text field, so it is the only time the
    // island is allowed to take keyboard focus.
    if (this.lastSyncedView !== State.view) {
      const wasChat = this.lastSyncedView === "prompt";
      this.lastSyncedView = State.view;
      if (State.view === "prompt") {
        this.takeKeyboardFocus();
      } else if (wasChat) {
        void Bridge.focusWindow(false);
      }
    }

    // Compact: the summon chord, dimmed, with the mini integration mini characters beside it.
    const showGrid = State.mode === "compact";
    this.miniGrid.style.opacity = showGrid ? "1" : "0";
    if (showGrid) {
      const hotkey = State.settings.hotkey || "Ctrl+Alt+M";
      const others = State.otherTasks.slice(0, 4);
      const key = `${hotkey}|${others.map((t) => t.id).join("|")}`;
      if (this.miniGrid.dataset.key !== key) {
        this.miniGrid.dataset.key = key;
        this.miniGrid.replaceChildren();
        this.miniGrid.append(h("div", { class: "hotkey-hint", text: hotkey }));
        const grid = h("div", { class: "mini-grid-cells" });
        for (const t of others) grid.append(createMiniBot(t, 13));
        this.miniGrid.append(grid);
        pruneMiniBots();
      }
    }

    syncMiniBotStates(State.tasks);
    this.engine.setState(State.effectiveState);
  }

  /** Applies settings coming from Rust at boot. */
  applySettings() {
    Sound.setEnabled(State.settings.soundEnabled);
    Sound.setVolume(State.settings.soundVolume);
    this.fsm.homeToPetitDelay = State.settings.autoCloseInterval;
    this.fsm.petitToHiddenDelay = State.settings.notchHideInterval;
    State.notify();
  }

  get panelSize() {
    return { w: PANEL_W, h: PANEL_H };
  }

  get chatHeight() {
    return chatPromptHeight(State.chatHistory.length);
  }
}

// Island open/close FSM, port of IslandStateMachine.swift.
// No DOM, no Tauri: it only reports transitions.

export type FsmState = "hidden" | "petit" | "home" | "greeting";

export class IslandStateMachine {
  state: FsmState = "hidden";

  onTransition: ((from: FsmState, to: FsmState) => void) | null = null;

  /** home → petit delay, seconds. */
  homeToPetitDelay = 15;
  /** petit → hidden delay, seconds. */
  petitToHiddenDelay = 60;
  /** An alert waiting for an answer stays open, even when the mouse leaves. */
  pinned = false;

  private petitHide: number | null = null;
  private homeCollapse: number | null = null;
  /** The pointer is over the island, so a countdown means "waiting for it to leave". */
  private hovering = false;

  // ── Inputs ──────────────────────────────────────────────────────────────────

  launch() {
    this.cancelTimers();
    this.transition("greeting");
  }

  mouseEntered() {
    this.hovering = true;
    switch (this.state) {
      case "hidden":
        this.cancelTimers();
        this.transition("petit");
        break;
      case "petit":
        this.clear("petitHide");
        break;
      case "home":
        this.clear("homeCollapse");
        break;
      case "greeting":
        // The splash has its own clock and nothing may stretch it: a greeting that waits for
        // the pointer to leave is a greeting somebody sits and stares at.
        break;
    }
  }

  mouseLeft() {
    this.hovering = false;
    switch (this.state) {
      case "hidden":
        break;
      case "petit":
        this.schedulePetitHide();
        break;
      case "home":
        this.scheduleHomeCollapse();
        break;
      case "greeting":
        // Same as above: leaving does not end it early either, the clock does.
        break;
    }
  }

  click() {
    if (this.state !== "petit") return;
    this.cancelTimers();
    this.transition("home");
  }

  /** The greeting animation finished. The splash hands straight over to the main screen —
   *  landing on the bare bar meant the first thing anyone saw was the thing they did not
   *  open the app for. */
  greetComplete() {
    if (this.state !== "greeting") return;
    this.transition("home");
  }

  /** Non-alert work event: show compact from hidden. */
  reveal() {
    if (this.state !== "hidden") return;
    this.cancelTimers();
    this.transition("petit");
    this.schedulePetitHide();
  }

  /** Alert or explicit request: open straight to expanded. */
  forceHome() {
    this.cancelTimers();
    this.transition("home");
  }

  /// Explicit close (OK button, Escape, an alert being answered).
  forcePetit() {
    this.cancelTimers();
    this.transition("petit");
  }

  /** The island is on screen because something opened it from the bar, so the machine has to
   *  agree. `expand()` sets the render mode without moving the machine, which left the bar's
   *  own `petitHide` counting down under an expanded island; it fired into `hidden` and took
   *  the window with it mid-sentence. Drops every timer the machine armed for itself and
   *  returns whether one of them would have hidden the island. */
  settleOpen(): boolean {
    const wasFolding = this.petitHide != null && this.state !== "hidden";
    this.cancelTimers();
    // Never claim `home` while the greeting has the stage: the greeting folds through its own
    // transition, and a machine already in `home` makes `greetComplete` a no-op, which left the
    // greeting on screen with nothing left to end it.
    if (this.state !== "home" && this.state !== "greeting") this.state = "home";
    return wasFolding;
  }

  forceHidden() {
    this.cancelTimers();
    this.transition("hidden");
  }

  // ── Timers ──────────────────────────────────────────────────────────────────

  private schedulePetitHide() {
    this.clear("petitHide");
    this.petitHide = window.setTimeout(() => {
      this.petitHide = null;
      if (this.state === "petit") this.transition("hidden");
    }, this.petitToHiddenDelay * 1000);
  }

  private scheduleHomeCollapse() {
    this.clear("homeCollapse");
    if (this.pinned) return;
    this.homeCollapse = window.setTimeout(() => {
      this.homeCollapse = null;
      // Re-checked when it fires: a chat turn can pin the island after this was armed.
      if (this.state === "home" && !this.pinned) this.transition("petit");
    }, this.homeToPetitDelay * 1000);
  }

  private clear(which: "petitHide" | "homeCollapse") {
    const id = this[which];
    if (id != null) window.clearTimeout(id);
    this[which] = null;
  }

  cancelTimers() {
    this.clear("petitHide");
    this.clear("homeCollapse");
  }

  /** Where the machine stands, for a log line that has to explain a card that never came. */
  status(): string {
    return `${this.state} hover=${this.hovering}`;
  }

  private transition(next: FsmState) {
    if (next === this.state) return;
    const from = this.state;
    this.state = next;
    this.onTransition?.(from, next);
  }
}

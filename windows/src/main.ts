// Entry point: boot the bridge, wire the island, start the greeting.

import "./style.css";
import { Bridge, IS_TAURI, onEvent } from "./core/bridge";
import { Sound } from "./core/sound";
import { State, type Settings } from "./core/state";
import { currentLang, setLang, storedLang, t } from "./core/i18n";
import { newsKey } from "./core/whats-new";
import { Island } from "./island/island";
import { registerIntegrationHandlers, refreshConfigured } from "./island/integrations";

async function main() {
  const root = document.getElementById("root");
  if (!root) return;

  void Sound.preload();

  const builtWith = currentLang();
  const island = new Island(root);

  const boot = await Bridge.boot();
  if (boot) {
    State.settings = { ...State.settings, ...boot.settings };
    setLang(State.settings.language);
  }
  // The stored home view may differ from the one already built, so reload it.
  if (State.settings.language !== builtWith && storedLang() === State.settings.language) {
    location.reload();
    return;
  }
  island.applySettings();
  State.loadIntegrationTasks();

  await onEvent<{ x: number; y: number }>("cursor", ({ x, y }) => island.onCursor(x, y));

  /** Pause has to reach Rust too, or the pollers keep calling out. */
  const setPaused = (on: boolean) => {
    if (State.paused === on) return;
    State.paused = on;
    void Bridge.setPaused(on);
  };

  await onEvent<string>("tray", (what) => {
    switch (what) {
      case "settings":
        setPaused(false);
        island.alert("settings");
        break;
      case "open":
        setPaused(false);
        island.alert(State.defaultView());
        break;
      case "pause":
        setPaused(!State.paused);
        if (State.paused) island.fsm.forceHidden();
        else island.reveal();
        break;
    }
  });

  await onEvent<null>("screen-changed", () => void Bridge.reposition());

  // Ctrl+Alt+M, registered by Rust. Same as clicking the island: show it.
  await onEvent<null>("hotkey", () => {
    setPaused(false);
    island.revealOrOpen();
  });

  // Ctrl+Alt+Shift+S, registered by Rust. Same as clicking the eye.
  await onEvent<null>("hotkey-snip", () => {
    setPaused(false);
    island.snipStart();
  });

  // A click anywhere else in Windows. The island cannot hear it, it is click-through
  // whenever the mouse is away, so Rust watches the mouse button and tells us.
  await onEvent<null>("click-outside", () => island.clickOutside());

  // The selection overlay answered, with a region, or with nothing (Esc).
  await onEvent<{ width: number; height: number; bytes: number }>("snip-done", (info) =>
    island.snipDone(info),
  );

  // The settings window writes preferences; apply them here without a restart.
  await onEvent<Settings>("settings-changed", (s) => {
    const languageChanged = s.language !== State.settings.language;
    State.settings = { ...State.settings, ...s };
    setLang(State.settings.language);
    if (languageChanged) {
      location.reload();
      return;
    }
    island.applySettings();
    State.loadIntegrationTasks();
    void refreshConfigured();
  });

  registerIntegrationHandlers(island);

  island.launch();

  async function announce() {
    const version = boot?.version ?? "";
    if (!version) return;
    const seen = State.settings.lastSeenVersion;
    if (seen !== version) void Bridge.markVersionSeen(version);
    // A fresh install was never updated, so it has nothing to be told about.
    const changed = seen !== "" && seen !== version ? newsKey(version) : null;
    const update = await Bridge.checkUpdate();
    if (!changed && !update) return;
    const body = [
      changed ? t(changed) : null,
      update ? t("news.updateLine", update.version, update.url) : null,
    ]
      .filter((line): line is string => line !== null)
      .join("\n");
    State.newsMessage = {
      title: changed ? t("news.title", version) : t("news.updateTitle"),
      body,
    };
    State.notify();
    // While the greeting is up, island.ts hands the card over when it folds.
    // Otherwise open it now, but never over something the user just opened.
    if (island.fsm.state === "greeting") return;
    if (State.mode === "expanded" && State.view !== "whatsnew") return;
    island.alert("whatsnew");
  }
  void announce();

  // In a plain browser there is no wake strip behind the cursor: make the whole
  // page wake the island so the visuals can be checked with `npm run dev`.
  if (!IS_TAURI) {
    document.addEventListener("click", () => Sound.resume(), { once: true });
  }
}

void main();

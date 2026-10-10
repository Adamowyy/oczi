// Entry point: boot the bridge, wire the island, start the greeting.

import "./style.css";
import { Bridge, IS_TAURI, onEvent, type ReminderEvent } from "./core/bridge";
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
  // The home view is built a line above, before the boot reply lands. If the
  // stored choice differs from the one the page was built in, loading it again is
  // the honest fix. The second half of the test is what keeps a page whose storage
  // is blocked from reloading itself forever.
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

  // The summon chord, registered by Rust (Ctrl+Alt+M unless the user recorded
  // another one). Same as clicking the island: show it.
  await onEvent<null>("hotkey", () => {
    setPaused(false);
    island.revealOrOpen();
  });

  // The screenshot chord, registered by Rust (Ctrl+Alt+Shift+S by default). Same
  // as clicking the eye.
  await onEvent<null>("hotkey-snip", () => {
    setPaused(false);
    island.snipStart();
  });

  // The dictation chord, registered by Rust (Ctrl+Shift+Space unless the user recorded
  // another one). One press starts listening, the next one stops it early; otherwise the
  // recording ends itself on a pause and the text goes on its way.
  await onEvent<null>("hotkey-voice", () => {
    setPaused(false);
    island.dictateStart();
  });

  // A click anywhere else in Windows. The island cannot hear it — it is click-through
  // whenever the mouse is away — so Rust watches the mouse button and tells us.
  await onEvent<null>("click-outside", () => island.clickOutside());

  // The selection overlay answered — with a region, or with nothing (Esc).
  await onEvent<{ width: number; height: number; bytes: number }>("snip-done", (info) =>
    island.snipDone(info),
  );

  // A reminder has come due. Rust kept the time; this side only shows the card,
  // pinned so it waits to be answered instead of counting down while it is read.
  await onEvent<ReminderEvent>("reminder", (r) => {
    // One card at a time: a second reminder that lands while the first is up waits
    // its turn rather than taking the card away.
    if (State.reminder) {
      State.queuedReminders.push(r);
      void Bridge.log(`reminder #${r.id} queued (${State.queuedReminders.length})`);
      return;
    }
    State.reminder = r;
    State.reminderShownAt = performance.now();
    State.isPinned = true;
    island.alert("reminder");
    void Bridge.log(`reminder #${r.id} shown view=${State.view} mode=${State.mode}`);
  });

  // The settings window writes preferences; apply them here without a restart.
  await onEvent<Settings>("settings-changed", (s) => {
    const languageChanged = s.language !== State.settings.language;
    State.settings = { ...State.settings, ...s };
    setLang(State.settings.language);
    if (languageChanged) {
      // Every view bakes its text when it is built, so the honest way to
      // re-translate the island is to load it again — the settings window does
      // exactly that for its own widgets.
      location.reload();
      return;
    }
    island.applySettings();
    State.loadIntegrationTasks();
    void refreshConfigured();
  });

  registerIntegrationHandlers(island);

  island.launch();

  /** Once per launch, and never while it sits in the background: is there
   *  anything to say about this version? The card takes the stage when the
   *  greeting folds, and the release check rides along in the same pass. */
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
    // Otherwise open it now — but never over something the user just opened.
    if (island.fsm.state === "greeting") return;
    if (State.mode === "expanded" && State.view !== "whatsnew") return;
    island.alert("whatsnew");
  }
  void announce();

  // The listener above is the only thing that can show a reminder, so nothing may
  // be fired before it is wired — this is the "I am listening" Rust waits for, and
  // the moment anything missed while Oczi was off is handed over. It comes last on
  // purpose: the greeting starts a few lines up, and a card shown before that is a
  // card the greeting paints over.
  await Bridge.remindersReady();

  // In a plain browser there is no wake strip behind the cursor: make the whole
  // page wake the island so the visuals can be checked with `npm run dev`.
  if (!IS_TAURI) {
    document.addEventListener("click", () => Sound.resume(), { once: true });
  }
}

void main();

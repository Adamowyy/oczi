// Chat view, DOM port of PromptView / ChatBubble / TypingDotsView from
// IslandViewContent.swift.

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { Bridge, type ChatContext } from "../core/bridge";
import { Sound } from "../core/sound";
import { State, type ChatMessage } from "../core/state";
import { t } from "../core/i18n";
import type { ViewActions, ViewHost } from "./views";

let nextId = 1;

/** A conversation left alone for an hour has expired and is dropped on both sides. */
const SESSION_TTL_MS = 60 * 60 * 1000;

export function bubble(message: ChatMessage): HTMLElement {
  if (message.role === "user") {
    return h(
      "div",
      { class: "chat-row user" },
      h("div", { class: "bubble", text: message.content }),
    );
  }
  return h("div", { class: "chat-row" }, h("div", { class: "reply", text: message.content }));
}

export function typingDots(): HTMLElement {
  return h(
    "div",
    { class: "chat-row" },
    h("div", { class: "typing" }, h("i"), h("i"), h("i")),
  );
}

/** The coloured chip showing what the question is about (a dropped file). */
function contextChip(label: string): HTMLElement {
  const chip = h("div", { class: "chip" }, h("i", { class: "chip-dot" }), h("span", { text: label }));
  requestAnimationFrame(() => chip.classList.add("settled"));
  return chip;
}

export function buildPrompt(actions: ViewActions, onHeightChange: () => void): ViewHost {
  const chipRow = h("div", { class: "chip-row" });
  const log = h("div", { class: "chat-log" });
  const input = h("input", {
    type: "text",
    class: "chat-input",
    placeholder: t("chat.placeholder"),
    spellcheck: "false",
  }) as HTMLInputElement;
  const send = h("button", { class: "send-btn", title: t("chat.send") }, svg(ICONS.arrowUp, 11));
  const snip = h(
    "button",
    { class: "snip-btn", title: t("chat.snipTip") },
    svg(ICONS.eye, 13),
  );
  snip.addEventListener("click", () => actions.snip());
  // A fresh conversation: the old one goes away on both sides of the IPC, so the model
  // stops seeing it either.
  const fresh = h(
    "button",
    { class: "snip-btn", title: t("chat.newTip") },
    svg(ICONS.plus, 12),
  );
  const bar = h("div", { class: "chat-bar" }, fresh, snip, input, send);

  const el = h(
    "div",
    { class: "view" },
    h("div", { class: "card wash chat-card" }, h("div", { class: "chat-body" }, chipRow, log, bar)),
  );
  (el.querySelector(".card") as HTMLElement).style.setProperty("--wash", "rgba(99,102,241,0.5)");

  let sending = false;
  let renderedCount = -1;

  async function submit() {
    const query = input.value.trim();
    if (!query || sending) return;
    if (State.chatHistory.length > 0 && Date.now() - State.chatLastActivity > SESSION_TTL_MS) {
      State.chatHistory = [];
      State.snip = null;
      State.droppedFile = null;
      await Bridge.chatReset();
    }
    State.chatLastActivity = Date.now();
    input.value = "";
    sending = true;
    Sound.play("send");
    // The mouse is nowhere near the island, it was just used to type, so the usual
    // countdown would fold it away mid-answer. Hold it open for the whole round trip.
    actions.holdOpen(0);

    State.chatHistory.push({ id: nextId++, role: "user", content: query });
    State.stateOverride = "thinking";
    State.notify();
    onHeightChange();

    const file = State.droppedFile;
    const context: ChatContext | null = State.snip
      ? { kind: "image", name: "screenshot.png" }
      : State.chatHistory.length === 1 && file
        ? { kind: "file", name: file.name, path: file.path }
        : null;

    try {
      const reply = await Bridge.chatSend(query, context);
      State.chatHistory.push({ id: nextId++, role: "assistant", content: reply.text });
      // The picture is in the conversation now, so the chip has done its job.
      State.snip = null;
      State.stateOverride = null;
      Sound.play("finish");
    } catch (err) {
      State.stateOverride = null;
      State.noteMessage = String(err).replace(/^Error:\s*/, "");
      State.view = "note";
      Sound.play("error");
    } finally {
      sending = false;
      // Re-enable before focusing: the browser blurs a disabled input, and focus()
      // on a still-disabled field silently does nothing.
      input.disabled = false;
      // Hold open until the user closes it, so a long answer can be read through.
      actions.holdOpen(0);
      State.notify();
      onHeightChange();
      input.focus();
    }
  }

  send.addEventListener("click", () => void submit());
  fresh.addEventListener("click", () => {
    if (sending) return; // never pull the rug out from under a turn in flight
    State.chatHistory = [];
    State.snip = null;
    State.droppedFile = null;
    State.chatLastActivity = Date.now();
    void Bridge.chatReset();
    Sound.play("blip");
    renderedCount = -1;
    State.notify();
    onHeightChange();
    input.focus();
  });
  input.addEventListener("keydown", (e) => {
    if ((e as KeyboardEvent).key === "Enter") {
      e.preventDefault();
      void submit();
    }
    e.stopPropagation(); // Escape closes the island, not the chat
  });

  return {
    el,
    sync() {
      const label = State.snip
        ? t("chat.snipLabel", State.snip.width, State.snip.height)
        : State.droppedFile?.name ?? "";
      if (chipRow.dataset.label !== label) {
        chipRow.dataset.label = label;
        clear(chipRow);
        if (label) chipRow.append(contextChip(label));
      }

      const thinking = State.stateOverride === "thinking";
      const count = State.chatHistory.length + (thinking ? 0.5 : 0);
      if (count !== renderedCount) {
        renderedCount = count;
        clear(log);
        for (const m of State.chatHistory) log.append(bubble(m));
        if (thinking) log.append(typingDots());
        log.scrollTop = log.scrollHeight;
      }

      input.placeholder = State.snip
        ? t("chat.placeholderSnip")
        : State.chatHistory.length === 0
          ? t("chat.placeholder")
          : "Kontynuuj…";
      input.disabled = sending;
    },
    focus() {
      input.focus();
      input.select();
    },
  };
}
